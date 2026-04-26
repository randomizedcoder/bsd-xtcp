{ pkgs, src }:

let
  constants = import ./constants.nix;

  # Per-VM FreeBSD port build + test script generator.
  #
  # 1. Ensure ports tree exists on VM
  # 2. Rsync port files into /usr/ports/net/tcpstats-kmod
  # 3. Regenerate distinfo (make makesum)
  # 4. Lint with portlint
  # 5. Build, stage, verify plist
  # 6. Install, load kmod, verify /dev/tcpstats
  # 7. Package and deinstall
  # 8. Test option variants (DTRACE, STATS)
  # 9. Rsync distinfo back to host
  mkPortTestScript = name: vm: ''
    VM_HOST="''${FREEBSD_HOST:-${vm.host}}"
    PORT_SRC="''${PORT_SRC:-../freebsd-ports/net/tcpstats-kmod}"

    echo ""
    echo "============================================="
    echo "  port-test-${name}: ${vm.label} ($VM_HOST)"
    echo "============================================="
    echo "  port source: $PORT_SRC"

    PASS=0
    FAIL=0
    WARN=0

    pass() {
      echo "  PASS: $1"
      PASS=$((PASS + 1))
    }

    fail() {
      echo "  FAIL: $1"
      FAIL=$((FAIL + 1))
    }

    warn() {
      echo "  WARN: $1"
      WARN=$((WARN + 1))
    }

    # Verify port source exists on host
    if [ ! -f "$PORT_SRC/Makefile" ]; then
      fail "port source not found at $PORT_SRC/Makefile"
      echo "  Set PORT_SRC to the path containing the port Makefile"
      exit 1
    fi

    # --- Phase A: Ensure ports tree ---
    echo ""
    echo "--- ${name}: ensuring ports tree ---"
    ssh "$VM_HOST" 'test -d /usr/ports/Mk || (echo "Installing ports tree..." && env ASSUME_ALWAYS_YES=yes pkg install -y git-lite && git clone --depth 1 https://git.FreeBSD.org/ports.git /usr/ports)'
    if ssh "$VM_HOST" 'test -d /usr/ports/Mk'; then
      pass "ports tree present"
    else
      fail "ports tree missing"
      exit 1
    fi

    # Ensure kernel source is present (required for kmod builds)
    echo "--- ${name}: ensuring kernel source ---"
    ssh "$VM_HOST" 'test -d /usr/src/sys || (echo "Fetching kernel source..." && fetch -o /tmp/src.txz "https://download.freebsd.org/releases/$(uname -m)/$(uname -r | sed "s/-p[0-9]*//")/src.txz" && tar -C / -xf /tmp/src.txz && rm /tmp/src.txz)' || true
    if ssh "$VM_HOST" 'test -d /usr/src/sys'; then
      pass "kernel source present"
    else
      fail "kernel source missing"
      exit 1
    fi

    # --- Phase B: Rsync port files ---
    echo ""
    echo "--- ${name}: syncing port files ---"
    ssh "$VM_HOST" 'mkdir -p /usr/ports/net/tcpstats-kmod'
    rsync -av --delete "$PORT_SRC/" "$VM_HOST:/usr/ports/net/tcpstats-kmod/"
    pass "port files synced"

    # --- Phase C: Regenerate distinfo ---
    echo ""
    echo "--- ${name}: regenerating distinfo (make makesum) ---"
    if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes makesum'; then
      pass "make makesum"
    else
      fail "make makesum"
      exit 1
    fi

    # Rsync updated distinfo back to host
    echo "--- ${name}: syncing distinfo back ---"
    rsync -av "$VM_HOST:/usr/ports/net/tcpstats-kmod/distinfo" "$PORT_SRC/distinfo"
    pass "distinfo synced back"

    # --- Phase D: Lint ---
    echo ""
    echo "--- ${name}: running portlint ---"
    ssh "$VM_HOST" 'command -v portlint >/dev/null 2>&1 || env ASSUME_ALWAYS_YES=yes pkg install -y portlint'
    PORTLINT_OUTPUT=$(ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && portlint -AC' 2>&1) || true
    echo "$PORTLINT_OUTPUT"
    if echo "$PORTLINT_OUTPUT" | grep -q "^0 errors"; then
      pass "portlint (0 errors)"
    elif echo "$PORTLINT_OUTPUT" | grep -q "errors"; then
      ERRORS=$(echo "$PORTLINT_OUTPUT" | grep "errors" | head -1)
      warn "portlint: $ERRORS"
    else
      pass "portlint completed"
    fi

    # --- Phase E: Build, stage, verify ---
    echo ""
    echo "--- ${name}: building port (make clean stage) ---"
    if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes clean && make BATCH=yes stage'; then
      pass "make stage"
    else
      fail "make stage"
      exit 1
    fi

    echo "--- ${name}: stage-qa ---"
    if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes stage-qa'; then
      pass "make stage-qa"
    else
      warn "make stage-qa had issues"
    fi

    echo "--- ${name}: check-plist ---"
    if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes check-plist'; then
      pass "make check-plist"
    else
      fail "make check-plist"
    fi

    # --- Phase F: Install and verify ---
    echo ""
    echo "--- ${name}: installing port ---"
    if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes install'; then
      pass "make install"
    else
      fail "make install"
    fi

    echo "--- ${name}: loading and verifying kmod ---"
    ssh "$VM_HOST" 'kldstat -q -n tcpstats && kldunload tcpstats || true'
    if ssh "$VM_HOST" 'kldload tcpstats'; then
      pass "kldload tcpstats"
    else
      fail "kldload tcpstats"
    fi

    if ssh "$VM_HOST" 'kldstat | grep -q tcpstats'; then
      pass "kldstat shows tcpstats"
    else
      fail "kldstat missing tcpstats"
    fi

    if ssh "$VM_HOST" 'test -c /dev/tcpstats'; then
      pass "/dev/tcpstats exists"
    else
      warn "/dev/tcpstats not found (may require configuration)"
    fi

    # Unload before next phases
    ssh "$VM_HOST" 'kldunload tcpstats || true'

    # --- Phase G: Package and deinstall ---
    echo ""
    echo "--- ${name}: packaging ---"
    if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes package'; then
      pass "make package"
    else
      warn "make package failed"
    fi

    echo "--- ${name}: deinstalling ---"
    if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes deinstall'; then
      pass "make deinstall"
    else
      warn "make deinstall failed"
    fi

    # --- Phase H: Option variants ---
    echo ""
    echo "--- ${name}: testing DTRACE option ---"
    if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes clean && make BATCH=yes WITH="DTRACE"'; then
      pass "build with DTRACE"
    else
      fail "build with DTRACE"
    fi

    echo "--- ${name}: testing STATS option ---"
    if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes clean && make BATCH=yes WITH="STATS"'; then
      pass "build with STATS"
    else
      fail "build with STATS"
    fi

    echo "--- ${name}: testing DTRACE+STATS options ---"
    if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes clean && make BATCH=yes WITH="DTRACE STATS"'; then
      pass "build with DTRACE STATS"
    else
      fail "build with DTRACE STATS"
    fi

    # Final cleanup
    ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes clean' || true

    # --- Summary ---
    echo ""
    echo "============================================="
    echo "  port-test-${name}: ${vm.label}"
    echo "  Results: $PASS passed, $FAIL failed, $WARN warnings"
    echo "============================================="

    if [ "$FAIL" -gt 0 ]; then
      echo "  FAILED: port-test-${name}"
      exit 1
    fi

    echo "  PASSED: port-test-${name}"
  '';

  # Per-VM packages
  perVmPackages = builtins.mapAttrs (name: vm:
    pkgs.writeShellApplication {
      name = "port-test-${name}";
      runtimeInputs = [ pkgs.rsync pkgs.openssh ];
      excludeShellChecks = [ "SC2029" ];
      text = mkPortTestScript name vm;
    }
  ) constants.freebsdVMs;

  # Combined package that iterates all VMs sequentially
  portTestFreebsd = pkgs.writeShellApplication {
    name = "port-test-freebsd";
    runtimeInputs = [ pkgs.rsync pkgs.openssh ];
    excludeShellChecks = [ "SC2029" "SC2030" "SC2031" ];
    text = let
      vmScripts = pkgs.lib.mapAttrsToList
        (name: vm: mkPortTestScript name vm)
        constants.freebsdVMs;
    in ''
      PASS_COUNT=0
      FAIL_COUNT=0

      echo "========================================="
      echo "  port-test-freebsd: testing all VMs"
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
      echo "  port-test-freebsd: $PASS_COUNT passed, $FAIL_COUNT failed"
      echo "========================================="

      if [ "$FAIL_COUNT" -gt 0 ]; then
        exit 1
      fi
    '';
  };

  # Rename per-VM packages to port-test-<vmname>
  perVmExports = pkgs.lib.mapAttrs' (name: pkg:
    { name = "port-test-${name}"; value = pkg; }
  ) perVmPackages;

in
{
  "port-test-freebsd" = portTestFreebsd;
} // perVmExports
