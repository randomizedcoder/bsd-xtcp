use std::path::Path;

use anyhow::{Context, Result};

use super::process::run_cmd;

/// C compiler configuration.
pub struct CompileConfig<'a> {
    pub cc: &'a str,
    pub kmod_src: &'a str,
}

impl<'a> CompileConfig<'a> {
    fn test_dir(&self) -> String {
        format!("{}/test", self.kmod_src)
    }

    fn parser_src(&self) -> String {
        format!("{}/tcp_stats_filter_parse.c", self.kmod_src)
    }

    fn include_flag(&self) -> String {
        format!("-I{}", self.kmod_src)
    }

    /// Compile with given flags, sources, and output name.
    fn compile(
        &self,
        output: &str,
        flags: &[&str],
        sources: &[&str],
        extra_args: &[&str],
    ) -> Result<String> {
        let out_path = format!("{}/{output}", self.test_dir());
        let include = self.include_flag();

        let mut args: Vec<&str> = Vec::new();
        args.extend_from_slice(flags);
        args.push("-o");
        args.push(&out_path);
        for src in sources {
            args.push(src);
        }
        args.push(&include);
        args.extend_from_slice(extra_args);

        run_cmd(self.cc, &args).with_context(|| format!("compile {output}"))?;

        Ok(out_path)
    }

    /// Build unit test binary.
    pub fn build_unit(&self) -> Result<String> {
        let test_src = format!("{}/test_filter_parse.c", self.test_dir());
        let parser = self.parser_src();
        self.compile(
            "test_unit",
            &["-Wall", "-Wextra"],
            &[&test_src, &parser],
            &[],
        )
    }

    /// Build memcheck binary (debug, no optimization).
    pub fn build_memcheck(&self) -> Result<String> {
        let test_src = format!("{}/test_filter_parse.c", self.test_dir());
        let parser = self.parser_src();
        self.compile(
            "test_memcheck",
            &["-g", "-O0", "-Wall", "-Wextra"],
            &[&test_src, &parser],
            &[],
        )
    }

    /// Build AddressSanitizer + UBSan binary.
    pub fn build_asan(&self) -> Result<String> {
        let test_src = format!("{}/test_filter_parse.c", self.test_dir());
        let parser = self.parser_src();
        self.compile(
            "test_asan",
            &[
                "-g",
                "-O1",
                "-fsanitize=address,undefined",
                "-fno-omit-frame-pointer",
                "-fno-sanitize-recover=all",
                "-Wall",
                "-Wextra",
            ],
            &[&test_src, &parser],
            &[],
        )
    }

    /// Build UBSan-only binary.
    pub fn build_ubsan(&self) -> Result<String> {
        let test_src = format!("{}/test_filter_parse.c", self.test_dir());
        let parser = self.parser_src();
        self.compile(
            "test_ubsan",
            &[
                "-g",
                "-O1",
                "-fsanitize=undefined",
                "-fno-sanitize-recover=all",
                "-Wall",
                "-Wextra",
            ],
            &[&test_src, &parser],
            &[],
        )
    }

    /// Build benchmark binary.
    pub fn build_bench(&self) -> Result<String> {
        let bench_src = format!("{}/bench_filter_parse.c", self.test_dir());
        let parser = self.parser_src();
        self.compile(
            "bench",
            &["-O2", "-Wall", "-Wextra"],
            &[&bench_src, &parser],
            &[],
        )
    }

    /// Build callgrind benchmark binary.
    pub fn build_callgrind(&self) -> Result<String> {
        let bench_src = format!("{}/bench_filter_parse.c", self.test_dir());
        let parser = self.parser_src();
        self.compile(
            "bench_cg",
            &["-O2", "-g", "-Wall", "-Wextra"],
            &[&bench_src, &parser],
            &[],
        )
    }

    /// Build gen_connections binary.
    pub fn build_gen_conn(&self) -> Result<String> {
        let src = format!("{}/gen_connections.c", self.test_dir());
        let parser = self.parser_src();
        self.compile(
            "gen_connections",
            &["-O2", "-Wall", "-Wextra"],
            &[&src, &parser],
            &[],
        )
    }

    /// Build read_tcpstats binary.
    #[allow(dead_code)]
    pub fn build_read_tcpstats(&self) -> Result<String> {
        let src = format!("{}/read_tcpstats.c", self.test_dir());
        let parser = self.parser_src();
        self.compile(
            "read_tcpstats",
            &["-O2", "-Wall", "-Wextra"],
            &[&src, &parser],
            &[],
        )
    }

    /// Build bench_read_tcpstats binary.
    pub fn build_bench_read(&self) -> Result<String> {
        let src = format!("{}/bench_read_tcpstats.c", self.test_dir());
        self.compile(
            "bench_read_tcpstats",
            &["-O2", "-Wall", "-Wextra"],
            &[&src],
            &["-lpthread"],
        )
    }

    /// Build test_dos_limits binary.
    pub fn build_dos_limits(&self) -> Result<String> {
        let src = format!("{}/test_dos_limits.c", self.test_dir());
        self.compile(
            "test_dos_limits",
            &["-O2", "-Wall", "-Wextra"],
            &[&src],
            &[],
        )
    }

    /// Build kernel module.
    pub fn build_kmod(&self, extra_cflags: Option<&str>) -> Result<()> {
        let mut args = vec!["-C", self.kmod_src, "clean", "all"];
        let flag_str;
        if let Some(flags) = extra_cflags {
            flag_str = format!("EXTRA_CFLAGS={flags}");
            args.push(&flag_str);
        }
        run_cmd("make", &args)?;
        Ok(())
    }

    /// Check that required source files exist.
    #[allow(dead_code)]
    pub fn check_sources(&self) -> Result<()> {
        let test_dir = self.test_dir();
        let required = [
            format!("{}/test_filter_parse.c", test_dir),
            format!("{}/bench_filter_parse.c", test_dir),
            self.parser_src(),
        ];
        for path in &required {
            if !Path::new(path).exists() {
                anyhow::bail!("required source not found: {path}");
            }
        }
        Ok(())
    }
}
