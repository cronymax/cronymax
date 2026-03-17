// LLM client — async-openai provider dispatch with SSE streaming.
#![allow(dead_code)]

use std::sync::Arc;

use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{
        ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage, ChatCompletionTool,
        ChatCompletionToolType, CreateChatCompletionRequestArgs, FunctionObject,
    },
};
use futures::StreamExt;
use tokio::task::JoinHandle;
use winit::event_loop::EventLoopProxy;

use crate::ai::context::{ChatMessage, MessageRole};
use crate::ai::stream::{AppEvent, TokenUsage, ToolCallInfo};

/// Parameters for an LLM streaming request.
struct StreamRequest {
    client: Client<OpenAIConfig>,
    model: String,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<serde_json::Value>>,
    proxy: EventLoopProxy<AppEvent>,
    session_id: u32,
}

/// Parameters for a raw Copilot SSE streaming request (bypasses async-openai
/// deserialization which requires fields the Copilot API may omit).
struct CopilotStreamRequest {
    session_token: String,
    api_base: String,
    model: String,
    messages: Vec<ChatMessage>,
    tools: Option<Vec<serde_json::Value>>,
    proxy: EventLoopProxy<AppEvent>,
    session_id: u32,
}

/// LLM provider backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LlmProvider {
    OpenAI,
    Ollama,
    /// GitHub Copilot — uses the OpenAI-compatible endpoint at api.githubcopilot.com.
    Copilot,
    /// Anthropic Claude (OpenAI-compatible proxy or direct).
    Anthropic,
    Custom,
}

/// LLM configuration per profile.
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    pub model: String,
    pub api_base: Option<String>,
    pub api_key_env: Option<String>,
    pub max_context_tokens: usize,
    pub reserve_tokens: usize,
    pub system_prompt: Option<String>,
    pub auto_compact: bool,
    pub secret_storage: crate::services::secret::SecretStorage,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: LlmProvider::OpenAI,
            model: "gpt-4o".into(),
            api_base: None,
            api_key_env: Some("OPENAI_API_KEY".into()),
            max_context_tokens: 128_000,
            reserve_tokens: 4096,
            system_prompt: Some("You are a helpful terminal assistant.".into()),
            auto_compact: false,
            secret_storage: crate::services::secret::SecretStorage::Auto,
        }
    }
}

/// A specific model selection (provider + model pair).
#[derive(Debug, Clone)]
pub struct ModelSelection {
    pub provider: LlmProvider,
    pub model: String,
    pub display_label: String,
}

/// An entry in the model catalogue returned by `available_models()`.
#[derive(Debug, Clone)]
pub struct ModelListItem {
    pub provider: LlmProvider,
    pub model: String,
    pub display_label: String,
    pub available: bool,
}

impl ModelListItem {
    /// Human-readable provider name string.
    pub fn provider_name(&self) -> &'static str {
        match self.provider {
            LlmProvider::OpenAI => "OpenAI",
            LlmProvider::Anthropic => "Anthropic",
            LlmProvider::Copilot => "GitHub Copilot",
            LlmProvider::Ollama => "Ollama (local)",
            LlmProvider::Custom => "Custom",
        }
    }
}

/// Build an `async-openai` client with Copilot-specific headers.
fn build_copilot_client(token: &str, api_base: &str) -> Client<OpenAIConfig> {
    let mut default_headers = reqwest::header::HeaderMap::new();
    default_headers.insert("Copilot-Integration-Id", "vscode-chat".parse().unwrap());
    default_headers.insert("Editor-Version", "vscode/1.100.0".parse().unwrap());
    default_headers.insert(
        "Editor-Plugin-Version",
        "copilot-chat/0.26.3".parse().unwrap(),
    );

    let http_client = reqwest::Client::builder()
        .default_headers(default_headers)
        .build()
        .unwrap_or_default();

    let oai_config = OpenAIConfig::new()
        .with_api_base(api_base)
        .with_api_key(token);

    Client::with_config(oai_config).with_http_client(http_client)
}

/// Copilot headers used for both model listing and chat requests.
const COPILOT_HEADERS: &[(&str, &str)] = &[
    ("Copilot-Integration-Id", "vscode-chat"),
    ("Editor-Version", "vscode/1.100.0"),
    ("Editor-Plugin-Version", "copilot-chat/0.26.3"),
    ("Openai-Intent", "conversation-panel"),
];

/// Exchange a Copilot OAuth token for a short-lived API session token.
///
/// GET https://api.github.com/copilot_internal/v2/token
///   Authorization: token <copilot_oauth_token>
///   + Copilot headers
///
/// Returns `(session_token, api_base)` on success.
async fn exchange_copilot_token(oauth_token: &str) -> anyhow::Result<(String, String)> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let mut req = client
        .get("https://api.github.com/copilot_internal/v2/token")
        .header("Authorization", format!("token {}", oauth_token))
        .header("Accept", "application/json")
        .header("User-Agent", "cronymax/0.1.0");
    for &(k, v) in COPILOT_HEADERS {
        req = req.header(k, v);
    }

    let resp = req.send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Copilot token exchange failed (HTTP {status}): {body}");
    }

    let json: serde_json::Value = resp.json().await?;
    let token = json
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("Copilot token response missing 'token' field"))?
        .to_string();
    let api_base = json
        .pointer("/endpoints/api")
        .and_then(|v| v.as_str())
        .unwrap_or("https://api.githubcopilot.com")
        .to_string();

    log::info!("Copilot: exchanged OAuth token for session token (endpoint: {api_base})");
    Ok((token, api_base))
}

/// Try to resolve Copilot auth from cached token only (non-interactive).
///
/// Returns `Some((oauth_token, session_token, api_base))` if a cached token
/// exists and can be exchanged successfully.  Returns `None` otherwise —
/// the caller should prompt the user to run `:copilot-login`.
async fn try_cached_copilot_auth() -> Option<(String, String, String)> {
    if let Some(cached) = copilot_auth::load_cached_token() {
        match exchange_copilot_token(&cached).await {
            Ok((session, api)) => return Some((cached, session, api)),
            Err(e) => log::warn!("Cached Copilot token expired or invalid: {e}"),
        }
    }
    None
}

/// LLM client that handles streaming chat completions.
pub struct LlmClient {
    config: LlmConfig,
    openai_client: Option<Client<OpenAIConfig>>,
    /// Additional provider endpoints from config.toml to include in model fetching.
    configured_providers: Vec<crate::config::ProviderConfig>,
    /// Secret store for resolving API keys from the system keychain.
    secret_store: crate::services::secret::SecretStore,
    /// Raw GitHub token for Copilot token exchange (re-exchange on expiry).
    copilot_github_token: Option<String>,
}

impl LlmClient {
    /// Create a new LLM client from the given configuration.
    pub fn new(config: &LlmConfig, secret_store: &crate::services::secret::SecretStore) -> Self {
        let mut copilot_github_token = None;
        let openai_client = match config.provider {
            LlmProvider::OpenAI | LlmProvider::Custom => {
                let mut oai_config = OpenAIConfig::new();

                // Set API key from keychain / env var.
                let key_name = crate::services::secret::provider_api_key(
                    if config.provider == LlmProvider::Custom {
                        "custom"
                    } else {
                        "openai"
                    },
                );
                if let Ok(Some(key)) = secret_store.resolve(
                    &key_name,
                    config.api_key_env.as_deref(),
                    &config.secret_storage,
                ) {
                    oai_config = oai_config.with_api_key(key);
                }

                // Override base URL if provided.
                if let Some(base) = &config.api_base
                    && !base.is_empty()
                {
                    oai_config = oai_config.with_api_base(base);
                }

                Some(Client::with_config(oai_config))
            }
            LlmProvider::Ollama => {
                // For Ollama, we use the OpenAI-compatible endpoint.
                let base = config
                    .api_base
                    .as_deref()
                    .unwrap_or("http://localhost:11434/v1");
                let oai_config = OpenAIConfig::new()
                    .with_api_base(base)
                    .with_api_key("ollama"); // Ollama doesn't need a real key.
                Some(Client::with_config(oai_config))
            }
            LlmProvider::Copilot => {
                // GitHub Copilot — try cached OAuth token only (non-interactive).
                // If no cached token exists, the user must run `:copilot-login`.
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                match rt {
                    Ok(rt) => match rt.block_on(try_cached_copilot_auth()) {
                        Some((oauth_token, session_token, api_base)) => {
                            copilot_github_token = Some(oauth_token);
                            Some(build_copilot_client(&session_token, &api_base))
                        }
                        None => {
                            log::info!(
                                "Copilot: no cached OAuth token. \
                                 Run :copilot-login to authenticate."
                            );
                            None
                        }
                    },
                    Err(e) => {
                        log::warn!("Failed to create tokio runtime for Copilot auth: {e}");
                        None
                    }
                }
            }
            LlmProvider::Anthropic => {
                // Anthropic Claude — use OpenAI-compatible proxy or direct endpoint.
                let base = config
                    .api_base
                    .as_deref()
                    .unwrap_or("https://api.anthropic.com/v1");
                let mut oai_config = OpenAIConfig::new().with_api_base(base);

                let env_var = config.api_key_env.as_deref().unwrap_or("ANTHROPIC_API_KEY");
                let key_name = crate::services::secret::provider_api_key("anthropic");
                if let Ok(Some(key)) =
                    secret_store.resolve(&key_name, Some(env_var), &config.secret_storage)
                {
                    oai_config = oai_config.with_api_key(key);
                }

                Some(Client::with_config(oai_config))
            }
        };

        Self {
            config: config.clone(),
            openai_client,
            configured_providers: Vec::new(),
            secret_store: secret_store.clone(),
            copilot_github_token,
        }
    }

    /// Set the configured provider list from config.toml `[ai.providers]`.
    pub fn set_configured_providers(&mut self, providers: Vec<crate::config::ProviderConfig>) {
        self.configured_providers = providers;
    }

    /// Get the configured provider list.
    pub fn configured_providers(&self) -> &[crate::config::ProviderConfig] {
        &self.configured_providers
    }

    /// Return the configured model name.
    pub fn model_name(&self) -> &str {
        &self.config.model
    }

    /// Whether the Copilot provider is authenticated (has an OAuth token).
    pub fn copilot_authenticated(&self) -> bool {
        self.copilot_github_token.is_some() && self.openai_client.is_some()
    }

    /// Start the Copilot OAuth device-code login flow.
    ///
    /// 1. Requests a device code from GitHub.
    /// 2. Sends `AppEvent::CopilotDeviceCode` so the UI can open the
    ///    verification URL in the internal webview.
    /// 3. Polls for the access token in the background.
    /// 4. On success, sends `AppEvent::CopilotLoginComplete`.
    pub fn start_copilot_login(
        &self,
        proxy: EventLoopProxy<AppEvent>,
        runtime: &Arc<tokio::runtime::Runtime>,
    ) {
        runtime.spawn(async move {
            match copilot_auth::request_device_code().await {
                Ok(dc) => {
                    log::info!(
                        "Copilot login: enter code {} at {}",
                        dc.user_code,
                        dc.verification_uri
                    );
                    // Tell the UI to show the verification page.
                    let _ = proxy.send_event(AppEvent::CopilotDeviceCode {
                        user_code: dc.user_code,
                        verification_uri: dc.verification_uri.clone(),
                    });

                    // Poll for the token in the background.
                    match copilot_auth::poll_for_token(&dc.device_code, dc.interval).await {
                        Ok(oauth_token) => {
                            if let Err(e) = copilot_auth::save_cached_token(&oauth_token) {
                                log::warn!("Failed to cache Copilot token: {e}");
                            }
                            match exchange_copilot_token(&oauth_token).await {
                                Ok((session_token, api_base)) => {
                                    let _ = proxy.send_event(AppEvent::CopilotLoginComplete {
                                        oauth_token,
                                        session_token,
                                        api_base,
                                    });
                                }
                                Err(e) => {
                                    log::error!("Copilot: token exchange after login failed: {e}");
                                    let _ = proxy.send_event(AppEvent::CopilotLoginFailed {
                                        error: e.to_string(),
                                    });
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Copilot: device login failed: {e}");
                            let _ = proxy.send_event(AppEvent::CopilotLoginFailed {
                                error: e.to_string(),
                            });
                        }
                    }
                }
                Err(e) => {
                    log::error!("Copilot: device code request failed: {e}");
                    let _ = proxy.send_event(AppEvent::CopilotLoginFailed {
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    /// Called when the async device flow completes successfully.
    /// Updates the client with the new OAuth + session tokens.
    pub fn complete_copilot_login(
        &mut self,
        oauth_token: String,
        session_token: &str,
        api_base: &str,
    ) {
        self.copilot_github_token = Some(oauth_token);
        self.openai_client = Some(build_copilot_client(session_token, api_base));
        log::info!("Copilot: login complete, client updated");
    }

    /// Stream a chat completion. Spawns a tokio task that sends AppEvents via the proxy.
    /// Returns the JoinHandle for cancellation.
    pub fn stream_chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Option<Vec<serde_json::Value>>,
        proxy: EventLoopProxy<AppEvent>,
        session_id: u32,
        runtime: &Arc<tokio::runtime::Runtime>,
    ) -> JoinHandle<()> {
        let model = self.config.model.clone();
        let provider = self.config.provider.clone();
        let openai_client = self.openai_client.clone();
        let copilot_gh_token = self.copilot_github_token.clone();

        runtime.spawn(async move {
            let result = {
                if provider == LlmProvider::Copilot {
                    // Copilot uses raw SSE to avoid async-openai deserialization
                    // issues (Copilot API omits the `model` field in stream chunks).
                    let (session_token, api_base) = if let Some(ref gh_token) = copilot_gh_token {
                        match exchange_copilot_token(gh_token).await {
                            Ok(pair) => pair,
                            Err(e) => {
                                let _ = proxy.send_event(AppEvent::LlmError {
                                    session_id,
                                    error: format!("Copilot token exchange failed: {e}"),
                                });
                                return;
                            }
                        }
                    } else {
                        let _ = proxy.send_event(AppEvent::LlmError {
                            session_id,
                            error: "Copilot not authenticated. Run :copilot-login".into(),
                        });
                        return;
                    };
                    Self::do_stream_copilot(CopilotStreamRequest {
                        session_token,
                        api_base,
                        model,
                        messages,
                        tools,
                        proxy: proxy.clone(),
                        session_id,
                    })
                    .await
                } else {
                    // Non-Copilot providers: use async-openai typed stream.
                    match openai_client {
                        Some(client) => {
                            Self::do_stream(StreamRequest {
                                client,
                                model,
                                messages,
                                tools,
                                proxy: proxy.clone(),
                                session_id,
                            })
                            .await
                        }
                        None => Err(anyhow::anyhow!(
                            "LLM client not initialized. Check that the required API key \
                             environment variable is set for the {:?} provider.",
                            provider
                        )),
                    }
                }
            };
            if let Err(e) = result {
                let _ = proxy.send_event(AppEvent::LlmError {
                    session_id,
                    error: e.to_string(),
                });
            }
        })
    }

    async fn do_stream(req: StreamRequest) -> anyhow::Result<()> {
        let StreamRequest {
            client,
            model,
            messages,
            tools,
            proxy,
            session_id,
        } = req;
        // Convert ChatMessages to OpenAI format.
        let oai_messages: Vec<ChatCompletionRequestMessage> = messages
            .iter()
            .map(|m| match m.role {
                MessageRole::System => {
                    ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                        content:
                            async_openai::types::ChatCompletionRequestSystemMessageContent::Text(
                                m.content.clone(),
                            ),
                        name: None,
                    })
                }
                MessageRole::User => {
                    ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                        content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                            m.content.clone(),
                        ),
                        name: None,
                    })
                }
                MessageRole::Assistant => {
                    // Convert stored ToolCallInfo → ChatCompletionMessageToolCall.
                    let oai_tc: Option<Vec<async_openai::types::ChatCompletionMessageToolCall>> =
                        if m.tool_calls.is_empty() {
                            None
                        } else {
                            Some(
                                m.tool_calls
                                    .iter()
                                    .map(|tc| async_openai::types::ChatCompletionMessageToolCall {
                                        id: tc.id.clone(),
                                        r#type: ChatCompletionToolType::Function,
                                        function: async_openai::types::FunctionCall {
                                            name: tc.function_name.clone(),
                                            arguments: tc.arguments.clone(),
                                        },
                                    })
                                    .collect(),
                            )
                        };
                    // If content is empty and tool calls present, send null content.
                    let content = if m.content.is_empty() && oai_tc.is_some() {
                        None
                    } else {
                        Some(
                            async_openai::types::ChatCompletionRequestAssistantMessageContent::Text(
                                m.content.clone(),
                            ),
                        )
                    };
                    ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
                        content,
                        name: None,
                        tool_calls: oai_tc,
                        refusal: None,
                        audio: None,
                        #[allow(deprecated)]
                        function_call: None,
                    })
                }
                MessageRole::Tool => {
                    // Tool messages use the OpenAI tool message format.
                    ChatCompletionRequestMessage::Tool(
                        async_openai::types::ChatCompletionRequestToolMessage {
                            content:
                                async_openai::types::ChatCompletionRequestToolMessageContent::Text(
                                    m.content.clone(),
                                ),
                            tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
                        },
                    )
                }
                // Info messages should be filtered out before reaching the API;
                // treat as user message as a defensive fallback.
                MessageRole::Info => {
                    ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                        content: async_openai::types::ChatCompletionRequestUserMessageContent::Text(
                            m.content.clone(),
                        ),
                        name: None,
                    })
                }
            })
            .collect();

        // Convert JSON tool definitions to typed ChatCompletionTool structs.
        let oai_tools: Option<Vec<ChatCompletionTool>> = tools.map(|tool_defs| {
            tool_defs
                .into_iter()
                .filter_map(|t| {
                    let func = t.get("function")?;
                    Some(ChatCompletionTool {
                        r#type: ChatCompletionToolType::Function,
                        function: FunctionObject {
                            name: func.get("name")?.as_str()?.to_string(),
                            description: func
                                .get("description")
                                .and_then(|d| d.as_str())
                                .map(|s| s.to_string()),
                            parameters: func.get("parameters").cloned(),
                            strict: None,
                        },
                    })
                })
                .collect()
        });

        let mut builder = CreateChatCompletionRequestArgs::default();
        builder.model(&model).messages(oai_messages).stream(true);
        if let Some(tools) = oai_tools
            && !tools.is_empty()
        {
            builder.tools(tools);
        }
        let request = builder.build()?;

        // Retry with exponential backoff on transient errors (429 rate limit, 5xx server errors).
        // Backoff schedule: 10s, 20s, 30s, 60s — covers typical 60-second rate-limit windows.
        const MAX_RETRIES: u32 = 4;
        const BACKOFF_SECS: [u64; 4] = [10, 20, 30, 60];
        let mut stream = {
            let mut last_err = None;
            let mut attempt_stream = None;
            for attempt in 0..=MAX_RETRIES {
                match client.chat().create_stream(request.clone()).await {
                    Ok(s) => {
                        attempt_stream = Some(s);
                        break;
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        let is_retryable = err_str.contains("429")
                            || err_str.contains("rate")
                            || err_str.contains("Too Many Requests")
                            || err_str.contains("500")
                            || err_str.contains("502")
                            || err_str.contains("503")
                            || err_str.contains("529");
                        if is_retryable && attempt < MAX_RETRIES {
                            let delay =
                                std::time::Duration::from_secs(BACKOFF_SECS[attempt as usize]);
                            log::warn!(
                                "LLM stream attempt {}/{} failed ({}), retrying in {:?}...",
                                attempt + 1,
                                MAX_RETRIES + 1,
                                err_str,
                                delay
                            );
                            let _ = proxy.send_event(AppEvent::LlmToken {
                                session_id,
                                token: format!(
                                    "\n[Retrying in {}s due to rate limit...]\n",
                                    delay.as_secs()
                                ),
                            });
                            tokio::time::sleep(delay).await;
                            last_err = Some(e);
                        } else {
                            return Err(e.into());
                        }
                    }
                }
            }
            match attempt_stream {
                Some(s) => s,
                None => {
                    return Err(last_err.map(|e| anyhow::anyhow!(e)).unwrap_or_else(|| {
                        anyhow::anyhow!("Failed to create stream after retries")
                    }));
                }
            }
        };

        let mut full_response = String::new();
        let mut tool_calls: Vec<ToolCallInfo> = Vec::new();
        let mut usage: Option<TokenUsage> = None;

        // Mid-stream retry budget for 429/5xx that arrive as stream items
        // rather than from create_stream (some providers return 200 OK then
        // send the error inside the SSE body).
        // Backoff schedule: 10s, 20s, 30s, 60s.
        const MID_STREAM_RETRIES: u32 = 4;
        const MID_BACKOFF_SECS: [u64; 4] = [10, 20, 30, 60];
        let mut mid_retries_left = MID_STREAM_RETRIES;

        'stream_loop: loop {
            while let Some(result) = stream.next().await {
                match result {
                    Ok(response) => {
                        for choice in &response.choices {
                            // Accumulate content tokens.
                            if let Some(ref content) = choice.delta.content {
                                full_response.push_str(content);
                                let _ = proxy.send_event(AppEvent::LlmToken {
                                    session_id,
                                    token: content.clone(),
                                });
                            }

                            // Accumulate tool calls.
                            if let Some(ref tc) = choice.delta.tool_calls {
                                for call in tc {
                                    let idx = call.index as usize;
                                    while tool_calls.len() <= idx {
                                        tool_calls.push(ToolCallInfo {
                                            id: String::new(),
                                            function_name: String::new(),
                                            arguments: String::new(),
                                        });
                                    }
                                    if let Some(ref id) = call.id {
                                        tool_calls[idx].id = id.clone();
                                    }
                                    if let Some(ref func) = call.function {
                                        if let Some(ref name) = func.name {
                                            tool_calls[idx].function_name = name.clone();
                                        }
                                        if let Some(ref args) = func.arguments {
                                            tool_calls[idx].arguments.push_str(args);
                                        }
                                    }
                                }
                            }
                        }

                        // Capture usage if reported.
                        if let Some(ref u) = response.usage {
                            usage = Some(TokenUsage {
                                prompt_tokens: u.prompt_tokens,
                                completion_tokens: u.completion_tokens,
                                total_tokens: u.total_tokens,
                            });
                        }
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        let is_retryable = err_str.contains("429")
                            || err_str.contains("rate")
                            || err_str.contains("Too Many Requests")
                            || err_str.contains("529");
                        if is_retryable && mid_retries_left > 0 && full_response.is_empty() {
                            mid_retries_left -= 1;
                            let attempt_idx = (MID_STREAM_RETRIES - mid_retries_left - 1) as usize;
                            let delay =
                                std::time::Duration::from_secs(MID_BACKOFF_SECS[attempt_idx]);
                            log::warn!(
                                "Transient stream error ({err_str}), retrying in {delay:?} \
                                 ({mid_retries_left} retries left)…"
                            );
                            let _ = proxy.send_event(AppEvent::LlmToken {
                                session_id,
                                token: format!(
                                    "\n[Rate limited — retrying in {}s…]\n",
                                    delay.as_secs()
                                ),
                            });
                            tokio::time::sleep(delay).await;
                            // Re-create the stream and retry.
                            match client.chat().create_stream(request.clone()).await {
                                Ok(new_stream) => {
                                    stream = new_stream;
                                    continue 'stream_loop;
                                }
                                Err(retry_err) => {
                                    let _ = proxy.send_event(AppEvent::LlmError {
                                        session_id,
                                        error: format!("Stream error: {retry_err}"),
                                    });
                                    return Ok(());
                                }
                            }
                        }
                        if is_retryable {
                            log::warn!("Transient stream error ({err_str}), reporting to user");
                        }
                        let _ = proxy.send_event(AppEvent::LlmError {
                            session_id,
                            error: format!("Stream error: {e}"),
                        });
                        return Ok(());
                    }
                }
            }
            break; // Stream ended normally.
        }

        // Stream completed — send LlmDone.
        let _ = proxy.send_event(AppEvent::LlmDone {
            session_id,
            full_response,
            usage,
            tool_calls,
        });

        Ok(())
    }

    /// Copilot-specific SSE streaming that tolerates missing `model` field.
    ///
    /// The GitHub Copilot API returns SSE chunks that may omit the `model`
    /// field, which causes `async-openai`'s strict deserialization to fail.
    /// This method does raw reqwest + manual SSE line parsing with a lenient
    /// response struct.
    async fn do_stream_copilot(req: CopilotStreamRequest) -> anyhow::Result<()> {
        let CopilotStreamRequest {
            session_token,
            api_base,
            model,
            messages,
            tools,
            proxy,
            session_id,
        } = req;

        // Build the JSON request body (OpenAI-compatible format).
        let oai_messages: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    MessageRole::System => "system",
                    MessageRole::User => "user",
                    MessageRole::Assistant => "assistant",
                    MessageRole::Tool => "tool",
                    MessageRole::Info => "user", // should be filtered, defensive fallback
                };
                let mut obj = serde_json::json!({
                    "role": role,
                    "content": m.content,
                });
                if m.role == MessageRole::Tool
                    && let Some(ref id) = m.tool_call_id
                {
                    obj["tool_call_id"] = serde_json::json!(id);
                }
                if m.role == MessageRole::Assistant && !m.tool_calls.is_empty() {
                    let tcs: Vec<serde_json::Value> = m
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.function_name,
                                    "arguments": tc.arguments,
                                }
                            })
                        })
                        .collect();
                    obj["tool_calls"] = serde_json::json!(tcs);
                    if m.content.is_empty() {
                        obj["content"] = serde_json::Value::Null;
                    }
                }
                obj
            })
            .collect();

        let mut body = serde_json::json!({
            "model": model,
            "messages": oai_messages,
            "stream": true,
        });
        if let Some(tool_defs) = tools
            && !tool_defs.is_empty()
        {
            body["tools"] = serde_json::json!(tool_defs);
        }

        let url = format!("{}/chat/completions", api_base.trim_end_matches('/'));
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(300))
            .build()?;

        // Retry loop for the initial POST.
        const MAX_RETRIES: u32 = 4;
        const BACKOFF_SECS: [u64; 4] = [10, 20, 30, 60];
        let mut resp = None;
        for attempt in 0..=MAX_RETRIES {
            let r = http
                .post(&url)
                .header("Authorization", format!("Bearer {}", session_token))
                .header("Content-Type", "application/json")
                .header("Accept", "text/event-stream")
                .header("User-Agent", "cronymax/0.1.0")
                .header("Copilot-Integration-Id", "vscode-chat")
                .header("Editor-Version", "vscode/1.100.0")
                .header("Editor-Plugin-Version", "copilot-chat/0.26.3")
                .header("Openai-Intent", "conversation-panel")
                .body(body.to_string())
                .send()
                .await;
            match r {
                Ok(r) if r.status().is_success() => {
                    resp = Some(r);
                    break;
                }
                Ok(r) => {
                    let status = r.status();
                    let is_retryable = status.as_u16() == 429 || status.is_server_error();
                    if is_retryable && attempt < MAX_RETRIES {
                        let delay = std::time::Duration::from_secs(BACKOFF_SECS[attempt as usize]);
                        log::warn!(
                            "Copilot stream attempt {}/{} got {status}, retrying in {delay:?}…",
                            attempt + 1,
                            MAX_RETRIES + 1
                        );
                        let _ = proxy.send_event(AppEvent::LlmToken {
                            session_id,
                            token: format!(
                                "\n[Retrying in {}s due to rate limit...]\n",
                                delay.as_secs()
                            ),
                        });
                        tokio::time::sleep(delay).await;
                    } else {
                        let body_text = r.text().await.unwrap_or_default();
                        anyhow::bail!("Copilot API error ({status}): {body_text}");
                    }
                }
                Err(e) => {
                    if attempt < MAX_RETRIES {
                        let delay = std::time::Duration::from_secs(BACKOFF_SECS[attempt as usize]);
                        log::warn!("Copilot request failed: {e}, retrying in {delay:?}…");
                        tokio::time::sleep(delay).await;
                    } else {
                        anyhow::bail!("Copilot request failed after retries: {e}");
                    }
                }
            }
        }
        let resp = resp.ok_or_else(|| anyhow::anyhow!("Failed to connect to Copilot API"))?;

        // Parse SSE stream line by line.
        let mut full_response = String::new();
        let mut tool_calls: Vec<ToolCallInfo> = Vec::new();
        let mut usage: Option<TokenUsage> = None;
        let mut byte_stream = resp.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = byte_stream.next().await {
            let chunk = chunk?;
            buf.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete lines.
            while let Some(newline_pos) = buf.find('\n') {
                let line = buf[..newline_pos].trim_end_matches('\r').to_string();
                buf = buf[newline_pos + 1..].to_string();

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    let data = data.trim();
                    if data == "[DONE]" {
                        break;
                    }

                    // Lenient deserialization — `model` is optional.
                    #[derive(serde::Deserialize)]
                    struct CopilotStreamChunk {
                        #[allow(dead_code)]
                        id: Option<String>,
                        choices: Option<Vec<CopilotChoice>>,
                        usage: Option<CopilotUsage>,
                    }
                    #[derive(serde::Deserialize)]
                    struct CopilotChoice {
                        delta: Option<CopilotDelta>,
                        #[allow(dead_code)]
                        finish_reason: Option<String>,
                    }
                    #[derive(serde::Deserialize)]
                    struct CopilotDelta {
                        content: Option<String>,
                        tool_calls: Option<Vec<CopilotToolCall>>,
                    }
                    #[derive(serde::Deserialize)]
                    struct CopilotToolCall {
                        index: u32,
                        id: Option<String>,
                        function: Option<CopilotFunction>,
                    }
                    #[derive(serde::Deserialize)]
                    struct CopilotFunction {
                        name: Option<String>,
                        arguments: Option<String>,
                    }
                    #[derive(serde::Deserialize)]
                    struct CopilotUsage {
                        prompt_tokens: u32,
                        completion_tokens: u32,
                        total_tokens: u32,
                    }

                    match serde_json::from_str::<CopilotStreamChunk>(data) {
                        Ok(chunk) => {
                            if let Some(choices) = chunk.choices {
                                for choice in &choices {
                                    if let Some(ref delta) = choice.delta {
                                        if let Some(ref content) = delta.content {
                                            full_response.push_str(content);
                                            let _ = proxy.send_event(AppEvent::LlmToken {
                                                session_id,
                                                token: content.clone(),
                                            });
                                        }
                                        if let Some(ref tcs) = delta.tool_calls {
                                            for tc in tcs {
                                                let idx = tc.index as usize;
                                                while tool_calls.len() <= idx {
                                                    tool_calls.push(ToolCallInfo {
                                                        id: String::new(),
                                                        function_name: String::new(),
                                                        arguments: String::new(),
                                                    });
                                                }
                                                if let Some(ref id) = tc.id {
                                                    tool_calls[idx].id = id.clone();
                                                }
                                                if let Some(ref func) = tc.function {
                                                    if let Some(ref name) = func.name {
                                                        tool_calls[idx].function_name =
                                                            name.clone();
                                                    }
                                                    if let Some(ref args) = func.arguments {
                                                        tool_calls[idx].arguments.push_str(args);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(u) = chunk.usage {
                                usage = Some(TokenUsage {
                                    prompt_tokens: u.prompt_tokens,
                                    completion_tokens: u.completion_tokens,
                                    total_tokens: u.total_tokens,
                                });
                            }
                        }
                        Err(e) => {
                            log::warn!("Copilot SSE parse error: {e} — data: {data}");
                        }
                    }
                }
            }
        }

        let _ = proxy.send_event(AppEvent::LlmDone {
            session_id,
            full_response,
            usage,
            tool_calls,
        });

        Ok(())
    }
}

mod accessors;
mod copilot_auth;
pub mod ollama_manager;
