{ pkgs, src }:

let
  constants = import ./constants.nix;

  # Per-VM FreeBSD port build + test script generator.
  #
  # tcpstats-kmod:
  #   A. Ensure ports tree exists on VM
  #   B. Rsync port files into /usr/ports/net/tcpstats-kmod
  #   C. Regenerate distinfo (make makesum)
  #   D. Lint with portlint
  #   E. Build, stage, verify plist
  #   F. Install, load kmod, verify /dev/tcpstats
  #   G. Package and deinstall
  #   H. Test option variants (DTRACE, STATS)
  #
  # tcpstats-reader (if port source exists):
  #   I. Rsync port files into /usr/ports/net/tcpstats-reader
  #   J. Regenerate distinfo (make makesum)
  #   K. Build, stage, verify plist
  #   L. Install, verify binary short flags and man page
  #   M. Test reader with live kmod (-c 1 -p)
  #   N. Package and deinstall
  mkPortTestScript = name: vm: ''
    VM_HOST="''${FREEBSD_HOST:-${vm.host}}"
    PORT_SRC="''${PORT_SRC:-../freebsd-ports/net/tcpstats-kmod}"
    READER_PORT_SRC="''${READER_PORT_SRC:-../freebsd-ports/net/tcpstats-reader}"

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

    # Final kmod cleanup
    ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes clean' || true

    # =========================================================
    # tcpstats-reader port tests
    # =========================================================

    if [ -f "$READER_PORT_SRC/Makefile" ]; then
      echo ""
      echo "============================================="
      echo "  ${name}: tcpstats-reader port tests"
      echo "============================================="

      # --- Phase I: Sync reader port files ---
      echo ""
      echo "--- ${name}: syncing tcpstats-reader port files ---"
      ssh "$VM_HOST" 'mkdir -p /usr/ports/net/tcpstats-reader'
      rsync -av --delete "$READER_PORT_SRC/" "$VM_HOST:/usr/ports/net/tcpstats-reader/"
      pass "reader port files synced"

      # --- Phase J: Regenerate reader distinfo ---
      echo ""
      echo "--- ${name}: regenerating reader distinfo (make makesum) ---"
      if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-reader && make BATCH=yes makesum'; then
        pass "reader make makesum"
      else
        fail "reader make makesum"
      fi

      # Rsync updated distinfo back to host
      echo "--- ${name}: syncing reader distinfo back ---"
      rsync -av "$VM_HOST:/usr/ports/net/tcpstats-reader/distinfo" "$READER_PORT_SRC/distinfo"
      pass "reader distinfo synced back"

      # --- Phase K: Build, stage, verify reader ---
      echo ""
      echo "--- ${name}: building reader port (make clean stage) ---"
      if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-reader && make BATCH=yes clean && make BATCH=yes stage'; then
        pass "reader make stage"
      else
        fail "reader make stage"
      fi

      echo "--- ${name}: reader stage-qa ---"
      if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-reader && make BATCH=yes stage-qa'; then
        pass "reader make stage-qa"
      else
        warn "reader make stage-qa had issues"
      fi

      echo "--- ${name}: reader check-plist ---"
      if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-reader && make BATCH=yes check-plist'; then
        pass "reader make check-plist"
      else
        fail "reader make check-plist"
      fi

      # --- Phase L: Install reader and verify ---
      echo ""
      echo "--- ${name}: installing reader ---"
      ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-reader && make BATCH=yes deinstall' 2>/dev/null || true
      if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-reader && make BATCH=yes install'; then
        pass "reader make install"
      else
        fail "reader make install"
      fi

      echo "--- ${name}: verifying tcpstats-reader binary ---"
      if ssh "$VM_HOST" 'tcpstats-reader -h 2>&1 | grep -q "\\-c, --count"'; then
        pass "reader -h shows short flags"
      else
        fail "reader -h missing short flags"
      fi

      echo "--- ${name}: verifying tcpstats-reader man page ---"
      if ssh "$VM_HOST" 'man -w tcpstats-reader >/dev/null 2>&1'; then
        pass "reader man page installed"
      else
        fail "reader man page missing"
      fi

      # --- Phase M: Test reader with live kmod ---
      echo ""
      echo "--- ${name}: testing reader with live kmod ---"
      ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes install' || true
      ssh "$VM_HOST" 'kldstat -q -n tcpstats && kldunload tcpstats || true'
      ssh "$VM_HOST" 'kldload tcpstats' || true
      if ssh "$VM_HOST" 'tcpstats-reader -c 1 -p 2>&1 | head -1 | grep -q "{"'; then
        pass "reader -c 1 -p produces JSON"
      else
        warn "reader could not read from /dev/tcpstats (kmod may not be loaded)"
      fi

      # Cleanup
      ssh "$VM_HOST" 'kldunload tcpstats || true'
      ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-kmod && make BATCH=yes deinstall' 2>/dev/null || true

      # --- Phase N: Package and deinstall reader ---
      echo ""
      echo "--- ${name}: packaging reader ---"
      if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-reader && make BATCH=yes package'; then
        pass "reader make package"
      else
        warn "reader make package failed"
      fi

      echo "--- ${name}: deinstalling reader ---"
      if ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-reader && make BATCH=yes deinstall'; then
        pass "reader make deinstall"
      else
        warn "reader make deinstall failed"
      fi

      # Final reader cleanup
      ssh "$VM_HOST" 'cd /usr/ports/net/tcpstats-reader && make BATCH=yes clean' || true

    else
      echo ""
      echo "--- ${name}: skipping tcpstats-reader tests (no port source at $READER_PORT_SRC) ---"
    fi

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
