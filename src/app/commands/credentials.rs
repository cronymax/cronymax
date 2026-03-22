// Credential command handlers — `:credentials store|list|remove` subcommands.

use crate::ai::skills::credentials;
use crate::app::state::AppState;

/// Dispatch `:credentials <subcommand>` to the appropriate handler.
pub(crate) fn handle_credentials_command(state: &mut AppState, args: &str) {
    let args = args.trim();
    let (sub, rest) = match args.split_once(char::is_whitespace) {
        Some((s, r)) => (s, r.trim()),
        None => (args, ""),
    };

    match sub {
        "store" => handle_credentials_store(state, rest),
        "list" | "ls" => handle_credentials_list(state),
        "remove" | "rm" => handle_credentials_remove(state, rest),
        "" | "help" => handle_credentials_help(state),
        other => {
            log::warn!("Unknown :credentials subcommand: {other}");
            handle_credentials_help(state);
        }
    }
}

/// Parse `--service <s> --key <k>` flag pairs from args.
fn parse_flags(args: &str) -> (Option<String>, Option<String>) {
    let parts: Vec<&str> = args.split_whitespace().collect();
    let mut service = None;
    let mut key = None;
    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "--service" | "-s" => {
                if i + 1 < parts.len() {
                    service = Some(parts[i + 1].to_string());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "--key" | "-k" => {
                if i + 1 < parts.len() {
                    key = Some(parts[i + 1].to_string());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    (service, key)
}

fn handle_credentials_store(state: &mut AppState, args: &str) {
    let (service, key) = parse_flags(args);
    let (service, key) = match (service, key) {
        (Some(s), Some(k)) => (s, k),
        _ => {
            show_info(
                state,
                "Usage: `:credentials store --service <name> --key <key>`\n\
                              The secret value will be read from the next prompt input.",
            );
            return;
        }
    };

    // For security, look for `--value` in args (for scripted use).
    // In interactive mode the user would be prompted, but since we don't have
    // a modal prompt, accept it inline for now.
    let parts: Vec<&str> = args.split_whitespace().collect();
    let mut value = None;
    for i in 0..parts.len() {
        if (parts[i] == "--value" || parts[i] == "-v") && i + 1 < parts.len() {
            value = Some(parts[i + 1].to_string());
        }
    }

    let Some(val) = value else {
        show_info(
            state,
            "Please provide `--value <secret>` to store the credential.\n\
                          Example: `:credentials store --service openai --key api_key --value sk-...`",
        );
        return;
    };

    let secret_store = state.secret_store.clone();
    match credentials::credential_store(&secret_store, &service, &key, &val) {
        Ok(()) => show_info(state, &format!("✅ Credential `{service}:{key}` stored.")),
        Err(e) => show_info(state, &format!("❌ Failed to store credential: {e}")),
    }
}

fn handle_credentials_list(state: &mut AppState) {
    let entries = credentials::credential_list();
    if entries.is_empty() {
        show_info(
            state,
            "No credentials stored. Use `:credentials store --service <name> --key <key> --value <secret>` to add one.",
        );
        return;
    }
    let mut lines = vec![format!("**Stored credentials** ({}):", entries.len())];
    for e in &entries {
        lines.push(format!(
            "- `{}:{}` (created {})",
            e.service, e.key, e.created_at
        ));
    }
    show_info(state, &lines.join("\n"));
}

fn handle_credentials_remove(state: &mut AppState, args: &str) {
    let (service, key) = parse_flags(args);
    let (service, key) = match (service, key) {
        (Some(s), Some(k)) => (s, k),
        _ => {
            show_info(
                state,
                "Usage: `:credentials remove --service <name> --key <key>`",
            );
            return;
        }
    };

    let secret_store = state.secret_store.clone();
    match credentials::credential_remove(&secret_store, &service, &key) {
        Ok(()) => show_info(state, &format!("✅ Credential `{service}:{key}` removed.")),
        Err(e) => show_info(state, &format!("❌ Failed to remove credential: {e}")),
    }
}

fn handle_credentials_help(state: &mut AppState) {
    let help = "\
**:credentials** — Manage stored credentials (OS keychain)\n\
\n\
- `:credentials store --service <name> --key <key> --value <secret>`\n\
- `:credentials list` — List stored credential entries (never shows values)\n\
- `:credentials remove --service <name> --key <key>` — Remove a credential\n\
\n\
Alias: `:creds`";
    show_info(state, help);
}

/// Show an info message in the active chat session (both chat + prompt editor block).
fn show_info(state: &mut AppState, text: &str) {
    if let Some(sid) = crate::ui::tiles::active_terminal_session(&state.ui.tile_tree) {
        super::super::util::push_info_block(state, sid, text);
    }
}
