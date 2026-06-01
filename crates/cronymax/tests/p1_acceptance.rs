//! Phase 1 acceptance demo (`P1-T06`).
//!
//! Drives the `cronymax` CLI end-to-end against a pure-declaration
//! extension (no Node host yet — that lands in Phase 2). The shape of this
//! test is the public contract for the install/list/enable/disable/uninstall
//! lifecycle.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

/// Path to the CLI binary built by Cargo. Provided by Cargo at compile time
/// because `[[bin]] name = "cronymax"` lives in the same crate's manifest.
const CRONYMAX_BIN: &str = env!("CARGO_BIN_EXE_cronymax");

fn write_pure_declaration_extension(root: &Path, id: &str, publisher: &str) {
    fs::create_dir_all(root.join("dist")).unwrap();
    fs::write(root.join("dist/main.js"), b"// hello from Phase 1\n").unwrap();
    let manifest = format!(
        r#"{{
            "id": "{id}",
            "name": "Phase 1 Demo",
            "version": "0.1.0",
            "publisher": "{publisher}",
            "engines": {{ "cronymax": "^1.0" }},
            "main": "./dist/main.js",
            "activationEvents": ["onCommand:{id}.hello"],
            "contributes": {{
                "cronymax.command": [
                    {{ "id": "{id}.hello", "title": "Hello" }}
                ]
            }},
            "capabilities": {{
                "extension-points": ["cronymax.command"]
            }}
        }}"#
    );
    fs::write(root.join("cronymax-extension.json"), manifest).unwrap();
}

fn run_cli(reg_root: &Path, args: &[&str]) -> Output {
    let mut full = vec!["--root".to_string(), reg_root.display().to_string()];
    full.extend(args.iter().map(|s| s.to_string()));
    Command::new(CRONYMAX_BIN)
        .args(&full)
        .output()
        .expect("invoking cronymax binary")
}

fn assert_ok(out: &Output, what: &str) {
    if !out.status.success() {
        panic!(
            "{what} failed: status={:?}\nstdout: {}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

#[test]
fn pure_declaration_extension_lifecycle() {
    let reg_root = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    write_pure_declaration_extension(src.path(), "alice.phase1", "alice");

    // 1. `list` on a fresh root is empty.
    let out = run_cli(reg_root.path(), &["ext", "list"]);
    assert_ok(&out, "ext list (empty)");
    assert!(
        stdout(&out).contains("no extensions installed"),
        "fresh list should report empty; got:\n{}",
        stdout(&out),
    );

    // 2. `install <src>` succeeds and reports the id + version.
    let out = run_cli(
        reg_root.path(),
        &["ext", "install", &src.path().display().to_string()],
    );
    assert_ok(&out, "ext install");
    assert!(stdout(&out).contains("installed alice.phase1 (0.1.0)"));
    assert!(
        reg_root
            .path()
            .join("alice.phase1/cronymax-extension.json")
            .is_file(),
        "manifest copied to registry",
    );
    assert!(
        reg_root.path().join("registry.json").is_file(),
        "registry.json persisted",
    );

    // 3. `list` shows the extension as enabled.
    let out = run_cli(reg_root.path(), &["ext", "list"]);
    assert_ok(&out, "ext list (after install)");
    let listing = stdout(&out);
    assert!(listing.contains("alice.phase1"));
    assert!(listing.contains("0.1.0"));
    assert!(
        listing.contains("enabled"),
        "expected enabled, got:\n{listing}"
    );

    // 4. `disable` flips the flag.
    let out = run_cli(reg_root.path(), &["ext", "disable", "alice.phase1"]);
    assert_ok(&out, "ext disable");
    assert!(stdout(&out).contains("disabled alice.phase1"));

    let out = run_cli(reg_root.path(), &["ext", "list"]);
    assert_ok(&out, "ext list (after disable)");
    assert!(
        stdout(&out).contains("disabled"),
        "expected disabled in listing"
    );

    // 5. `enable` flips it back.
    let out = run_cli(reg_root.path(), &["ext", "enable", "alice.phase1"]);
    assert_ok(&out, "ext enable");
    assert!(stdout(&out).contains("enabled alice.phase1"));

    // 6. Re-install on top of an existing id fails cleanly without
    // corrupting state.
    let out = run_cli(
        reg_root.path(),
        &["ext", "install", &src.path().display().to_string()],
    );
    assert!(!out.status.success(), "second install must fail");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already installed"),
        "stderr should mention duplicate; got: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    // Registry still shows the original entry intact.
    let out = run_cli(reg_root.path(), &["ext", "list"]);
    assert_ok(&out, "ext list (after failed dup install)");
    assert!(stdout(&out).contains("alice.phase1"));

    // 7. `uninstall` removes the dir and the entry.
    let out = run_cli(reg_root.path(), &["ext", "uninstall", "alice.phase1"]);
    assert_ok(&out, "ext uninstall");
    assert!(stdout(&out).contains("uninstalled alice.phase1"));
    assert!(
        !reg_root.path().join("alice.phase1").exists(),
        "uninstall must delete the dir",
    );

    // 8. `list` is empty again.
    let out = run_cli(reg_root.path(), &["ext", "list"]);
    assert_ok(&out, "ext list (after uninstall)");
    assert!(stdout(&out).contains("no extensions installed"));
}

#[test]
fn install_with_invalid_manifest_fails_atomically() {
    let reg_root = TempDir::new().unwrap();
    let src = TempDir::new().unwrap();
    // publisher mismatch — should be caught by `validate`.
    write_pure_declaration_extension(src.path(), "alice.bad", "bob");

    let out = run_cli(
        reg_root.path(),
        &["ext", "install", &src.path().display().to_string()],
    );
    assert!(!out.status.success(), "install must reject bad manifest");
    assert!(
        !reg_root.path().join("alice.bad").exists(),
        "no copy on validation failure",
    );
}

#[test]
fn unknown_subcommand_errors_without_crashing() {
    let reg_root = TempDir::new().unwrap();
    let out = run_cli(reg_root.path(), &["ext", "frobnicate"]);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown subcommand"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn help_flag_exits_clean() {
    let reg_root = TempDir::new().unwrap();
    let out = run_cli(reg_root.path(), &["--help"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("USAGE:"));
}
