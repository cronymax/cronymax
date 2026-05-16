//! `test_runner.*` built-in tools.
//!
//! Provides three tools to the agent loop:
//!
//! * `test_runner.discover` — scan the workspace for test suites.
//! * `test_runner.run_suite` — run a named suite; returns a
//!   [`TestRunnerResult`].
//! * `test_runner.get_last_report` — return the most recent
//!   `TestRunnerResult` stored per Flow Run, or `null`.
//!
//! ## Supported runners
//!
//! | Runner  | Detection heuristic                        | Invocation                              |
//! |---------|--------------------------------------------|-----------------------------------------|
//! | Jest    | `package.json` with `jest` in scripts/deps | `npx jest --json --forceExit`           |
//! | Vitest  | `package.json` with `vitest` in scripts    | `npx vitest run --reporter=json`        |
//! | Pytest  | `pyproject.toml` or `setup.cfg`            | `pytest --json-report --json-report-file=-` |
//! | Go test | `*_test.go` files                          | `go test -json ./...`                   |
//!
//! Producer-only restriction: the tools MUST NOT be registered for
//! reviewer agents. The registration helper logs a warning and skips
//! registration if the agent kind is `"reviewer"`.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;

use crate::agent_loop::tools::ToolOutcome;
use crate::llm::ToolDef;

// ── TestRunnerResult ──────────────────────────────────────────────────────────

/// Normalised result of a test suite run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRunnerResult {
    pub suite: String,
    pub runner: RunnerKind,
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
    pub duration_ms: u64,
    /// Failed test names and their error messages.
    pub failures: Vec<TestFailure>,
    /// Optional line coverage percentage (0–100).
    pub coverage: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestFailure {
    pub name: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunnerKind {
    Jest,
    Vitest,
    Pytest,
    GoTest,
}

// ── DiscoveredSuite ───────────────────────────────────────────────────────────

/// A test suite discovered in the workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSuite {
    /// Short human-readable name, e.g. `"jest (web)"` or `"pytest (backend)"`.
    pub name: String,
    pub runner: RunnerKind,
    /// Workspace-relative path to the project root for this suite.
    pub project_root: String,
}

// ── Last report store ─────────────────────────────────────────────────────────

/// In-memory store for the most recent `TestRunnerResult` per flow-run-id.
#[derive(Debug, Default)]
pub struct LastReportStore {
    inner: RwLock<HashMap<String, TestRunnerResult>>,
}

impl LastReportStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set(&self, run_id: &str, result: TestRunnerResult) {
        self.inner.write().insert(run_id.to_owned(), result);
    }

    pub fn get(&self, run_id: &str) -> Option<TestRunnerResult> {
        self.inner.read().get(run_id).cloned()
    }
}

// ── Discovery ────────────────────────────────────────────────────────────────

/// Scan `workspace_root` for recognisable test suites.
pub fn discover_suites(workspace_root: &Path) -> Vec<DiscoveredSuite> {
    let mut suites = Vec::new();

    // Walk top-level directories (and workspace root itself) for package.json.
    for entry in std::fs::read_dir(workspace_root)
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            let pkg = path.join("package.json");
            if pkg.exists() {
                if let Some(kind) = detect_js_runner(&pkg) {
                    let name = match kind {
                        RunnerKind::Jest => format!(
                            "jest ({})",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        RunnerKind::Vitest => format!(
                            "vitest ({})",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        _ => unreachable!(),
                    };
                    suites.push(DiscoveredSuite {
                        name,
                        runner: kind,
                        project_root: path.to_string_lossy().into_owned(),
                    });
                }
            }
        }
    }

    // Check workspace root package.json.
    let root_pkg = workspace_root.join("package.json");
    if root_pkg.exists() {
        if let Some(kind) = detect_js_runner(&root_pkg) {
            let name = match kind {
                RunnerKind::Jest => "jest (root)".into(),
                RunnerKind::Vitest => "vitest (root)".into(),
                _ => unreachable!(),
            };
            suites.push(DiscoveredSuite {
                name,
                runner: kind,
                project_root: workspace_root.to_string_lossy().into_owned(),
            });
        }
    }

    // Pytest: look for pyproject.toml or setup.cfg.
    for file in &["pyproject.toml", "setup.cfg"] {
        if workspace_root.join(file).exists() {
            suites.push(DiscoveredSuite {
                name: "pytest".into(),
                runner: RunnerKind::Pytest,
                project_root: workspace_root.to_string_lossy().into_owned(),
            });
            break;
        }
    }

    // Go test: look for any *_test.go files (walk shallow).
    if has_go_tests(workspace_root) {
        suites.push(DiscoveredSuite {
            name: "go test".into(),
            runner: RunnerKind::GoTest,
            project_root: workspace_root.to_string_lossy().into_owned(),
        });
    }

    suites
}

fn detect_js_runner(pkg_path: &Path) -> Option<RunnerKind> {
    let raw = std::fs::read_to_string(pkg_path).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;

    // Check devDependencies, dependencies, and scripts for vitest or jest.
    let has_dep = |name: &str| -> bool {
        v.get("devDependencies").and_then(|d| d.get(name)).is_some()
            || v.get("dependencies").and_then(|d| d.get(name)).is_some()
    };
    let scripts_contain = |name: &str| -> bool {
        v.get("scripts")
            .and_then(|s| s.as_object())
            .map(|m| m.values().any(|v| v.as_str().unwrap_or("").contains(name)))
            .unwrap_or(false)
    };

    if has_dep("vitest") || scripts_contain("vitest") {
        Some(RunnerKind::Vitest)
    } else if has_dep("jest") || scripts_contain("jest") {
        Some(RunnerKind::Jest)
    } else {
        None
    }
}

fn has_go_tests(root: &Path) -> bool {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            let p = e.path();
            p.is_file()
                && p.extension().map(|x| x == "go").unwrap_or(false)
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.ends_with("_test.go"))
                    .unwrap_or(false)
        })
}

// ── Runners ───────────────────────────────────────────────────────────────────

async fn run_jest(project_root: &Path, filter: Option<&str>) -> anyhow::Result<TestRunnerResult> {
    let mut cmd = Command::new("npx");
    cmd.arg("jest")
        .arg("--json")
        .arg("--forceExit")
        .current_dir(project_root);
    if let Some(f) = filter {
        cmd.arg(format!("--testPathPattern={f}"));
    }
    let start = Instant::now();
    let out = cmd.output().await?;
    let elapsed = start.elapsed();
    parse_jest_output(&out.stdout, &out.stderr, elapsed)
}

fn parse_jest_output(
    stdout: &[u8],
    stderr: &[u8],
    elapsed: Duration,
) -> anyhow::Result<TestRunnerResult> {
    // Jest writes its JSON reporter output to stdout.
    let json: Value = serde_json::from_slice(stdout).map_err(|e| {
        let err_text = String::from_utf8_lossy(stderr);
        anyhow::anyhow!("jest JSON parse error: {e}\nstderr: {err_text}")
    })?;

    let num_passed = json["numPassedTests"].as_u64().unwrap_or(0) as u32;
    let num_failed = json["numFailedTests"].as_u64().unwrap_or(0) as u32;
    let num_pending = json["numPendingTests"].as_u64().unwrap_or(0) as u32;
    let total = num_passed + num_failed + num_pending;

    let mut failures = Vec::new();
    if let Some(suites) = json["testResults"].as_array() {
        for suite in suites {
            if let Some(results) = suite["testResults"].as_array() {
                for t in results {
                    if t["status"].as_str() == Some("failed") {
                        let name = t["fullName"].as_str().unwrap_or("").to_string();
                        let message = t["failureMessages"]
                            .as_array()
                            .and_then(|m| m.first())
                            .and_then(|m| m.as_str())
                            .unwrap_or("")
                            .to_string();
                        failures.push(TestFailure { name, message });
                    }
                }
            }
        }
    }

    Ok(TestRunnerResult {
        suite: "jest".into(),
        runner: RunnerKind::Jest,
        total,
        passed: num_passed,
        failed: num_failed,
        skipped: num_pending,
        duration_ms: elapsed.as_millis() as u64,
        failures,
        coverage: None,
    })
}

async fn run_vitest(project_root: &Path, filter: Option<&str>) -> anyhow::Result<TestRunnerResult> {
    let mut cmd = Command::new("npx");
    cmd.arg("vitest")
        .arg("run")
        .arg("--reporter=json")
        .current_dir(project_root);
    if let Some(f) = filter {
        cmd.arg(f);
    }
    let start = Instant::now();
    let out = cmd.output().await?;
    let elapsed = start.elapsed();
    parse_vitest_output(&out.stdout, &out.stderr, elapsed)
}

fn parse_vitest_output(
    stdout: &[u8],
    _stderr: &[u8],
    elapsed: Duration,
) -> anyhow::Result<TestRunnerResult> {
    // Vitest JSON reporter writes to stdout.
    let json: Value = serde_json::from_slice(stdout)
        .map_err(|e| anyhow::anyhow!("vitest JSON parse error: {e}"))?;

    // Vitest JSON structure: { testResults: [ { assertionResults: [...] } ] }
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut failures = Vec::new();

    if let Some(suites) = json["testResults"].as_array() {
        for suite in suites {
            if let Some(assertions) = suite["assertionResults"].as_array() {
                for t in assertions {
                    match t["status"].as_str() {
                        Some("passed") => passed += 1,
                        Some("failed") => {
                            failed += 1;
                            let name = t["fullName"]
                                .as_str()
                                .or_else(|| t["title"].as_str())
                                .unwrap_or("")
                                .to_string();
                            let message = t["failureMessages"]
                                .as_array()
                                .and_then(|m| m.first())
                                .and_then(|m| m.as_str())
                                .unwrap_or("")
                                .to_string();
                            failures.push(TestFailure { name, message });
                        }
                        Some("pending") | Some("skipped") | Some("todo") => skipped += 1,
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(TestRunnerResult {
        suite: "vitest".into(),
        runner: RunnerKind::Vitest,
        total: passed + failed + skipped,
        passed,
        failed,
        skipped,
        duration_ms: elapsed.as_millis() as u64,
        failures,
        coverage: None,
    })
}

async fn run_pytest(project_root: &Path, filter: Option<&str>) -> anyhow::Result<TestRunnerResult> {
    let mut cmd = Command::new("pytest");
    cmd.arg("--json-report")
        .arg("--json-report-file=-")
        .current_dir(project_root);
    if let Some(f) = filter {
        cmd.arg(f);
    }
    let start = Instant::now();
    let out = cmd.output().await?;
    let elapsed = start.elapsed();
    parse_pytest_output(&out.stdout, &out.stderr, elapsed)
}

fn parse_pytest_output(
    stdout: &[u8],
    _stderr: &[u8],
    elapsed: Duration,
) -> anyhow::Result<TestRunnerResult> {
    let json: Value = serde_json::from_slice(stdout)
        .map_err(|e| anyhow::anyhow!("pytest JSON parse error: {e}"))?;

    let summary = &json["summary"];
    let passed = summary["passed"].as_u64().unwrap_or(0) as u32;
    let failed = summary["failed"].as_u64().unwrap_or(0) as u32;
    let skipped = summary["skipped"].as_u64().unwrap_or(0) as u32;
    let total = summary["total"]
        .as_u64()
        .unwrap_or((passed + failed + skipped) as u64) as u32;

    let mut failures = Vec::new();
    if let Some(tests) = json["tests"].as_array() {
        for t in tests {
            if t["outcome"].as_str() == Some("failed") {
                let name = t["nodeid"].as_str().unwrap_or("").to_string();
                let message = t["call"]["longrepr"].as_str().unwrap_or("").to_string();
                failures.push(TestFailure { name, message });
            }
        }
    }

    Ok(TestRunnerResult {
        suite: "pytest".into(),
        runner: RunnerKind::Pytest,
        total,
        passed,
        failed,
        skipped,
        duration_ms: elapsed.as_millis() as u64,
        failures,
        coverage: None,
    })
}

async fn run_go_test(
    project_root: &Path,
    filter: Option<&str>,
) -> anyhow::Result<TestRunnerResult> {
    let mut cmd = Command::new("go");
    cmd.arg("test").arg("-json").current_dir(project_root);
    if let Some(f) = filter {
        cmd.arg(f);
    } else {
        cmd.arg("./...");
    }
    let start = Instant::now();
    let out = cmd.output().await?;
    let elapsed = start.elapsed();
    parse_go_test_output(&out.stdout, &out.stderr, elapsed)
}

fn parse_go_test_output(
    stdout: &[u8],
    _stderr: &[u8],
    elapsed: Duration,
) -> anyhow::Result<TestRunnerResult> {
    // Go test -json emits one JSON object per line.
    let text = std::str::from_utf8(stdout)
        .map_err(|e| anyhow::anyhow!("go test output is not UTF-8: {e}"))?;

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut skipped = 0u32;
    let mut failures: Vec<TestFailure> = Vec::new();
    let mut failure_outputs: HashMap<String, Vec<String>> = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let action = v["Action"].as_str().unwrap_or("");
        let test_name = v["Test"].as_str().unwrap_or("").to_string();
        match action {
            "pass" if !test_name.is_empty() => passed += 1,
            "fail" if !test_name.is_empty() => {
                failed += 1;
                let message = failure_outputs
                    .remove(&test_name)
                    .map(|lines| lines.join(""))
                    .unwrap_or_default();
                failures.push(TestFailure {
                    name: test_name,
                    message,
                });
            }
            "skip" if !test_name.is_empty() => skipped += 1,
            "output" if !test_name.is_empty() => {
                if let Some(output) = v["Output"].as_str() {
                    failure_outputs
                        .entry(test_name)
                        .or_default()
                        .push(output.to_string());
                }
            }
            _ => {}
        }
    }

    Ok(TestRunnerResult {
        suite: "go test".into(),
        runner: RunnerKind::GoTest,
        total: passed + failed + skipped,
        passed,
        failed,
        skipped,
        duration_ms: elapsed.as_millis() as u64,
        failures,
        coverage: None,
    })
}

// ── Public dispatch surface ───────────────────────────────────────────────────

/// Discover suites from `workspace_root`.
pub async fn tool_discover(workspace_root: &Path) -> ToolOutcome {
    let suites = discover_suites(workspace_root);
    ToolOutcome::Output(serde_json::to_value(&suites).unwrap_or_else(|_| Value::Array(vec![])))
}

/// Run a named suite. `suite_name` must match a `DiscoveredSuite::name`.
/// `filter` is an optional test-name / path filter (runner-specific).
pub async fn tool_run_suite(
    workspace_root: &Path,
    suite_name: &str,
    filter: Option<&str>,
    store: &LastReportStore,
    run_id: &str,
) -> ToolOutcome {
    let suites = discover_suites(workspace_root);
    let suite = match suites.iter().find(|s| s.name == suite_name) {
        Some(s) => s,
        None => {
            return ToolOutcome::Error(format!(
                "suite '{}' not found; available: {:?}",
                suite_name,
                suites.iter().map(|s| &s.name).collect::<Vec<_>>()
            ))
        }
    };

    let project_root = PathBuf::from(&suite.project_root);
    let result = match suite.runner {
        RunnerKind::Jest => run_jest(&project_root, filter).await,
        RunnerKind::Vitest => run_vitest(&project_root, filter).await,
        RunnerKind::Pytest => run_pytest(&project_root, filter).await,
        RunnerKind::GoTest => run_go_test(&project_root, filter).await,
    };

    match result {
        Ok(r) => {
            store.set(run_id, r.clone());
            ToolOutcome::Output(serde_json::to_value(&r).unwrap_or(Value::Null))
        }
        Err(e) => ToolOutcome::Error(format!("run_suite failed: {e}")),
    }
}

/// Return the last stored report for `run_id`, or `null`.
pub async fn tool_get_last_report(store: &LastReportStore, run_id: &str) -> ToolOutcome {
    match store.get(run_id) {
        Some(r) => ToolOutcome::Output(serde_json::to_value(&r).unwrap_or(Value::Null)),
        None => ToolOutcome::Output(Value::Null),
    }
}

// ── ToolDef builders ─────────────────────────────────────────────────────────

pub fn discover_tool_def() -> ToolDef {
    ToolDef {
        name: "test_runner.discover".into(),
        description: "Scan the workspace for test suites (Jest, Vitest, Pytest, Go test). \
                       Returns a list of DiscoveredSuite objects."
            .into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    }
}

pub fn run_suite_tool_def() -> ToolDef {
    ToolDef {
        name: "test_runner.run_suite".into(),
        description: "Run a test suite discovered by test_runner.discover. \
                       Returns a TestRunnerResult with pass/fail counts and failure details."
            .into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "suite": {
                    "type": "string",
                    "description": "The suite name returned by test_runner.discover"
                },
                "filter": {
                    "type": "string",
                    "description": "Optional test-name or path filter (runner-specific)"
                }
            },
            "required": ["suite"]
        }),
    }
}

pub fn get_last_report_tool_def() -> ToolDef {
    ToolDef {
        name: "test_runner.get_last_report".into(),
        description: "Return the most recent TestRunnerResult for the current flow run, \
                       or null if no test has been run yet."
            .into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::TempDir;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    // ── Jest parser ──────────────────────────────────────────────────────

    #[test]
    fn parse_jest_all_passing() {
        let json = serde_json::json!({
            "numPassedTests": 10,
            "numFailedTests": 0,
            "numPendingTests": 2,
            "testResults": []
        });
        let stdout = serde_json::to_vec(&json).unwrap();
        let result = parse_jest_output(&stdout, b"", ms(500)).unwrap();
        assert_eq!(result.passed, 10);
        assert_eq!(result.failed, 0);
        assert_eq!(result.skipped, 2);
        assert_eq!(result.total, 12);
        assert!(result.failures.is_empty());
    }

    #[test]
    fn parse_jest_with_failures() {
        let json = serde_json::json!({
            "numPassedTests": 3,
            "numFailedTests": 1,
            "numPendingTests": 0,
            "testResults": [{
                "testResults": [{
                    "status": "failed",
                    "fullName": "MyTest fails on null",
                    "failureMessages": ["Expected true but got false"]
                }]
            }]
        });
        let stdout = serde_json::to_vec(&json).unwrap();
        let result = parse_jest_output(&stdout, b"", ms(200)).unwrap();
        assert_eq!(result.failed, 1);
        assert_eq!(result.failures[0].name, "MyTest fails on null");
    }

    // ── Vitest parser ────────────────────────────────────────────────────

    #[test]
    fn parse_vitest_mixed() {
        let json = serde_json::json!({
            "testResults": [{
                "assertionResults": [
                    {"status": "passed", "fullName": "a"},
                    {"status": "failed", "fullName": "b", "failureMessages": ["err"]},
                    {"status": "skipped", "fullName": "c"}
                ]
            }]
        });
        let stdout = serde_json::to_vec(&json).unwrap();
        let result = parse_vitest_output(&stdout, b"", ms(100)).unwrap();
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed, 1);
        assert_eq!(result.skipped, 1);
        assert_eq!(result.total, 3);
    }

    // ── Pytest parser ─────────────────────────────────────────────────────

    #[test]
    fn parse_pytest_summary() {
        let json = serde_json::json!({
            "summary": { "passed": 8, "failed": 2, "skipped": 1, "total": 11 },
            "tests": [
                { "outcome": "failed", "nodeid": "test_foo.py::test_bar",
                  "call": { "longrepr": "AssertionError" } }
            ]
        });
        let stdout = serde_json::to_vec(&json).unwrap();
        let result = parse_pytest_output(&stdout, b"", ms(300)).unwrap();
        assert_eq!(result.passed, 8);
        assert_eq!(result.failed, 2);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].name, "test_foo.py::test_bar");
    }

    // ── Go test parser ────────────────────────────────────────────────────

    #[test]
    fn parse_go_test_output_basic() {
        let lines = [
            r#"{"Action":"run","Test":"TestA"}"#,
            r#"{"Action":"output","Test":"TestA","Output":"--- PASS: TestA\n"}"#,
            r#"{"Action":"pass","Test":"TestA"}"#,
            r#"{"Action":"run","Test":"TestB"}"#,
            r#"{"Action":"output","Test":"TestB","Output":"panic: nil pointer\n"}"#,
            r#"{"Action":"fail","Test":"TestB"}"#,
        ];
        let stdout = lines.join("\n").into_bytes();
        let result = parse_go_test_output(&stdout, b"", ms(150)).unwrap();
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed, 1);
        assert_eq!(result.failures[0].name, "TestB");
    }

    // ── Discover ──────────────────────────────────────────────────────────

    #[test]
    fn discover_finds_jest_suite() {
        let dir = TempDir::new().unwrap();
        let pkg = serde_json::json!({
            "name": "my-app",
            "devDependencies": { "jest": "^29" }
        });
        std::fs::write(
            dir.path().join("package.json"),
            serde_json::to_string(&pkg).unwrap(),
        )
        .unwrap();

        let suites = discover_suites(dir.path());
        assert!(suites.iter().any(|s| s.runner == RunnerKind::Jest));
    }

    #[test]
    fn discover_finds_vitest_suite() {
        let dir = TempDir::new().unwrap();
        let pkg = serde_json::json!({
            "name": "my-app",
            "devDependencies": { "vitest": "^1" }
        });
        std::fs::write(
            dir.path().join("package.json"),
            serde_json::to_string(&pkg).unwrap(),
        )
        .unwrap();

        let suites = discover_suites(dir.path());
        assert!(suites.iter().any(|s| s.runner == RunnerKind::Vitest));
    }

    #[test]
    fn discover_finds_pytest_suite() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "[tool.pytest]\n").unwrap();

        let suites = discover_suites(dir.path());
        assert!(suites.iter().any(|s| s.runner == RunnerKind::Pytest));
    }

    #[test]
    fn discover_finds_go_test() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("main_test.go"), "package main\n").unwrap();

        let suites = discover_suites(dir.path());
        assert!(suites.iter().any(|s| s.runner == RunnerKind::GoTest));
    }

    // ── LastReportStore ───────────────────────────────────────────────────

    #[test]
    fn last_report_store_set_and_get() {
        let store = LastReportStore::new();
        assert!(store.get("run-1").is_none());
        store.set(
            "run-1",
            TestRunnerResult {
                suite: "jest".into(),
                runner: RunnerKind::Jest,
                total: 5,
                passed: 5,
                failed: 0,
                skipped: 0,
                duration_ms: 100,
                failures: vec![],
                coverage: None,
            },
        );
        assert_eq!(store.get("run-1").unwrap().passed, 5);
    }
}
