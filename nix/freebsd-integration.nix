{ pkgs, src }:

let
  constants = import ./constants.nix;

  # Per-VM integration test script generator.
  #
  # Configurable via environment:
  #   INTEGRATION_TARGET  Target to run (default: live_integration)
  #                       Compile-only: all, unit, memcheck, asan, ubsan, bench, ...
  #                       Live (require kmod): live_integration, live_all, live_smoke, ...
  #                       Setup: pkg_setup
  #   FREEBSD_HOST        SSH host override
  #   FREEBSD_DIR         Remote project dir override
  #
  # Steps:
  # 1. Ensure rsync, Rust, protobuf on FreeBSD VM
  # 2. Rsync full project source
  # 3. Run pkg_setup (idempotent env setup)
  # 4. Build workspace on VM
  # 5. Build kmod
  # 6. Load kmod (live_* targets only)
  # 7. Run kmod-integration <target>
  # 8. Unload kmod (live_* targets only)
  mkIntegrationScript = name: vm: ''
    VM_HOST="''${FREEBSD_HOST:-${vm.host}}"
    VM_DIR="''${FREEBSD_DIR:-/root/bsd-xtcp}"
    VM_TARGET="''${INTEGRATION_TARGET:-live_integration}"
    VM_CATEGORY="''${1:-all}"

    echo ""
    echo "============================================="
    echo "  ${name}: ${vm.label} ($VM_HOST)"
    echo "============================================="
    echo "  project dir: $VM_DIR"
    echo "  target:      $VM_TARGET"
    echo "  category:    $VM_CATEGORY"

    # Ensure rsync is available
    ssh "$VM_HOST" 'command -v rsync >/dev/null 2>&1 || env ASSUME_ALWAYS_YES=yes pkg install -y rsync'

    # Ensure Rust toolchain is available
    echo "--- ${name}: ensuring Rust toolchain ---"
    ssh "$VM_HOST" 'command -v cargo >/dev/null 2>&1 || env ASSUME_ALWAYS_YES=yes pkg install -y rust'

    # Ensure protobuf compiler is available
    echo "--- ${name}: ensuring protobuf ---"
    ssh "$VM_HOST" 'command -v protoc >/dev/null 2>&1 || env ASSUME_ALWAYS_YES=yes pkg install -y protobuf'

    # Rsync full project source
    echo "--- ${name}: syncing source ---"
    ssh "$VM_HOST" mkdir -p "$VM_DIR"
    rsync -av --delete \
      --exclude target/ \
      --exclude .git/ \
      "${src}/" \
      "$VM_HOST:$VM_DIR/"

    # Build full workspace on VM (touch src + utils + tests to ensure cargo detects changes)
    echo "--- ${name}: building workspace ---"
    ssh "$VM_HOST" "cd $VM_DIR && find src utils tests -name '*.rs' -exec touch {} + && cargo build --release --workspace"

    # Idempotent FreeBSD environment setup (replaces freebsd-pkg-setup.sh)
    echo "--- ${name}: running pkg_setup ---"
    ssh "$VM_HOST" "$VM_DIR/target/release/kmod-integration pkg_setup"

    # Build kmod (with stats+dtrace for targets that need observability)
    # Set TCPSTATS_DEBUG=1 to add verbose filter debug logging to dmesg
    KMOD_DEBUG_FLAG=""
    if [ "''${TCPSTATS_DEBUG:-}" = "1" ]; then
      KMOD_DEBUG_FLAG=" -DTCPSTATS_DEBUG"
      echo "--- ${name}: TCPSTATS_DEBUG enabled ---"
    fi
    echo "--- ${name}: building KLD ---"
    case "$VM_TARGET" in
      live_all|live_stats|live_dtrace)
        ssh "$VM_HOST" "cd $VM_DIR/kmod/tcp_stats_kld && make clean all EXTRA_CFLAGS='-DTCPSTATS_STATS -DTCPSTATS_DTRACE$KMOD_DEBUG_FLAG'"
        ;;
      *)
        ssh "$VM_HOST" "cd $VM_DIR/kmod/tcp_stats_kld && make clean all EXTRA_CFLAGS='$KMOD_DEBUG_FLAG'"
        ;;
    esac

    # Load kmod only for live_* targets
    if [[ "$VM_TARGET" == live_* ]]; then
      echo "--- ${name}: loading KLD ---"
      ssh "$VM_HOST" "kldstat -q -n tcp_stats_kld && kldunload tcp_stats_kld; kldload $VM_DIR/kmod/tcp_stats_kld/tcp_stats_kld.ko"

      # Raise fd limit for concurrent readers
      ssh "$VM_HOST" "sysctl dev.tcpstats.max_open_fds=64" || true
    fi

    # Run kmod-integration with the selected target
    # Forward TCPSTATS_DEBUG so the Rust harness can pass it to kmod_build
    REMOTE_ENV=""
    if [ "''${TCPSTATS_DEBUG:-}" = "1" ]; then
      REMOTE_ENV="TCPSTATS_DEBUG=1 "
    fi
    echo "--- ${name}: running $VM_TARGET (category=$VM_CATEGORY) ---"
    TEST_RC=0
    if [ "$VM_TARGET" = "live_integration" ]; then
      ssh "$VM_HOST" "''${REMOTE_ENV}$VM_DIR/target/release/kmod-integration $VM_TARGET --category $VM_CATEGORY --tcp-echo $VM_DIR/target/release/tcp-echo --kmod-src $VM_DIR/kmod/tcp_stats_kld --exporter $VM_DIR/target/release/tcp-stats-kld-exporter" || TEST_RC=$?
    else
      ssh "$VM_HOST" "''${REMOTE_ENV}$VM_DIR/target/release/kmod-integration $VM_TARGET --tcp-echo $VM_DIR/target/release/tcp-echo --kmod-src $VM_DIR/kmod/tcp_stats_kld --exporter $VM_DIR/target/release/tcp-stats-kld-exporter" || TEST_RC=$?
    fi

    # Dump kmod debug log if TCPSTATS_DEBUG was enabled
    if [[ "$VM_TARGET" == live_* ]] && [ "''${TCPSTATS_DEBUG:-}" = "1" ]; then
      echo "--- ${name}: dmesg (tcp_stats_kld debug) ---"
      ssh "$VM_HOST" "dmesg | grep tcp_stats_kld | tail -200" || true
    fi

    # Rsync structured test output back to the Linux host
    echo "--- ${name}: fetching test output ---"
    REMOTE_OUTPUT=$(ssh "$VM_HOST" "ls -1dt /tmp/kmod-integration/20* 2>/dev/null | head -1" || true)
    if [ -n "$REMOTE_OUTPUT" ]; then
      LOCAL_OUTPUT="./test-output/${name}/$(basename "$REMOTE_OUTPUT")"
      mkdir -p "$LOCAL_OUTPUT"
      rsync -av "$VM_HOST:$REMOTE_OUTPUT/" "$LOCAL_OUTPUT/" || true
      echo "  test output: $LOCAL_OUTPUT"
    else
      echo "  no test output directory found on remote"
    fi

    # Propagate test exit code
    if [ "$TEST_RC" -ne 0 ]; then
      exit "$TEST_RC"
    fi

    # Unload kmod only for live_* targets
    if [[ "$VM_TARGET" == live_* ]]; then
      echo "--- ${name}: unloading KLD ---"
      ssh "$VM_HOST" "kldunload tcp_stats_kld" || true
    fi

    echo "============================================="
    echo "  ${name}: ${vm.label} PASSED"
    echo "============================================="
  '';

  # Per-VM packages
  perVmPackages = builtins.mapAttrs (name: vm:
    pkgs.writeShellApplication {
      name = "integration-test-${name}";
      runtimeInputs = [ pkgs.rsync pkgs.openssh ];
      excludeShellChecks = [ "SC2029" "SC2053" ];
      text = mkIntegrationScript name vm;
    }
  ) constants.freebsdVMs;

  # Combined package that iterates all VMs sequentially
  integration-test-freebsd = pkgs.writeShellApplication {
    name = "integration-test-freebsd";
    runtimeInputs = [ pkgs.rsync pkgs.openssh ];
    excludeShellChecks = [ "SC2029" "SC2053" ];
    text = let
      vmScripts = pkgs.lib.mapAttrsToList
        (name: vm: mkIntegrationScript name vm)
        constants.freebsdVMs;
    in ''
      PASS_COUNT=0
      FAIL_COUNT=0
      VM_CATEGORY="''${1:-all}"
      VM_TARGET="''${INTEGRATION_TARGET:-live_integration}"

      echo "========================================="
      echo "  integration-test-freebsd: testing all VMs"
      echo "  target:   $VM_TARGET"
      echo "  category: $VM_CATEGORY"
      echo "========================================="

    '' + builtins.concatStringsSep "\n" (map (script: ''
      if ( ${script} ); then
        PASS_COUNT=$((PASS_COUNT + 1))
      else
        FAIL_COUNT=$((FAIL_COUNT + 1))
      fi
    '') vmScripts) + ''

      echo ""
      echo "========================================="
      echo "  integration-test-freebsd: $PASS_COUNT passed, $FAIL_COUNT failed"
      echo "========================================="

      if [ "$FAIL_COUNT" -gt 0 ]; then
        exit 1
      fi
    '';
  };

  # Rename per-VM packages to integration-test-<vmname>
  perVmExports = pkgs.lib.mapAttrs' (name: pkg:
    { name = "integration-test-${name}"; value = pkg; }
  ) perVmPackages;

in
{
  inherit integration-test-freebsd;
} // perVmExports
