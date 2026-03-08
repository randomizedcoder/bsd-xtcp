{ pkgs, src }:

let
  kmodSrc = "${src}/kmod/tcp_stats_kld";
  parserSrcs = "${kmodSrc}/tcp_stats_filter_parse.c";
  parserHdr = "-I${kmodSrc}";
  testSrc = "${kmodSrc}/test/test_filter_parse.c";
  benchSrc = "${kmodSrc}/test/bench_filter_parse.c";

  # All C source files for source-level scanners (no compilation needed).
  allCSrc = "${kmodSrc}";

  # Shared GCC warning flags — maximum static analysis via compiler warnings.
  gccWarningFlags = builtins.concatStringsSep " " [
    "-Wall" "-Wextra" "-Wpedantic"
    "-Wshadow" "-Wconversion" "-Wsign-conversion"
    "-Wformat=2" "-Wformat-overflow=2" "-Wformat-truncation=2"
    "-Wnull-dereference" "-Wdouble-promotion" "-Wundef"
    "-Wstrict-prototypes" "-Wold-style-definition" "-Wmissing-prototypes"
    "-Wmissing-declarations" "-Wredundant-decls" "-Wnested-externs"
    "-Wjump-misses-init" "-Wlogical-op" "-Wduplicated-cond"
    "-Wduplicated-branches" "-Wrestrict" "-Wwrite-strings" "-Wcast-qual"
    "-Wcast-align=strict" "-Wpointer-arith" "-Wbad-function-cast"
    "-Wimplicit-fallthrough=5" "-Wswitch-enum" "-Wswitch-default"
    "-Wstringop-overflow=4" "-Wstringop-truncation" "-Walloca" "-Wvla"
    "-Wstack-protector" "-fstack-protector-strong"
    "-Werror"
  ];

  # --- Tier A: Dual-compile sources (filter parser) ---

  kmod-analysis-gcc-warnings = pkgs.writeShellApplication {
    name = "kmod-analysis-gcc-warnings";
    runtimeInputs = [ pkgs.gcc ];
    text = ''
      echo "=== kmod-analysis-gcc-warnings: GCC max warnings + -Werror ==="
      WORK=$(mktemp -d)
      trap 'rm -rf "$WORK"' EXIT

      echo "--- Compiling filter parser test ---"
      gcc ${gccWarningFlags} -o "$WORK/test" \
          ${testSrc} ${parserSrcs} ${parserHdr}
      echo "  filter parser test: OK"

      echo "--- Compiling benchmark ---"
      gcc ${gccWarningFlags} -o "$WORK/bench" \
          ${benchSrc} ${parserSrcs} ${parserHdr}
      echo "  benchmark: OK"

      echo "=== kmod-analysis-gcc-warnings: PASSED ==="
    '';
  };

  kmod-analysis-gcc-fanalyzer = pkgs.writeShellApplication {
    name = "kmod-analysis-gcc-fanalyzer";
    runtimeInputs = [ pkgs.gcc ];
    text = ''
      echo "=== kmod-analysis-gcc-fanalyzer: GCC interprocedural analysis ==="
      WORK=$(mktemp -d)
      trap 'rm -rf "$WORK"' EXIT

      echo "--- Analyzing filter parser (test + parser source) ---"
      gcc -fanalyzer -Wall -Wextra -c -o "$WORK/test.o" \
          ${testSrc} ${parserHdr} 2>&1 | tee "$WORK/fanalyzer-test.log"
      gcc -fanalyzer -Wall -Wextra -c -o "$WORK/parser.o" \
          ${parserSrcs} ${parserHdr} 2>&1 | tee "$WORK/fanalyzer-parser.log"

      ISSUES=0
      if grep -c 'warning:' "$WORK"/fanalyzer-*.log 2>/dev/null; then
        ISSUES=$(grep -c 'warning:' "$WORK"/fanalyzer-*.log || true)
      fi
      echo "=== kmod-analysis-gcc-fanalyzer: completed ($ISSUES warnings) ==="
    '';
  };

  kmod-analysis-scan-build = pkgs.writeShellApplication {
    name = "kmod-analysis-scan-build";
    runtimeInputs = [ pkgs.clang-analyzer pkgs.clang ];
    text = ''
      echo "=== kmod-analysis-scan-build: Clang Static Analyzer ==="
      WORK=$(mktemp -d)
      trap 'rm -rf "$WORK"' EXIT

      CHECKERS=(
        -enable-checker core
        -enable-checker deadcode
        -enable-checker security
        -enable-checker unix
        -enable-checker alpha.security.ReturnPtrRange
        -enable-checker alpha.core.BoolAssignment
        -enable-checker alpha.core.CastSize
      )

      echo "--- Analyzing filter parser source ---"
      scan-build "''${CHECKERS[@]}" -o "$WORK/report" \
          gcc -Wall -Wextra -c -o "$WORK/parser.o" \
          ${parserSrcs} ${parserHdr}

      echo "--- Analyzing filter parser test ---"
      scan-build "''${CHECKERS[@]}" -o "$WORK/report" \
          gcc -Wall -Wextra -c -o "$WORK/test.o" \
          ${testSrc} ${parserHdr}

      echo "--- Analyzing benchmark ---"
      scan-build "''${CHECKERS[@]}" -o "$WORK/report" \
          gcc -Wall -Wextra -c -o "$WORK/bench.o" \
          ${benchSrc} ${parserHdr}

      # Report findings
      if [ -d "$WORK/report" ] && [ "$(ls -A "$WORK/report" 2>/dev/null)" ]; then
        echo "scan-build reports generated in $WORK/report"
        find "$WORK/report" -name "*.html" | head -20
      else
        echo "No issues found by scan-build."
      fi

      echo "=== kmod-analysis-scan-build: DONE ==="
    '';
  };

  kmod-analysis-clang-tidy = pkgs.writeShellApplication {
    name = "kmod-analysis-clang-tidy";
    runtimeInputs = [ pkgs.clang-tools ];
    text = ''
      echo "=== kmod-analysis-clang-tidy: clang-tidy analysis ==="

      TIDY_CONFIG="${kmodSrc}/.clang-tidy"
      if [ ! -f "$TIDY_CONFIG" ]; then
        echo "WARNING: .clang-tidy config not found at $TIDY_CONFIG"
      fi

      ISSUES=0

      # Tier A: compilable sources (filter parser)
      echo "--- Tier A: filter parser ---"
      clang-tidy ${parserSrcs} \
          --config-file="$TIDY_CONFIG" \
          -- -I${kmodSrc} 2>&1 || ISSUES=$((ISSUES + 1))

      # Tier B: test files
      echo "--- Tier B: test files ---"
      for f in ${kmodSrc}/test/test_filter_parse.c \
               ${kmodSrc}/test/bench_filter_parse.c \
               ${kmodSrc}/test/fuzz_filter_parse.c; do
        if [ -f "$f" ]; then
          echo "  checking $f"
          clang-tidy "$f" \
              --config-file="$TIDY_CONFIG" \
              -- -I${kmodSrc} 2>&1 || ISSUES=$((ISSUES + 1))
        fi
      done

      # Tier C: kernel-only sources (header analysis only, skip compilation)
      echo "--- Tier C: kernel-only sources (header scan) ---"
      for f in ${kmodSrc}/tcp_stats_kld.c ${kmodSrc}/tcp_stats_kld.h; do
        if [ -f "$f" ]; then
          echo "  checking $f (source scan, may have include errors)"
          clang-tidy "$f" \
              --config-file="$TIDY_CONFIG" \
              -- -I${kmodSrc} 2>&1 || true
        fi
      done

      echo "=== kmod-analysis-clang-tidy: DONE (issues in $ISSUES file groups) ==="
    '';
  };

  kmod-analysis-cppcheck = pkgs.writeShellApplication {
    name = "kmod-analysis-cppcheck";
    runtimeInputs = [ pkgs.cppcheck ];
    text = ''
      echo "=== kmod-analysis-cppcheck: Cppcheck analysis ==="

      SUPPRESS_FILE="${kmodSrc}/.cppcheck-suppress"
      SUPPRESS_ARG=""
      if [ -f "$SUPPRESS_FILE" ]; then
        SUPPRESS_ARG="--suppressions-list=$SUPPRESS_FILE"
      fi

      cppcheck --enable=all --force \
          --inline-suppr \
          $SUPPRESS_ARG \
          -I${kmodSrc} \
          --language=c \
          --std=c11 \
          --error-exitcode=0 \
          ${allCSrc}/*.c \
          ${allCSrc}/test/*.c

      echo "=== kmod-analysis-cppcheck: DONE ==="
    '';
  };

  kmod-analysis-infer = pkgs.lib.optionalAttrs (builtins.hasAttr "infer" pkgs) {
    kmod-analysis-infer = pkgs.writeShellApplication {
      name = "kmod-analysis-infer";
      runtimeInputs = [ pkgs.infer pkgs.gcc ];
      text = ''
        echo "=== kmod-analysis-infer: Meta Infer interprocedural analysis ==="
        WORK=$(mktemp -d)
        trap 'rm -rf "$WORK"' EXIT

        cd "$WORK"
        infer run -- gcc -Wall -Wextra -c -o test.o \
            ${testSrc} ${parserSrcs} ${parserHdr}

        echo "=== kmod-analysis-infer: DONE ==="
      '';
    };
  };

  kmod-analysis-semgrep = pkgs.writeShellApplication {
    name = "kmod-analysis-semgrep";
    runtimeInputs = [ pkgs.semgrep ];
    text = ''
      echo "=== kmod-analysis-semgrep: Semgrep security scanning ==="

      CUSTOM_RULES="${kmodSrc}/.semgrep.yml"

      # Run custom kernel rules
      if [ -f "$CUSTOM_RULES" ]; then
        echo "--- Custom kernel rules ---"
        semgrep --config "$CUSTOM_RULES" ${allCSrc}/ --no-git-ignore || true
      fi

      # Run security audit rules
      echo "--- Security audit rules ---"
      semgrep --config "p/security-audit" ${allCSrc}/ --no-git-ignore || true

      echo "=== kmod-analysis-semgrep: DONE ==="
    '';
  };

  kmod-analysis-format-check = pkgs.writeShellApplication {
    name = "kmod-analysis-format-check";
    runtimeInputs = [ pkgs.clang-tools ];
    text = ''
      echo "=== kmod-analysis-format-check: clang-format check ==="

      clang-format --dry-run -Werror \
          ${allCSrc}/*.c ${allCSrc}/*.h \
          ${allCSrc}/test/*.c

      echo "=== kmod-analysis-format-check: PASSED ==="
    '';
  };

  kmod-analysis-flawfinder = pkgs.writeShellApplication {
    name = "kmod-analysis-flawfinder";
    runtimeInputs = [ pkgs.flawfinder ];
    text = ''
      echo "=== kmod-analysis-flawfinder: CWE-oriented source scan ==="

      flawfinder --columns --context --minlevel=1 \
          ${allCSrc}/*.c ${allCSrc}/*.h \
          ${allCSrc}/test/*.c

      echo "=== kmod-analysis-flawfinder: DONE ==="
    '';
  };

  kmod-analysis-all = pkgs.writeShellApplication {
    name = "kmod-analysis-all";
    runtimeInputs = [
      kmod-analysis-gcc-warnings
      kmod-analysis-gcc-fanalyzer
      kmod-analysis-scan-build
      kmod-analysis-clang-tidy
      kmod-analysis-cppcheck
      kmod-analysis-semgrep
      kmod-analysis-flawfinder
      kmod-analysis-format-check
    ];
    text = ''
      echo "============================================="
      echo "  kmod-analysis-all: running all C analyzers"
      echo "============================================="

      PASS=0
      FAIL=0
      SKIP=0

      run_tool() {
        local name="$1"
        echo ""
        echo ">>> Running $name..."
        if "$name"; then
          PASS=$((PASS + 1))
          echo "<<< $name: PASS"
        else
          FAIL=$((FAIL + 1))
          echo "<<< $name: FAIL (exit $?)"
        fi
      }

      run_tool kmod-analysis-gcc-warnings
      run_tool kmod-analysis-gcc-fanalyzer
      run_tool kmod-analysis-scan-build
      run_tool kmod-analysis-clang-tidy
      run_tool kmod-analysis-cppcheck
      run_tool kmod-analysis-semgrep
      run_tool kmod-analysis-flawfinder
      run_tool kmod-analysis-format-check

      # Infer (conditional — may not be available)
      if command -v infer &>/dev/null; then
        run_tool kmod-analysis-infer
      else
        SKIP=$((SKIP + 1))
        echo ">>> kmod-analysis-infer: SKIPPED (infer not available)"
      fi

      echo ""
      echo "============================================="
      echo "  kmod-analysis-all: SUMMARY"
      echo "    PASS: $PASS  FAIL: $FAIL  SKIP: $SKIP"
      echo "============================================="

      if [ "$FAIL" -gt 0 ]; then
        exit 1
      fi
    '';
  };

in
{
  inherit
    kmod-analysis-gcc-warnings
    kmod-analysis-gcc-fanalyzer
    kmod-analysis-scan-build
    kmod-analysis-clang-tidy
    kmod-analysis-cppcheck
    kmod-analysis-semgrep
    kmod-analysis-flawfinder
    kmod-analysis-format-check
    kmod-analysis-all;
} // kmod-analysis-infer
