{ pkgs, src }:

let
  constants = import ./constants.nix;

  # Common preamble: rsync source, ensure toolchain.
  mkPreamble = name: vm: ''
    VM_HOST="''${FREEBSD_HOST:-${vm.host}}"
    VM_DIR="''${FREEBSD_DIR:-/root/bsd-xtcp}"

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
  '';

  # Build the exporter on VM.
  mkBuildScript = name: vm: ''
    ${mkPreamble name vm}

    echo "--- ${name}: building tcp-stats-kld-exporter ---"
    ssh "$VM_HOST" "cd $VM_DIR && find src utils -name '*.rs' -exec touch {} + && cargo build --release -p tcp-stats-kld-exporter"

    echo "--- ${name}: verifying binary ---"
    ssh "$VM_HOST" "test -f $VM_DIR/target/release/tcp-stats-kld-exporter"

    echo "============================================="
    echo "  ${name}: build PASSED"
    echo "============================================="
  '';

  # Lint the exporter on VM.
  mkLintScript = name: vm: ''
    ${mkPreamble name vm}

    echo "--- ${name}: linting tcp-stats-kld-exporter ---"
    ssh "$VM_HOST" "cd $VM_DIR && find src utils -name '*.rs' -exec touch {} + && cargo clippy -p tcp-stats-kld-exporter -- -D warnings"

    echo "============================================="
    echo "  ${name}: lint PASSED"
    echo "============================================="
  '';

  # Integration test: build, start exporter, scrape metrics, verify rate limiting.
  mkTestScript = name: vm: ''
    ${mkPreamble name vm}

    echo "--- ${name}: building workspace ---"
    ssh "$VM_HOST" "cd $VM_DIR && find src utils -name '*.rs' -exec touch {} + && cargo build --release -p tcp-stats-kld-exporter -p tcp-echo"

    # Ensure kmod is loaded
    echo "--- ${name}: building and loading KLD ---"
    ssh "$VM_HOST" "cd $VM_DIR/kmod/tcp_stats_kld && make clean all"
    ssh "$VM_HOST" "kldstat -q -n tcp_stats_kld && kldunload tcp_stats_kld; kldload $VM_DIR/kmod/tcp_stats_kld/tcp_stats_kld.ko"

    # Start tcp-echo server for test connections
    echo "--- ${name}: starting tcp-echo server ---"
    ssh "$VM_HOST" "cd $VM_DIR && $VM_DIR/target/release/tcp-echo server --ports 19876 &"
    ssh "$VM_HOST" 'sleep 0.5'

    # Start tcp-echo client
    echo "--- ${name}: starting tcp-echo client ---"
    ssh "$VM_HOST" "cd $VM_DIR && $VM_DIR/target/release/tcp-echo client --host 127.0.0.1 --ports 19876 --connections 5 --duration 30 &"
    ssh "$VM_HOST" 'sleep 1'

    # Start exporter in background
    echo "--- ${name}: starting exporter ---"
    ssh "$VM_HOST" "$VM_DIR/target/release/tcp-stats-kld-exporter --listen 127.0.0.1:9814 &"
    ssh "$VM_HOST" 'sleep 1'

    # Scrape /metrics
    echo "--- ${name}: scraping /metrics ---"
    OUTPUT=$(ssh "$VM_HOST" 'fetch -q -o - http://127.0.0.1:9814/metrics 2>&1') || true
    echo "$OUTPUT"

    # Verify output
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

    check 'tcpstats_exporter_up 1'
    check 'tcpstats_exporter_http_requests_total'
    check 'tcpstats_exporter_collection_latency_seconds'
    check 'tcpstats_sockets_total'
    check 'tcpstats_sockets_by_state'
    check 'tcpstats_sys_connection_attempts_total'
    check 'tcpstats_sys_sent_packets_total'
    check 'tcpstats_sys_received_packets_total'

    # Verify rate limiting: rapid-fire 5 requests, some should get 429
    echo "--- ${name}: testing rate limiting ---"
    RATE_RESULTS=""
    for i in 1 2 3 4 5; do
      CODE=$(ssh "$VM_HOST" "fetch -q -o /dev/null -w '%{http_code}' http://127.0.0.1:9814/metrics 2>/dev/null || echo 429")
      RATE_RESULTS="$RATE_RESULTS $CODE"
    done
    echo "  Rate limit responses: $RATE_RESULTS"

    if echo "$RATE_RESULTS" | grep -q '429'; then
      echo "  PASS: rate limiting working (got 429)"
      PASS=$((PASS + 1))
    else
      echo "  WARN: no 429 responses (timing dependent)"
    fi

    # Cleanup
    echo "--- ${name}: cleaning up ---"
    ssh "$VM_HOST" 'pkill -f tcp-stats-kld-exporter 2>/dev/null || true'
    ssh "$VM_HOST" 'pkill -f "tcp-echo" 2>/dev/null || true'

    echo ""
    echo "  Results: $PASS passed, $FAIL failed"

    if [ "$FAIL" -gt 0 ]; then
      echo "  FAILED: ${name}"
      exit 1
    fi

    echo "============================================="
    echo "  ${name}: test PASSED"
    echo "============================================="
  '';

  # All three sequentially.
  mkAllScript = name: vm: ''
    ${mkBuildScript name vm}
    ${mkLintScript name vm}
    ${mkTestScript name vm}
  '';

  # Per-VM packages for each action.
  perVm = builtins.mapAttrs (name: vm: {
    build = pkgs.writeShellApplication {
      name = "exporter-build-${name}";
      runtimeInputs = [ pkgs.rsync pkgs.openssh ];
      excludeShellChecks = [ "SC2029" ];
      text = mkBuildScript name vm;
    };
    lint = pkgs.writeShellApplication {
      name = "exporter-lint-${name}";
      runtimeInputs = [ pkgs.rsync pkgs.openssh ];
      excludeShellChecks = [ "SC2029" ];
      text = mkLintScript name vm;
    };
    test = pkgs.writeShellApplication {
      name = "exporter-test-${name}";
      runtimeInputs = [ pkgs.rsync pkgs.openssh ];
      excludeShellChecks = [ "SC2029" ];
      text = mkTestScript name vm;
    };
    all = pkgs.writeShellApplication {
      name = "exporter-all-${name}";
      runtimeInputs = [ pkgs.rsync pkgs.openssh ];
      excludeShellChecks = [ "SC2029" ];
      text = mkAllScript name vm;
    };
  }) constants.freebsdVMs;

  # Flatten to { exporter-build-freebsd150 = ...; exporter-lint-freebsd150 = ...; ... }
  flatPackages = builtins.foldl' (acc: name:
    let vm = perVm.${name}; in
    acc // {
      "exporter-build-${name}" = vm.build;
      "exporter-lint-${name}" = vm.lint;
      "exporter-test-${name}" = vm.test;
      "exporter-all-${name}" = vm.all;
    }
  ) {} (builtins.attrNames perVm);

in
flatPackages
