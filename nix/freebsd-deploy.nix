{ pkgs, src }:

let
  constants = import ./constants.nix;

  # Per-VM deploy + build + test script generator.
  #
  # 1. Ensure Rust + protobuf available on FreeBSD VM
  # 2. Rsync full project source
  # 3. Build on VM
  # 4. Ensure KLD is loaded
  # 5. Run and verify output contains FreeBSD markers
  mkFreebsdDeployScript = name: vm: ''
    VM_HOST="''${FREEBSD_HOST:-${vm.host}}"
    VM_DIR="''${FREEBSD_DIR:-/root/tcpstats-reader}"

    echo ""
    echo "============================================="
    echo "  ${name}: ${vm.label} ($VM_HOST)"
    echo "============================================="
    echo "  project dir: $VM_DIR"

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

    # Build on VM (touch src to ensure cargo detects changes after rsync)
    echo "--- ${name}: building ---"
    ssh "$VM_HOST" "cd $VM_DIR && find src utils -name '*.rs' -exec touch {} + && cargo build --release"

    # Build and load KLD
    echo "--- ${name}: building KLD ---"
    ssh "$VM_HOST" "cd $VM_DIR/kmod/tcpstats && make clean all"

    echo "--- ${name}: loading KLD ---"
    ssh "$VM_HOST" "kldstat -q -n tcpstats && kldunload tcpstats; kldload $VM_DIR/kmod/tcpstats/tcpstats.ko"

    # Create a background TCP connection so the client has sockets to report
    echo "--- ${name}: creating test TCP connection ---"
    ssh "$VM_HOST" 'sh -c "nc -l 127.0.0.1 19876 >/dev/null 2>&1 &"'
    ssh "$VM_HOST" 'sh -c "nc 127.0.0.1 19876 </dev/null >/dev/null 2>&1 &"'
    ssh "$VM_HOST" 'sleep 0.5'

    # Run and capture output
    echo "--- ${name}: running tcpstats-reader ---"
    OUTPUT=$(ssh "$VM_HOST" "$VM_DIR/target/release/tcpstats-reader --count 1 --pretty" 2>&1) || true
    echo "$OUTPUT"

    # Clean up background nc processes
    ssh "$VM_HOST" 'pkill -f "nc.*19876" 2>/dev/null || true'

    # Verify output contains expected FreeBSD markers
    echo "--- ${name}: verifying output ---"
    PASS=0
    FAIL=0

    check() {
      if echo "$OUTPUT" | grep -q "$1"; then
        echo "  PASS: found $1"
        PASS=$((PASS + 1))
      else
        echo "  FAIL: missing $1"
        FAIL=$((FAIL + 1))
      fi
    }

    check '"platform"'
    check 'FREEBSD'
    check '"sndCwnd"'
    check '"rttUs"'
    check '"state"'

    echo ""
    echo "  Results: $PASS passed, $FAIL failed"

    if [ "$FAIL" -gt 0 ]; then
      echo "  FAILED: ${name}"
      exit 1
    fi

    echo "============================================="
    echo "  ${name}: ${vm.label} PASSED"
    echo "============================================="
  '';

  # Per-VM packages
  perVmPackages = builtins.mapAttrs (name: vm:
    pkgs.writeShellApplication {
      name = "tcpstats-reader-${name}";
      runtimeInputs = [ pkgs.rsync pkgs.openssh ];
      excludeShellChecks = [ "SC2029" ];
      text = mkFreebsdDeployScript name vm;
    }
  ) constants.freebsdVMs;

  # Combined package that iterates all VMs sequentially
  tcpstatsReaderFreebsd = pkgs.writeShellApplication {
    name = "tcpstats-reader-freebsd";
    runtimeInputs = [ pkgs.rsync pkgs.openssh ];
    excludeShellChecks = [ "SC2029" ];
    text = let
      vmScripts = pkgs.lib.mapAttrsToList
        (name: vm: mkFreebsdDeployScript name vm)
        constants.freebsdVMs;
    in ''
      PASS_COUNT=0
      FAIL_COUNT=0

      echo "========================================="
      echo "  tcpstats-reader-freebsd: testing all VMs"
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
      echo "  tcpstats-reader-freebsd: $PASS_COUNT passed, $FAIL_COUNT failed"
      echo "========================================="

      if [ "$FAIL_COUNT" -gt 0 ]; then
        exit 1
      fi
    '';
  };

  # Rename per-VM packages to tcpstats-reader-<vmname>
  perVmExports = pkgs.lib.mapAttrs' (name: pkg:
    { name = "tcpstats-reader-${name}"; value = pkg; }
  ) perVmPackages;

in
{
  "tcpstats-reader-freebsd" = tcpstatsReaderFreebsd;
} // perVmExports
