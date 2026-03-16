// macOS sandbox — Seatbelt SBPL generation + sandbox-exec.

use crate::sandbox::policy::SandboxPolicy;
use std::io;

/// Generate a Seatbelt SBPL profile string from a SandboxPolicy.
///
/// Uses an allow-default + deny-overrides strategy because macOS 26+
/// aborts when `subpath` / `regex` filters are used with *allow*
/// rules on `file-read*` / `file-write*`.
pub fn sbpl_from_policy(policy: &SandboxPolicy) -> String {
    let mut sbpl = String::new();
    sbpl.push_str("(version 1)\n");

    // Allow everything by default, then layer deny rules on top.
    sbpl.push_str("(allow default)\n\n");

    // ── Filesystem deny overrides ────────────────────────────────────────
    if !policy.fs.deny.is_empty() {
        sbpl.push_str("; Explicit deny overrides\n");
        for path in &policy.fs.deny {
            let expanded = SandboxPolicy::expand_path(path);
            sbpl.push_str(&format!(
                "(deny file-read* file-write* (subpath \"{}\"))\n",
                expanded.display()
            ));
        }
        sbpl.push('\n');
    }

    // ── Network deny ─────────────────────────────────────────────────────
    if policy.network.default_deny {
        sbpl.push_str("; Network: default deny\n");
        sbpl.push_str("(deny network*)\n");
    }

    sbpl
}

/// Spawn a sandboxed PTY child process using macOS sandbox-exec.
///
/// Writes the SBPL to a temp file and launches `sandbox-exec -f <path> <shell>`.
pub fn spawn_sandboxed_pty_macos(
    shell: &str,
    policy: &SandboxPolicy,
) -> io::Result<(tempfile::NamedTempFile, std::process::Child)> {
    let sbpl = sbpl_from_policy(policy);

    // Write SBPL to a temp file.
    use std::io::Write;
    let mut tmpfile = tempfile::NamedTempFile::new()?;
    tmpfile.write_all(sbpl.as_bytes())?;
    tmpfile.flush()?;

    let child = std::process::Command::new("sandbox-exec")
        .arg("-f")
        .arg(tmpfile.path())
        .arg(shell)
        .spawn()?;

    // Keep tmpfile alive — caller must hold it until child exits.
    Ok((tmpfile, child))
}
