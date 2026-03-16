// Ollama command handlers — `:ollama list|pull|remove|status|install|serve` subcommands.

use crate::ai::client::ollama_manager::OllamaManager;
use crate::ai::stream::AppEvent;
use crate::app::state::AppState;
use crate::renderer::terminal::SessionId;

/// Show an info message in the active chat session (both chat + prompt editor block).
fn show_info(state: &mut AppState, text: &str) {
    if let Some(sid) = active_chat_session_id(state) {
        super::super::util::push_info_block(state, sid, text);
    }
}

/// Dispatch `:ollama <subcommand>` to the appropriate handler.
pub(crate) fn handle_ollama_command(state: &mut AppState, args: &str) {
    let args = args.trim();
    let (sub, rest) = match args.split_once(char::is_whitespace) {
        Some((s, r)) => (s, r.trim()),
        None => (args, ""),
    };

    match sub {
        "list" | "ls" => handle_ollama_list(state),
        "pull" => handle_ollama_pull(state, rest),
        "remove" | "rm" => handle_ollama_remove(state, rest),
        "use" => handle_ollama_use(state, rest),
        "status" => handle_ollama_status(state),
        "install" => handle_ollama_install(state),
        "serve" | "start" => handle_ollama_serve(state),
        "" | "help" => handle_ollama_help(state),
        other => {
            log::warn!("Unknown :ollama subcommand: {other}");
            handle_ollama_help(state);
        }
    }
}

fn handle_ollama_list(state: &mut AppState) {
    let proxy = state.proxy.clone();
    let runtime = state.runtime.clone();
    let manager = OllamaManager::default();

    // Find active chat session for displaying info messages.
    let session_id = active_chat_session_id(state);

    runtime.spawn(async move {
        match manager.list_models().await {
            Ok(models) => {
                let text = if models.is_empty() {
                    "No local models found. Use `:ollama pull <model>` to download one.".into()
                } else {
                    let mut lines = vec![format!("**Local Ollama models** ({}):", models.len())];
                    for m in &models {
                        let size_mb = m.size / (1024 * 1024);
                        lines.push(format!(
                            "- `{}` — {}MB, {} {}",
                            m.name,
                            size_mb,
                            m.parameter_size(),
                            m.quantization_level()
                        ));
                    }
                    lines.join("\n")
                };
                let _ = proxy.send_event(crate::ai::stream::AppEvent::OllamaInfoMessage {
                    session_id,
                    text,
                });
            }
            Err(e) => {
                let _ = proxy.send_event(crate::ai::stream::AppEvent::OllamaInfoMessage {
                    session_id,
                    text: format_ollama_error(&e),
                });
            }
        }
    });
}

fn handle_ollama_pull(state: &mut AppState, model: &str) {
    if model.is_empty() {
        log::warn!(":ollama pull requires a model name");
        show_info(
            state,
            "Usage: `:ollama pull <model>`\n\n\
             **Popular models:**\n\
             - `llama3.1` — Meta Llama 3.1 (8B, general purpose)\n\
             - `llama3.1:70b` — Meta Llama 3.1 (70B, high quality)\n\
             - `gemma3` — Google Gemma 3 (4B, fast)\n\
             - `gemma3:12b` — Google Gemma 3 (12B)\n\
             - `qwen3` — Alibaba Qwen 3 (8B)\n\
             - `mistral` — Mistral 7B (fast, general)\n\
             - `codellama` — Code Llama (13B, coding)\n\
             - `deepseek-r1` — DeepSeek R1 (7B, reasoning)\n\
             - `phi4` — Microsoft Phi-4 (14B, compact)\n\
             - `nomic-embed-text` — Nomic Embed (embedding model)\n\n\
             Browse all: https://ollama.com/library",
        );
        return;
    }

    let proxy = state.proxy.clone();
    let runtime = state.runtime.clone();
    let manager = OllamaManager::default();
    let model = model.to_string();
    let session_id = active_chat_session_id(state);

    // Add initial progress info message.
    if let Some(sid) = session_id {
        super::super::util::push_info_block(state, sid, &format!("⬇ Pulling `{model}`…"));
    }

    runtime.spawn(async move {
        manager.pull_model(&model, proxy, session_id).await;
    });
}

fn handle_ollama_remove(state: &mut AppState, model: &str) {
    if model.is_empty() {
        log::warn!(":ollama remove requires a model name");
        show_info(state, "Usage: `:ollama remove <model>`");
        return;
    }

    let proxy = state.proxy.clone();
    let runtime = state.runtime.clone();
    let manager = OllamaManager::default();
    let model = model.to_string();
    let session_id = active_chat_session_id(state);

    runtime.spawn(async move {
        match manager.remove_model(&model).await {
            Ok(()) => {
                let _ = proxy.send_event(crate::ai::stream::AppEvent::OllamaInfoMessage {
                    session_id,
                    text: format!("✅ Model `{model}` removed successfully."),
                });
                // Trigger model list refresh.
                let _ = proxy.send_event(crate::ai::stream::AppEvent::OllamaPullComplete {
                    model: model.clone(),
                });
            }
            Err(e) => {
                let _ = proxy.send_event(crate::ai::stream::AppEvent::OllamaInfoMessage {
                    session_id,
                    text: format!("❌ Failed to remove `{model}`: {e}"),
                });
            }
        }
    });
}

fn handle_ollama_status(state: &mut AppState) {
    let proxy = state.proxy.clone();
    let runtime = state.runtime.clone();
    let manager = OllamaManager::default();
    let session_id = active_chat_session_id(state);

    runtime.spawn(async move {
        let text = match manager.show_status().await {
            Ok(status) => status,
            Err(e) => format_ollama_error(&e),
        };
        let _ =
            proxy.send_event(crate::ai::stream::AppEvent::OllamaInfoMessage { session_id, text });
    });
}

fn handle_ollama_install(state: &mut AppState) {
    let proxy = state.proxy.clone();
    let runtime = state.runtime.clone();
    let session_id = active_chat_session_id(state);

    show_info(state, "🔍 Checking Ollama installation…");

    runtime.spawn(async move {
        // Check if already installed.
        let check = tokio::process::Command::new("which")
            .arg("ollama")
            .output()
            .await;
        if let Ok(output) = &check
            && output.status.success()
        {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                session_id,
                text: format!(
                    "✅ Ollama is already installed at `{path}`.\n\n\
                     Use `:ollama serve` to start the daemon, or `:ollama status` to check."
                ),
            });
            return;
        }

        // Not installed — run the official install script with streamed output.
        let _ = proxy.send_event(AppEvent::OllamaPullProgress {
            session_id,
            text: "⬇ Installing Ollama via official install script…".into(),
        });

        use tokio::io::{AsyncBufReadExt, BufReader};

        let mut child = match tokio::process::Command::new("sh")
            .arg("-c")
            .arg("curl -fsSL https://ollama.com/install.sh | sh 2>&1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                    session_id,
                    text: format!(
                        "❌ Failed to run install script: {e}\n\n\
                         You can install manually:\n\
                         - macOS: `brew install ollama`\n\
                         - Linux: `curl -fsSL https://ollama.com/install.sh | sh`\n\
                         - Or visit https://ollama.com/download"
                    ),
                });
                return;
            }
        };

        // Stream stdout lines as progress updates.
        let stdout = child.stdout.take();
        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout).lines();
            let mut last_update = std::time::Instant::now() - std::time::Duration::from_millis(500);
            let mut last_line = String::new();

            while let Ok(Some(line)) = reader.next_line().await {
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }
                last_line = trimmed.clone();
                // Throttle UI updates to avoid flooding.
                if last_update.elapsed() >= std::time::Duration::from_millis(500) {
                    let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                        session_id,
                        text: format!("⬇ Installing Ollama…\n`{trimmed}`"),
                    });
                    last_update = std::time::Instant::now();
                }
            }

            // Show the last line if not already shown.
            if !last_line.is_empty() {
                let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                    session_id,
                    text: format!("⬇ Installing Ollama…\n`{last_line}`"),
                });
            }
        }

        // Wait for process to complete.
        match child.wait().await {
            Ok(status) if status.success() => {
                let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                    session_id,
                    text: "✅ Ollama installed successfully!\n\n\
                           Run `:ollama serve` to start the daemon, \
                           then `:ollama pull <model>` to download a model."
                        .into(),
                });
            }
            Ok(status) => {
                let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                    session_id,
                    text: format!(
                        "❌ Install script failed (exit {status}).\n\n\
                         You can install manually:\n\
                         - macOS: `brew install ollama`\n\
                         - Linux: `curl -fsSL https://ollama.com/install.sh | sh`\n\
                         - Or visit https://ollama.com/download"
                    ),
                });
            }
            Err(e) => {
                let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                    session_id,
                    text: format!("❌ Failed waiting for install: {e}"),
                });
            }
        }
    });
}

fn handle_ollama_serve(state: &mut AppState) {
    let proxy = state.proxy.clone();
    let runtime = state.runtime.clone();
    let session_id = active_chat_session_id(state);

    show_info(state, "🚀 Starting Ollama daemon…");

    runtime.spawn(async move {
        // Check if already running.
        let manager = OllamaManager::default();
        if let Ok(ver) = manager.detect().await {
            let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                session_id,
                text: format!("✅ Ollama is already running (v{ver})."),
            });
            return;
        }

        // Check if binary exists.
        let which = tokio::process::Command::new("which")
            .arg("ollama")
            .output()
            .await;
        if !which.is_ok_and(|o| o.status.success()) {
            let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                session_id,
                text: "❌ Ollama binary not found. Run `:ollama install` first.".into(),
            });
            return;
        }

        // Start `ollama serve` as a detached background process.
        match tokio::process::Command::new("ollama")
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_child) => {
                // Wait a moment for the server to start.
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                match manager.detect().await {
                    Ok(ver) => {
                        let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                            session_id,
                            text: format!("✅ Ollama daemon started (v{ver}).\n\nUse `:ollama pull <model>` to download a model."),
                        });
                    }
                    Err(_) => {
                        let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                            session_id,
                            text: "⏳ Ollama daemon is starting up… Try `:ollama status` in a few seconds.".into(),
                        });
                    }
                }
            }
            Err(e) => {
                let _ = proxy.send_event(AppEvent::OllamaPullProgress {
                    session_id,
                    text: format!("❌ Failed to start Ollama: {e}"),
                });
            }
        }
    });
}

fn handle_ollama_use(state: &mut AppState, model: &str) {
    if model.is_empty() {
        show_info(
            state,
            "Usage: `:ollama use <model>`\n\nSwitch the active LLM to a local Ollama model.\nRun `:ollama list` to see available models.",
        );
        return;
    }

    let model_name = model.to_string();
    let base_url = "http://localhost:11434/v1";

    // Reconfigure the LLM client to use this Ollama model.
    let new_config = crate::ai::client::LlmConfig {
        provider: crate::ai::client::LlmProvider::Ollama,
        model: model_name.clone(),
        api_base: Some(base_url.to_string()),
        api_key_env: None,
        max_context_tokens: 128_000,
        reserve_tokens: 4_096,
        system_prompt: state
            .llm_client
            .as_ref()
            .and_then(|c| c.system_prompt().map(String::from)),
        auto_compact: state
            .llm_client
            .as_ref()
            .map(|c| c.config().auto_compact)
            .unwrap_or(true),
        secret_storage: crate::services::secret::SecretStorage::Auto,
    };

    let mut new_client = crate::ai::client::LlmClient::new(&new_config, &state.secret_store);
    // Carry over configured providers for the model picker.
    if let Some(ref old_client) = state.llm_client {
        new_client.set_configured_providers(old_client.configured_providers().to_vec());
    }
    state.llm_client = Some(new_client);

    // Update prompt editors to show the new active model.
    if let Some(ref client) = state.llm_client {
        let seed = client.current_model_item();
        for pe in state.prompt_editors.values_mut() {
            // Find index of matching model, or prepend.
            if let Some(idx) = pe
                .model_items
                .iter()
                .position(|m| m.provider == seed.provider && m.model == seed.model)
            {
                pe.selected_model_idx = idx;
            } else {
                pe.model_items.insert(0, seed.clone());
                pe.selected_model_idx = 0;
            }
        }
    }

    show_info(
        state,
        &format!(
            "✅ Switched to **Ollama / {}**\n\nAll new chat messages will use this model.",
            model_name
        ),
    );
    log::info!("Switched LLM to Ollama model: {}", model_name);
}

fn handle_ollama_help(state: &mut AppState) {
    let help = "\
**:ollama** — Manage local Ollama models\n\
\n\
- `:ollama install` — Install Ollama (official install script)\n\
- `:ollama serve` — Start the Ollama daemon\n\
- `:ollama list` — List locally available models\n\
- `:ollama pull <model>` — Download a model (e.g., `llama3`, `mistral`)\n\
- `:ollama use <model>` — Switch to an Ollama model for chat\n\
- `:ollama remove <model>` — Delete a local model\n\
- `:ollama status` — Show Ollama daemon status";

    show_info(state, help);
}

/// Get the active chat session's terminal SessionId for info message routing.
fn active_chat_session_id(state: &AppState) -> Option<SessionId> {
    crate::ui::tiles::active_terminal_session(&state.tile_tree)
}

/// Format an Ollama error into a user-friendly message with installation instructions.
fn format_ollama_error(err: &anyhow::Error) -> String {
    let msg = err.to_string();
    if msg.contains("connection refused")
        || msg.contains("Connection refused")
        || msg.contains("connect error")
    {
        "❌ **Ollama is not reachable.**\n\n\
         Ollama may not be installed or not running.\n\
         - Install: `:ollama install`\n\
         - Start: `:ollama serve`"
            .to_string()
    } else {
        format!("❌ Ollama error: {msg}")
    }
}
