use super::*;

/// Per-provider fetch configuration: (display_name, provider, base_url, api_key, extra_headers).
type ProviderFetchConfig = (String, LlmProvider, String, String, Vec<(String, String)>);

impl LlmClient {
    /// Get the current LLM config.
    pub fn config(&self) -> &LlmConfig {
        &self.config
    }

    /// Max context tokens from the underlying config.
    pub fn max_context_tokens(&self) -> usize {
        self.config.max_context_tokens
    }

    /// Reserve tokens from the underlying config.
    pub fn reserve_tokens(&self) -> usize {
        self.config.reserve_tokens
    }

    /// System prompt from the underlying config.
    pub fn system_prompt(&self) -> Option<&str> {
        self.config.system_prompt.as_deref()
    }

    /// Get a clone of the underlying async-openai client, if initialized.
    ///
    /// Used by the channel agent loop to make LLM calls outside the
    /// EventLoopProxy-based streaming pattern.
    pub fn openai_client(&self) -> Option<Client<OpenAIConfig>> {
        self.openai_client.clone()
    }

    /// Return the currently configured model as a single-item list.
    /// The full model catalogue is populated asynchronously via `fetch_available_models()`.
    pub fn current_model_item(&self) -> ModelListItem {
        let provider_name = match self.config.provider {
            LlmProvider::OpenAI => "OpenAI",
            LlmProvider::Anthropic => "Anthropic",
            LlmProvider::Copilot => "GitHub Copilot",
            LlmProvider::Ollama => "Ollama (local)",
            LlmProvider::Custom => "Custom",
        };
        ModelListItem {
            provider: self.config.provider.clone(),
            model: self.config.model.clone(),
            display_label: format!("{} / {}", provider_name, self.config.model),
            available: true,
        }
    }

    /// Fetch models from provider APIs asynchronously.
    ///
    /// Queries the `/models` endpoint for each reachable provider and sends
    /// the result back via `AppEvent::ModelsLoaded`.
    pub fn fetch_available_models(
        &self,
        proxy: EventLoopProxy<AppEvent>,
        runtime: &Arc<tokio::runtime::Runtime>,
    ) {
        // Build per-provider fetch configs: (display_name, provider, base_url, api_key, extra_headers).
        let mut fetchers: Vec<ProviderFetchConfig> = Vec::new();

        // Track provider names already added (to avoid duplicates with configured providers).
        let mut added_providers = std::collections::HashSet::new();

        // Add user-configured providers from config.toml [[ai.providers]] first.
        for p in &self.configured_providers {
            let provider = match p.provider_type.as_str() {
                "openai" => LlmProvider::OpenAI,
                "ollama" => LlmProvider::Ollama,
                "copilot" => LlmProvider::Copilot,
                "anthropic" => LlmProvider::Anthropic,
                _ => LlmProvider::Custom,
            };
            let base = p.api_base.clone().unwrap_or_else(|| match provider {
                LlmProvider::OpenAI => "https://api.openai.com/v1".into(),
                LlmProvider::Ollama => "http://localhost:11434/v1".into(),
                LlmProvider::Copilot => "https://models.inference.ai.azure.com".into(),
                LlmProvider::Anthropic => "https://api.anthropic.com".into(),
                LlmProvider::Custom => "http://localhost:8080/v1".into(),
            });
            let key = {
                // Try keychain first, then env var.
                let key_name = crate::secret::provider_api_key(&p.name);
                self.secret_store
                    .resolve(&key_name, p.api_key_env.as_deref(), &p.secret_storage)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| {
                        if provider == LlmProvider::Ollama {
                            "ollama".into()
                        } else {
                            String::new()
                        }
                    })
            };
            if key.is_empty() {
                log::warn!("Skipping provider '{}': no API key available", p.name);
                continue;
            }
            let mut headers = Vec::new();
            // Anthropic-specific headers.
            if provider == LlmProvider::Anthropic {
                headers.push(("x-api-key".into(), key.clone()));
                headers.push(("anthropic-version".into(), "2023-06-01".into()));
            }
            // Copilot-specific: skip adding to fetchers, track as copilot token
            // (token exchange happens inside the async task).
            if provider == LlmProvider::Copilot {
                added_providers.insert(p.provider_type.clone());
                // Will be handled by copilot_github_token path below.
                continue;
            }
            added_providers.insert(p.provider_type.clone());
            fetchers.push((p.name.clone(), provider, base, key, headers));
        }

        // Fall back to auto-detected providers if not already configured.
        // OpenAI
        if !added_providers.contains("openai")
            && let Ok(key) = std::env::var("OPENAI_API_KEY")
        {
            let base = self
                .config
                .api_base
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".into());
            fetchers.push(("OpenAI".into(), LlmProvider::OpenAI, base, key, vec![]));
        }

        // GitHub Copilot — use the OAuth token resolved at startup (from
        // device-flow or cached hosts.json).  The token exchange into a
        // session token happens inside the async task below.
        let copilot_github_token = self.copilot_github_token.clone();
        // Anthropic
        if !added_providers.contains("anthropic")
            && let Ok(key) = std::env::var("ANTHROPIC_API_KEY")
        {
            let base = self
                .config
                .api_base
                .clone()
                .unwrap_or_else(|| "https://api.anthropic.com".into());
            fetchers.push((
                "Anthropic".into(),
                LlmProvider::Anthropic,
                base,
                key.clone(),
                vec![
                    ("x-api-key".into(), key),
                    ("anthropic-version".into(), "2023-06-01".into()),
                ],
            ));
        }

        // Ollama (local — no key needed)
        if !added_providers.contains("ollama") {
            fetchers.push((
                "Ollama (local)".into(),
                LlmProvider::Ollama,
                "http://localhost:11434/v1".into(),
                "ollama".into(),
                vec![],
            ));
        }

        // Seed with the currently active model so it's always in the list.
        let seed_model = self.current_model_item();

        runtime.spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default();

            let mut all_models: Vec<ModelListItem> = vec![seed_model];
            let mut seen = std::collections::HashSet::new();

            for (display_name, provider, base_url, api_key, extra_headers) in &fetchers {
                let url = format!("{}/models", base_url.trim_end_matches('/'));
                log::info!("Fetching models from {} ({})", display_name, url);

                let mut req = client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", api_key));
                for (k, v) in extra_headers {
                    req = req.header(k.as_str(), v);
                }
                let resp = req.send().await;

                match resp {
                    Ok(r) if r.status().is_success() => {
                        if let Ok(body) = r.text().await
                            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&body)
                        {
                            // Try standard OpenAI format: {"data": [...]}
                            // Then Ollama format: {"models": [...]}
                            // Then bare array: [...]  (GitHub Models / Azure)
                            let models_arr = json
                                .get("data")
                                .and_then(|d| d.as_array())
                                .or_else(|| json.get("models").and_then(|m| m.as_array()))
                                .or_else(|| json.as_array());

                            if let Some(arr) = models_arr {
                                for entry in arr {
                                    // Prefer "name" (friendly) over "id" (may be a long registry path).
                                    let id = entry
                                        .get("name")
                                        .and_then(|v| v.as_str())
                                        .or_else(|| entry.get("id").and_then(|v| v.as_str()))
                                        .unwrap_or_default();
                                    if id.is_empty() {
                                        continue;
                                    }
                                    let key = format!("{}:{}", display_name, id);
                                    if seen.insert(key) {
                                        all_models.push(ModelListItem {
                                            provider: provider.clone(),
                                            model: id.to_string(),
                                            display_label: format!("{} / {}", display_name, id),
                                            available: true,
                                        });
                                    }
                                }
                                log::info!(
                                    "{}: fetched {} models from API",
                                    display_name,
                                    arr.len()
                                );
                            }
                        }
                    }
                    Ok(r) => {
                        log::warn!("{}: /models returned status {}", display_name, r.status());
                    }
                    Err(e) => {
                        log::warn!("{}: /models fetch failed: {}", display_name, e);
                    }
                }
            }

            // Fetch Copilot models separately — requires async token exchange.
            if let Some(gh_token) = &copilot_github_token {
                match super::exchange_copilot_token(gh_token).await {
                    Ok((session_token, api_base)) => {
                        let url = format!("{}/models", api_base.trim_end_matches('/'));
                        log::info!("Fetching models from GitHub Copilot ({})", url);

                        let mut req = client
                            .get(&url)
                            .header("Authorization", format!("Bearer {}", session_token));
                        for &(k, v) in super::COPILOT_HEADERS {
                            req = req.header(k, v);
                        }

                        match req.send().await {
                            Ok(r) if r.status().is_success() => {
                                if let Ok(body) = r.text().await
                                    && let Ok(json) =
                                        serde_json::from_str::<serde_json::Value>(&body)
                                {
                                    let models_arr = json
                                        .get("data")
                                        .and_then(|d| d.as_array())
                                        .or_else(|| json.get("models").and_then(|m| m.as_array()))
                                        .or_else(|| json.as_array());

                                    if let Some(arr) = models_arr {
                                        let display_name = "GitHub Copilot";
                                        for entry in arr {
                                            let id = entry
                                                .get("id")
                                                .and_then(|v| v.as_str())
                                                .or_else(|| {
                                                    entry.get("name").and_then(|v| v.as_str())
                                                })
                                                .unwrap_or_default();
                                            if id.is_empty() {
                                                continue;
                                            }
                                            let key = format!("{display_name}:{id}");
                                            if seen.insert(key) {
                                                all_models.push(ModelListItem {
                                                    provider: LlmProvider::Copilot,
                                                    model: id.to_string(),
                                                    display_label: format!("{display_name} / {id}"),
                                                    available: true,
                                                });
                                            }
                                        }
                                        log::info!(
                                            "GitHub Copilot: fetched {} models from API",
                                            arr.len()
                                        );
                                    }
                                }
                            }
                            Ok(r) => {
                                log::warn!(
                                    "GitHub Copilot: /models returned status {}",
                                    r.status()
                                );
                            }
                            Err(e) => {
                                log::warn!("GitHub Copilot: /models fetch failed: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("GitHub Copilot: token exchange failed: {}", e);
                    }
                }
            }

            if !all_models.is_empty() {
                let _ = proxy.send_event(AppEvent::ModelsLoaded { models: all_models });
            }
        });
    }
}
