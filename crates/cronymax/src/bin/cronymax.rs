//! `cronymax` — user-facing CLI for the extension platform.
//!
//! v1 alpha exposes one verb group: `ext`. Subcommands here drive
//! [`cronymax::extensions::ExtensionRegistry`] end-to-end. Argv parsing is
//! hand-rolled to keep this binary free of clap (workspace policy: add deps
//! only when a task needs them).
//!
//! Default registry root: `~/.cronymax/extensions/` (or `%USERPROFILE%\.cronymax\extensions\`).
//! Override with `--root <dir>`.

use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use cronymax::extensions::host::node::SpawnConfig;
use cronymax::extensions::logging::LogManager;
use cronymax::extensions::runtime::SpawnConfigBuilder;
use cronymax::extensions::{
    default_bundled_bootstrap, default_bundled_node, default_registry_root, diagnostic, package,
    ExtensionRegistry, ExtensionRuntime, Manifest,
};

const HELP: &str = r#"cronymax — extension platform CLI

USAGE:
    cronymax <command> [args...]

COMMANDS:
    ext install <path>     Install an extension from a directory or .cmx archive
    ext list               List installed extensions
    ext enable <id>        Mark an extension as enabled
    ext disable <id>       Mark an extension as disabled
    ext uninstall <id>     Uninstall an extension
    ext package <dir>      Pack an extension directory into a .cmx archive
    ext dev <dir>          Run an extension from source, streaming its logs to
                           the terminal (add --watch to reload on file change)
    diagnostic-bundle      Collect logs + manifests + versions into a redacted zip

GLOBAL OPTIONS:
    --root <dir>           Override the extensions root
                           (default: ~/.cronymax/extensions)
    -o, --output <file>    Output path for `ext package` / `diagnostic-bundle`
                           (defaults: <id>-<version>.cmx / cronymax-diagnostic-<ts>.zip)
    -h, --help             Show this help
    --version              Show version
"#;

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().skip(1).collect();
    match run(&argv) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(argv: &[String]) -> Result<(), String> {
    let mut args: Vec<String> = argv.to_vec();

    // Strip global flags first so they can appear anywhere.
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return Ok(());
    }
    if args.iter().any(|a| a == "--version") {
        println!("cronymax {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let root_override = take_value_flag(&mut args, "--root")?;
    let mut output_override = take_value_flag(&mut args, "-o")?;
    if output_override.is_none() {
        output_override = take_value_flag(&mut args, "--output")?;
    }

    if args.is_empty() {
        eprint!("{HELP}");
        return Err("no command given".into());
    }

    let group = args.remove(0);

    // `diagnostic-bundle` is a top-level command (not under `ext`): collect
    // logs + manifests + versions into a redacted zip for bug reports.
    if group == "diagnostic-bundle" {
        return run_diagnostic_bundle(root_override.as_deref(), output_override.as_deref());
    }

    if group != "ext" {
        return Err(format!("unknown command `{group}`; try `cronymax --help`"));
    }
    if args.is_empty() {
        return Err(
            "`cronymax ext` requires a subcommand (install/list/enable/disable/uninstall/package)"
                .into(),
        );
    }
    let sub = args.remove(0);

    // `package` operates on a source tree, not the registry — handle it before
    // touching the (possibly nonexistent) registry root.
    if sub == "package" {
        let dir = require_one_arg(&args, "package", "<source-dir>")?;
        return run_package(Path::new(dir), output_override.as_deref());
    }

    // `dev` runs the extension from source in a throwaway registry — also
    // independent of the managed root.
    if sub == "dev" {
        let watch = take_bool_flag(&mut args, "--watch");
        let dir = require_one_arg(&args, "dev", "<source-dir>")?;
        return run_dev(Path::new(dir), watch);
    }

    let root = resolve_root(root_override.as_deref())?;
    let mut reg = ExtensionRegistry::new(&root);
    reg.refresh().map_err(|e| e.to_string())?;

    match sub.as_str() {
        "install" => {
            let src = require_one_arg(&args, "install", "<path>")?;
            let id = reg
                .install_from_path(Path::new(src))
                .map_err(|e| e.to_string())?;
            let entry = reg.get(&id).expect("just installed");
            println!(
                "installed {} ({}) → {}",
                entry.manifest.id,
                entry.manifest.version,
                entry.ext_dir.display(),
            );
        }
        "list" => {
            print_list(&reg);
        }
        "enable" => {
            let id = require_one_arg(&args, "enable", "<id>")?;
            reg.set_enabled(id, true).map_err(|e| e.to_string())?;
            println!("enabled {id}");
        }
        "disable" => {
            let id = require_one_arg(&args, "disable", "<id>")?;
            reg.set_enabled(id, false).map_err(|e| e.to_string())?;
            println!("disabled {id}");
        }
        "uninstall" => {
            let id = require_one_arg(&args, "uninstall", "<id>")?;
            reg.uninstall(id).map_err(|e| e.to_string())?;
            println!("uninstalled {id}");
        }
        other => {
            return Err(format!(
                "unknown subcommand `cronymax ext {other}`; try `cronymax --help`"
            ));
        }
    }
    Ok(())
}

/// Pack `dir` into a `.cmx`. Output path defaults to `<id>-<version>.cmx`
/// (read from the manifest) in the current directory.
fn run_package(dir: &Path, output_override: Option<&str>) -> Result<(), String> {
    let out_path = match output_override {
        Some(o) => PathBuf::from(o),
        None => {
            let manifest_path = dir.join("cronymax-extension.json");
            let raw = std::fs::read_to_string(&manifest_path)
                .map_err(|e| format!("could not read {}: {e}", manifest_path.display()))?;
            let manifest = Manifest::from_json(&raw).map_err(|e| e.to_string())?;
            PathBuf::from(format!("{}-{}.cmx", manifest.id, manifest.version))
        }
    };
    package::pack_dir_to_cmx(dir, &out_path).map_err(|e| e.to_string())?;
    println!("packaged {} → {}", dir.display(), out_path.display());
    Ok(())
}

/// Collect logs + manifests + versions into a redacted diagnostic zip.
/// `--root` overrides the extensions root; logs are read from its sibling
/// `logs/` under the same `~/.cronymax` base.
fn run_diagnostic_bundle(
    root_override: Option<&str>,
    output_override: Option<&str>,
) -> Result<(), String> {
    let extensions_root = resolve_root(root_override)?; // ~/.cronymax/extensions
    let base = extensions_root
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| extensions_root.clone()); // ~/.cronymax
    let logs_root = base.join("logs");
    let home = home_dir();

    let generated_at_ms = now_ms();
    let out_path = match output_override {
        Some(o) => PathBuf::from(o),
        None => PathBuf::from(format!(
            "cronymax-diagnostic-{}.zip",
            generated_at_ms / 1000
        )),
    };

    let inputs = diagnostic::BundleInputs {
        logs_root: &logs_root,
        extensions_root: &extensions_root,
        home: &home,
        versions: diagnostic::Versions {
            cronymax: env!("CARGO_PKG_VERSION").to_owned(),
            os: os_description(),
            node: node_version(),
        },
        generated_at_ms,
    };
    let written =
        diagnostic::build_diagnostic_bundle(&inputs, &out_path).map_err(|e| e.to_string())?;
    println!("wrote diagnostic bundle → {}", written.display());
    Ok(())
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// OS + arch, plus a best-effort `uname -mrs` kernel string when available.
fn os_description() -> String {
    let base = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
    if let Ok(out) = std::process::Command::new("uname").arg("-mrs").output() {
        if out.status.success() {
            let kernel = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            if !kernel.is_empty() {
                return format!("{base} ({kernel})");
            }
        }
    }
    base
}

/// Probe the bundled Node 26 (`CRONYMAX_NODE` override, else resolved bundle)
/// for its `--version`. `None` if it can't be found or run.
fn node_version() -> Option<String> {
    let node = std::env::var_os("CRONYMAX_NODE")
        .map(PathBuf::from)
        .or_else(default_bundled_node)?;
    let out = std::process::Command::new(node)
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn print_list(reg: &ExtensionRegistry) {
    let mut entries: Vec<_> = reg.iter().collect();
    entries.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    if entries.is_empty() {
        println!("no extensions installed (root: {})", reg.root().display());
        return;
    }
    println!("{:<32} {:<10} {:<10} PATH", "ID", "VERSION", "STATE");
    for e in entries {
        let state = if e.enabled { "enabled" } else { "disabled" };
        println!(
            "{:<32} {:<10} {:<10} {}",
            e.manifest.id,
            e.manifest.version,
            state,
            e.ext_dir.display(),
        );
    }
}

/// Find `flag` in `args` and remove `flag` plus its value. Returns the value
/// (or `None` if the flag was absent).
fn take_value_flag(args: &mut Vec<String>, flag: &str) -> Result<Option<String>, String> {
    let Some(idx) = args.iter().position(|a| a == flag) else {
        return Ok(None);
    };
    if idx + 1 >= args.len() {
        return Err(format!("`{flag}` requires a value"));
    }
    let value = args.remove(idx + 1);
    args.remove(idx);
    Ok(Some(value))
}

/// Remove a boolean `flag` (no value) from `args`; returns whether it was present.
fn take_bool_flag(args: &mut Vec<String>, flag: &str) -> bool {
    if let Some(idx) = args.iter().position(|a| a == flag) {
        args.remove(idx);
        true
    } else {
        false
    }
}

fn require_one_arg<'a>(
    args: &'a [String],
    sub: &str,
    placeholder: &str,
) -> Result<&'a str, String> {
    match args.first() {
        Some(v) => Ok(v.as_str()),
        None => Err(format!("`cronymax ext {sub}` requires {placeholder}")),
    }
}

fn resolve_root(override_path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = override_path {
        return Ok(PathBuf::from(p));
    }
    default_registry_root().ok_or("could not resolve HOME / USERPROFILE; pass --root <dir>".into())
}

// ───────────────────────────────────────────────────────────────────────────
// `cronymax ext dev <dir> [--watch]`
//
// A self-contained dev harness: install the extension from `<dir>` into a
// throwaway registry, activate it (spawns the Node host), and stream the
// host's stdout/stderr/output-channel logs to the terminal. With `--watch`,
// reload (re-copy + re-activate) whenever a source file changes. Ctrl-C stops
// and cleans up. Uses a temp registry + temp log session so the user's real
// `~/.cronymax` is never touched.

fn run_dev(dir: &Path, watch: bool) -> Result<(), String> {
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", dir.display()));
    }
    let manifest_path = dir.join("cronymax-extension.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("could not read {}: {e}", manifest_path.display()))?;
    let manifest = Manifest::from_json(&raw).map_err(|e| e.to_string())?;
    let ext_id = manifest.id.clone();

    let node = default_bundled_node().filter(|p| p.is_file()).ok_or(
        "bundled Node 26 not found; run scripts/fetch-node26.sh or set CRONYMAX_BUNDLED_DIR",
    )?;
    let bootstrap = default_bundled_bootstrap()
        .filter(|p| p.is_file())
        .ok_or("bundled extension-host-bootstrap.js not found; set CRONYMAX_BUNDLED_DIR")?;

    let rt = tokio::runtime::Runtime::new().map_err(|e| format!("start tokio runtime: {e}"))?;
    rt.block_on(dev_loop(dir, &ext_id, watch, node, bootstrap))
        .map_err(|e| e.to_string())
}

async fn dev_loop(
    dir: &Path,
    ext_id: &str,
    watch: bool,
    node: PathBuf,
    bootstrap: PathBuf,
) -> cronymax::extensions::error::ExtensionResult<()> {
    let tmp = tempfile::TempDir::new()?;
    let registry_root = tmp.path().join("registry");
    let logs_root = tmp.path().join("logs");
    let storage_root = tmp.path().join("storage");
    std::fs::create_dir_all(&registry_root)?;
    std::fs::create_dir_all(&logs_root)?;
    std::fs::create_dir_all(&storage_root)?;

    let runtime = ExtensionRuntime::new(ExtensionRegistry::new(&registry_root));
    let log_manager = Arc::new(LogManager::new_session(&logs_root)?);
    runtime.set_log_manager(log_manager.clone());

    // Dev spawn config: bundled Node + bootstrap, throwaway storage. Mirrors
    // the composition-root builder (services.rs) but rooted under the temp dir.
    let builder: SpawnConfigBuilder = Arc::new(move |id: &str, _m: &Manifest, ext_dir: &Path| {
        let storage_dir = storage_root.join(id).join("storage");
        let global_storage_dir = storage_root.join("global").join(id);
        let _ = std::fs::create_dir_all(&storage_dir);
        let _ = std::fs::create_dir_all(&global_storage_dir);
        SpawnConfig {
            ext_id: id.to_string(),
            node_binary: node.clone(),
            node_flags: vec!["--no-warnings".into()],
            bootstrap_js: bootstrap.clone(),
            ext_dir: ext_dir.to_path_buf(),
            storage_dir,
            global_storage_dir,
            workspace_dirs: Vec::new(),
            manifest_path: ext_dir.join("cronymax-extension.json"),
            max_restarts: 0,
            ping_interval: Some(SpawnConfig::ping_interval_default()),
            stdout_log: None,
            stderr_log: None,
            host_event_sink: None,
            rss_warn_bytes: None,
        }
    });
    runtime.set_spawn_config_builder(builder);

    println!("dev: installing `{ext_id}` from {}", dir.display());
    let id = runtime.install_from(dir).await?; // installs + activates (spawns host)
    println!("dev: `{id}` active — streaming logs (Ctrl-C to stop)\n");

    // Tail the host's log files (host.log / output.log / channels/*.log) to the
    // terminal. Runs until aborted on shutdown.
    let tail = tokio::spawn(tail_logs(log_manager.ext_dir(&id)));

    if watch {
        let mut last = max_mtime(dir);
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    let now = max_mtime(dir);
                    if now > last {
                        // Debounce: wait for the tree to settle (build in progress)
                        // before reloading.
                        last = now;
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        let settled = max_mtime(dir);
                        if settled != last {
                            last = settled;
                            continue; // still changing; recheck next tick
                        }
                        println!("\ndev: change detected, reloading `{id}`…");
                        if let Err(e) = runtime.uninstall(&id).await {
                            eprintln!("dev: reload (uninstall) failed: {e}");
                        }
                        match runtime.install_from(dir).await {
                            Ok(_) => println!("dev: reloaded `{id}`\n"),
                            Err(e) => eprintln!("dev: reload (install) failed: {e}"),
                        }
                    }
                }
            }
        }
    } else {
        let _ = tokio::signal::ctrl_c().await;
    }

    println!("\ndev: stopping…");
    tail.abort();
    if let Err(e) = runtime.uninstall(&id).await {
        eprintln!("dev: cleanup failed: {e}");
    }
    Ok(())
}

/// Newest mtime across `dir`, skipping `node_modules` / `.git` (build churn /
/// huge trees). `None` if the tree can't be walked.
fn max_mtime(dir: &Path) -> Option<SystemTime> {
    let mut newest: Option<SystemTime> = None;
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            name != "node_modules" && name != ".git"
        })
        .filter_map(Result::ok)
    {
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                newest = Some(newest.map_or(modified, |n| n.max(modified)));
            }
        }
    }
    newest
}

/// Poll-tail every `*.log` under `log_dir` (host.log / output.log /
/// channels/*.log), printing newly appended bytes with a per-source tag.
async fn tail_logs(log_dir: PathBuf) {
    use std::collections::HashMap;
    use std::io::SeekFrom;
    use tokio::io::{AsyncReadExt, AsyncSeekExt};

    let mut offsets: HashMap<PathBuf, u64> = HashMap::new();
    loop {
        for (path, tag) in discover_log_files(&log_dir) {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let len = meta.len();
            let off = offsets.entry(path.clone()).or_insert(0);
            if len < *off {
                *off = 0; // truncated / rotated — re-read from the top
            }
            if len > *off {
                if let Ok(mut f) = tokio::fs::File::open(&path).await {
                    if f.seek(SeekFrom::Start(*off)).await.is_ok() {
                        let mut buf = vec![0u8; (len - *off) as usize];
                        if f.read_exact(&mut buf).await.is_ok() {
                            print_tagged(&tag, &buf);
                            *off = len;
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// `(file, tag)` pairs for the log files under an extension's log dir.
fn discover_log_files(log_dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for (name, tag) in [("output.log", "stdout"), ("host.log", "stderr")] {
        let p = log_dir.join(name);
        if p.is_file() {
            out.push((p, tag.to_string()));
        }
    }
    if let Ok(rd) = std::fs::read_dir(log_dir.join("channels")) {
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().is_some_and(|e| e == "log") {
                let stem = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push((p, stem));
            }
        }
    }
    out
}

/// Print appended bytes, prefixing each non-empty line with `[tag]`.
fn print_tagged(tag: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        if !line.is_empty() {
            println!("[{tag}] {line}");
        }
    }
}
