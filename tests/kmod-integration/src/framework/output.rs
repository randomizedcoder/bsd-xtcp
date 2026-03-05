use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use chrono::Local;

/// Top-level output directory for a single run.
/// Created once in `main()`, owns the timestamped root directory.
pub struct RunOutput {
    root: PathBuf,
    results: Vec<TargetResult>,
    run_start: Instant,
    timestamp: String,
    #[allow(dead_code)]
    run_log: Option<File>,
}

/// Result of running one target (compile or live).
pub struct TargetResult {
    pub name: String,
    pub status: TargetStatus,
    pub duration_secs: f64,
    pub sub_tests: Vec<SubTestResult>,
}

/// Result of a single sub-test within a target.
pub struct SubTestResult {
    pub name: String,
    pub status: TargetStatus,
    pub duration_secs: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TargetStatus {
    Pass,
    Fail,
}

impl std::fmt::Display for TargetStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetStatus::Pass => write!(f, "PASS"),
            TargetStatus::Fail => write!(f, "FAIL"),
        }
    }
}

/// Writes lines with `[timestamp] ` prefix to a file.
#[allow(dead_code)]
pub struct TimestampedWriter {
    file: File,
}

#[allow(dead_code)]
impl TimestampedWriter {
    fn new(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("create log {}", path.display()))?;
        Ok(Self { file })
    }

    pub fn writeln(&mut self, msg: &str) {
        let ts = Local::now().format("%Y-%m-%dT%H:%M:%S%.3f");
        let _ = writeln!(self.file, "[{ts}] {msg}");
    }
}

/// Per-target output context. Creates a subdirectory under the run root
/// and owns `stdout.log` / `stderr.log` writers.
#[allow(dead_code)]
pub struct TargetLog {
    dir: PathBuf,
    stdout_writer: TimestampedWriter,
    stderr_writer: TimestampedWriter,
    run_log: Option<File>,
}

#[allow(dead_code)]
impl TargetLog {
    fn new(parent: &Path, name: &str, run_log: Option<&File>) -> Result<Self> {
        let dir = parent.join(name);
        fs::create_dir_all(&dir)
            .with_context(|| format!("create target dir {}", dir.display()))?;

        let stdout_writer = TimestampedWriter::new(&dir.join("stdout.log"))?;
        let stderr_writer = TimestampedWriter::new(&dir.join("stderr.log"))?;

        let run_log = run_log.and_then(|f| f.try_clone().ok());

        Ok(Self {
            dir,
            stdout_writer,
            stderr_writer,
            run_log,
        })
    }

    /// Write to terminal AND stdout.log with timestamp.
    pub fn println(&mut self, msg: &str) {
        println!("{msg}");
        self.stdout_writer.writeln(msg);
        if let Some(ref mut f) = self.run_log {
            let ts = Local::now().format("%Y-%m-%dT%H:%M:%S%.3f");
            let _ = writeln!(f, "[{ts}] {msg}");
        }
    }

    /// Write to terminal stderr AND stderr.log with timestamp.
    pub fn eprintln(&mut self, msg: &str) {
        eprintln!("{msg}");
        self.stderr_writer.writeln(msg);
        if let Some(ref mut f) = self.run_log {
            let ts = Local::now().format("%Y-%m-%dT%H:%M:%S%.3f");
            let _ = writeln!(f, "[{ts}] [ERR] {msg}");
        }
    }

    /// Returns the `processes/` subdir path (creates it).
    pub fn process_dir(&self) -> PathBuf {
        let p = self.dir.join("processes");
        let _ = fs::create_dir_all(&p);
        p
    }

    /// Returns a named subdirectory (creates it).
    pub fn sub_dir(&self, name: &str) -> PathBuf {
        let p = self.dir.join(name);
        let _ = fs::create_dir_all(&p);
        p
    }

    /// Returns the target directory path.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl RunOutput {
    /// Create a new timestamped output directory under `/tmp/kmod-integration/`.
    pub fn new() -> Result<Self> {
        let timestamp = Local::now().format("%Y-%m-%d-%H-%M-%S").to_string();
        let root = PathBuf::from("/tmp/kmod-integration").join(&timestamp);
        Self::new_at(root, timestamp)
    }

    /// Create a new output directory at a specific path.
    pub fn new_at(root: PathBuf, timestamp: String) -> Result<Self> {
        fs::create_dir_all(&root)
            .with_context(|| format!("create output dir {}", root.display()))?;

        let run_log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("run.log"))
            .ok();

        Ok(Self {
            root,
            results: Vec::new(),
            run_start: Instant::now(),
            timestamp,
            run_log,
        })
    }

    /// The root output directory path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create a `TargetLog` for a named target.
    #[allow(dead_code)]
    pub fn target(&self, name: &str) -> Result<TargetLog> {
        TargetLog::new(&self.root, name, self.run_log.as_ref())
    }

    /// Record a target result for summary generation.
    pub fn record(&mut self, result: TargetResult) {
        self.results.push(result);
    }

    /// Write `summary.txt` and `summary.json` to the output directory.
    pub fn write_summary(&self) -> Result<()> {
        self.write_summary_txt()?;
        self.write_summary_json()?;
        Ok(())
    }

    fn write_summary_txt(&self) -> Result<()> {
        let path = self.root.join("summary.txt");
        let mut f = File::create(&path)
            .with_context(|| format!("create {}", path.display()))?;

        let total_duration = self.run_start.elapsed().as_secs_f64();

        writeln!(f, "kmod-integration run: {}", self.timestamp)?;
        writeln!(f, "Total duration: {total_duration:.1}s")?;
        writeln!(f)?;
        writeln!(f, "{:<24} {:<8} {:>10}", "TARGET", "STATUS", "DURATION")?;
        writeln!(f, "{}", "-".repeat(44))?;

        let mut targets_passed = 0u32;
        let mut targets_failed = 0u32;
        let mut subs_passed = 0u32;
        let mut subs_failed = 0u32;

        for r in &self.results {
            writeln!(
                f,
                "{:<24} {:<8} {:>9.1}s",
                r.name, r.status, r.duration_secs
            )?;
            match r.status {
                TargetStatus::Pass => targets_passed += 1,
                TargetStatus::Fail => targets_failed += 1,
            }
            for s in &r.sub_tests {
                writeln!(
                    f,
                    "  {:<22} {:<8} {:>9.1}s",
                    s.name, s.status, s.duration_secs
                )?;
                match s.status {
                    TargetStatus::Pass => subs_passed += 1,
                    TargetStatus::Fail => subs_failed += 1,
                }
            }
        }

        writeln!(f, "{}", "-".repeat(44))?;
        write!(
            f,
            "TOTAL: {targets_passed} passed, {targets_failed} failed"
        )?;
        let total_subs = subs_passed + subs_failed;
        if total_subs > 0 {
            write!(
                f,
                " ({total_subs} sub-tests: {subs_passed} passed, {subs_failed} failed)"
            )?;
        }
        writeln!(f)?;

        Ok(())
    }

    fn write_summary_json(&self) -> Result<()> {
        let path = self.root.join("summary.json");
        let mut f = File::create(&path)
            .with_context(|| format!("create {}", path.display()))?;

        let total_duration = self.run_start.elapsed().as_secs_f64();

        let mut targets_passed = 0u32;
        let mut targets_failed = 0u32;
        let mut subs_passed = 0u32;
        let mut subs_failed = 0u32;

        // Build targets JSON array manually
        let mut targets_json = Vec::new();
        for r in &self.results {
            match r.status {
                TargetStatus::Pass => targets_passed += 1,
                TargetStatus::Fail => targets_failed += 1,
            }

            let status_str = match r.status {
                TargetStatus::Pass => "pass",
                TargetStatus::Fail => "fail",
            };

            let mut sub_json = Vec::new();
            for s in &r.sub_tests {
                match s.status {
                    TargetStatus::Pass => subs_passed += 1,
                    TargetStatus::Fail => subs_failed += 1,
                }
                let ss = match s.status {
                    TargetStatus::Pass => "pass",
                    TargetStatus::Fail => "fail",
                };
                sub_json.push(format!(
                    "      {{ \"name\": \"{}\", \"status\": \"{ss}\", \"duration_secs\": {:.1} }}",
                    escape_json(&s.name),
                    s.duration_secs
                ));
            }

            let sub_tests_str = if sub_json.is_empty() {
                "[]".to_string()
            } else {
                format!("[\n{}\n    ]", sub_json.join(",\n"))
            };

            targets_json.push(format!(
                "    {{ \"name\": \"{}\", \"status\": \"{status_str}\", \"duration_secs\": {:.1}, \"sub_tests\": {sub_tests_str} }}",
                escape_json(&r.name),
                r.duration_secs
            ));
        }

        let targets_str = if targets_json.is_empty() {
            "[]".to_string()
        } else {
            format!("[\n{}\n  ]", targets_json.join(",\n"))
        };

        write!(
            f,
            "{{\n  \"timestamp\": \"{}\",\n  \"total_duration_secs\": {:.1},\n  \"targets\": {targets_str},\n  \"totals\": {{ \"targets_passed\": {targets_passed}, \"targets_failed\": {targets_failed}, \"sub_tests_passed\": {subs_passed}, \"sub_tests_failed\": {subs_failed} }}\n}}\n",
            escape_json(&self.timestamp),
            total_duration
        )?;

        Ok(())
    }
}

/// Minimal JSON string escaping (backslash and double-quote).
fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
