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

use cronymax::extensions::{default_registry_root, package, ExtensionRegistry, Manifest};

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

GLOBAL OPTIONS:
    --root <dir>           Override the extensions root
                           (default: ~/.cronymax/extensions)
    -o, --output <file>    Output path for `ext package` (default: <id>-<version>.cmx)
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
