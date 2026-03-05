{ pkgs, src }:

let
  kmodSrc = "${src}/kmod/tcp_stats_kld";
  parserSrcs = "${kmodSrc}/tcp_stats_filter_parse.c";
  parserHdr = "-I${kmodSrc}";
  testSrc = "${kmodSrc}/test/test_filter_parse.c";
  benchSrc = "${kmodSrc}/test/bench_filter_parse.c";

  # Enhanced GCC warning flags (without -Werror to avoid breaking tests during triage).
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
  ];

  kmod-test-unit = pkgs.writeShellApplication {
    name = "kmod-test-unit";
    runtimeInputs = [ pkgs.gcc ];
    text = ''
      echo "=== kmod-test-unit: compile and run filter parser unit tests ==="
      WORK=$(mktemp -d)
      trap 'rm -rf "$WORK"' EXIT
      gcc ${gccWarningFlags} -o "$WORK/test" \
          ${testSrc} ${parserSrcs} ${parserHdr}
      "$WORK/test"
      echo "=== kmod-test-unit: PASSED ==="
    '';
  };

  kmod-test-memcheck = pkgs.writeShellApplication {
    name = "kmod-test-memcheck";
    runtimeInputs = [ pkgs.gcc pkgs.valgrind ];
    text = ''
      echo "=== kmod-test-memcheck: valgrind memcheck ==="
      WORK=$(mktemp -d)
      trap 'rm -rf "$WORK"' EXIT
      gcc -g -O0 ${gccWarningFlags} -o "$WORK/test" \
          ${testSrc} ${parserSrcs} ${parserHdr}
      valgrind --tool=memcheck --leak-check=full \
          --track-origins=yes --error-exitcode=1 \
          --show-error-list=yes "$WORK/test"
      echo "=== kmod-test-memcheck: PASSED ==="
    '';
  };

  kmod-test-asan = pkgs.writeShellApplication {
    name = "kmod-test-asan";
    runtimeInputs = [ pkgs.gcc ];
    text = ''
      echo "=== kmod-test-asan: AddressSanitizer + UBSan ==="
      WORK=$(mktemp -d)
      trap 'rm -rf "$WORK"' EXIT
      gcc -g -O1 -fsanitize=address,undefined \
          -fno-omit-frame-pointer -fno-sanitize-recover=all \
          ${gccWarningFlags} -o "$WORK/test" \
          ${testSrc} ${parserSrcs} ${parserHdr}
      "$WORK/test"
      echo "=== kmod-test-asan: PASSED ==="
    '';
  };

  kmod-test-ubsan = pkgs.writeShellApplication {
    name = "kmod-test-ubsan";
    runtimeInputs = [ pkgs.gcc ];
    text = ''
      echo "=== kmod-test-ubsan: UndefinedBehaviorSanitizer ==="
      WORK=$(mktemp -d)
      trap 'rm -rf "$WORK"' EXIT
      gcc -g -O1 -fsanitize=undefined \
          -fno-sanitize-recover=all \
          ${gccWarningFlags} -o "$WORK/test" \
          ${testSrc} ${parserSrcs} ${parserHdr}
      "$WORK/test"
      echo "=== kmod-test-ubsan: PASSED ==="
    '';
  };

  kmod-test-bench = pkgs.writeShellApplication {
    name = "kmod-test-bench";
    runtimeInputs = [ pkgs.gcc ];
    text = ''
      echo "=== kmod-test-bench: filter parser benchmark ==="
      WORK=$(mktemp -d)
      trap 'rm -rf "$WORK"' EXIT
      gcc -O2 ${gccWarningFlags} -o "$WORK/bench" \
          ${benchSrc} ${parserSrcs} ${parserHdr}
      "$WORK/bench" "''${1:-1000000}"
      echo "=== kmod-test-bench: DONE ==="
    '';
  };

  kmod-test-callgrind = pkgs.writeShellApplication {
    name = "kmod-test-callgrind";
    runtimeInputs = [ pkgs.gcc pkgs.valgrind ];
    text = ''
      echo "=== kmod-test-callgrind: callgrind CPU profiling ==="
      WORK=$(mktemp -d)
      trap 'rm -rf "$WORK"' EXIT
      gcc -O2 -g ${gccWarningFlags} -o "$WORK/bench" \
          ${benchSrc} ${parserSrcs} ${parserHdr}
      valgrind --tool=callgrind \
          --callgrind-out-file="$WORK/callgrind.out" \
          --collect-jumps=yes "$WORK/bench" 100000
      callgrind_annotate --auto=yes "$WORK/callgrind.out"
      echo "=== kmod-test-callgrind: DONE ==="
    '';
  };

  kmod-test-all = pkgs.writeShellApplication {
    name = "kmod-test-all";
    runtimeInputs = [
      kmod-test-unit
      kmod-test-memcheck
      kmod-test-asan
      kmod-test-ubsan
      kmod-test-bench
      kmod-test-callgrind
    ];
    text = ''
      echo "========================================="
      echo "  kmod-test-all: running full test suite"
      echo "========================================="
      kmod-test-unit
      kmod-test-memcheck
      kmod-test-asan
      kmod-test-ubsan
      kmod-test-bench
      kmod-test-callgrind
      echo "========================================="
      echo "  kmod-test-all: ALL PASSED"
      echo "========================================="
    '';
  };

in
{
  inherit
    kmod-test-unit
    kmod-test-memcheck
    kmod-test-asan
    kmod-test-ubsan
    kmod-test-bench
    kmod-test-callgrind
    kmod-test-all;
}
