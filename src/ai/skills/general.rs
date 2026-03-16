//! General skills — web search, file discovery, system info.
//!
//! These skills provide general-purpose capabilities available to all profiles.
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};
use crate::ai::stream::{AppEvent, PendingResultMap};

/// Register all general skills into the registry.
pub fn register_general_skills(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    register_get_system_info(registry);
    register_duckduckgo(registry);
    register_find_files(registry, proxy, pending_results);
}

// ─── get_system_info ─────────────────────────────────────────────────────────

fn register_get_system_info(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.general.get_system_info".into(),
        description: "Get information about the current system environment including OS, \
            architecture, shell, working directory, and hostname."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {}
        }),
        category: "general".into(),
    };

    let handler: SkillHandler = Arc::new(|_args: Value| {
        Box::pin(async move {
            let os = std::env::consts::OS;
            let arch = std::env::consts::ARCH;
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "unknown".into());
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| "unknown".into());
            let shell = std::env::var("SHELL")
                .or_else(|_| std::env::var("COMSPEC"))
                .unwrap_or_else(|_| "unknown".into());
            let hostname = hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".into());

            Ok(json!({
                "os": os,
                "arch": arch,
                "cwd": cwd,
                "home": home,
                "shell": shell,
                "hostname": hostname,
            }))
        })
    });

    registry.register(skill, handler);
}

// ─── duckduckgo ──────────────────────────────────────────────────────────────

fn register_duckduckgo(registry: &mut SkillRegistry) {
    let skill = Skill {
        name: "cronymax.general.duckduckgo".into(),
        description: "Search the web using DuckDuckGo and return a list of results \
            with titles, URLs, and snippets. Use this to find current information, \
            documentation, or answers to factual questions."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 5)"
                }
            },
            "required": ["query"]
        }),
        category: "general".into(),
    };

    let handler: SkillHandler = Arc::new(|args: Value| {
        Box::pin(async move {
            let query = args["query"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'query' argument"))?;
            let max_results = args["max_results"].as_u64().unwrap_or(5) as usize;

            let url = format!(
                "https://html.duckduckgo.com/html/?q={}",
                urlencoding::encode(query)
            );

            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .user_agent("Mozilla/5.0 (compatible; cronymax/1.0)")
                .build()?;

            let resp = client.get(&url).send().await?;
            let html = resp.text().await?;

            let results = parse_ddg_results(&html, max_results);

            Ok(json!({
                "results": results,
                "count": results.len(),
                "query": query,
            }))
        })
    });

    registry.register(skill, handler);
}

/// Parse DuckDuckGo HTML search results into structured JSON.
fn parse_ddg_results(html: &str, max_results: usize) -> Vec<Value> {
    let mut results = Vec::new();

    // DuckDuckGo HTML results are in <div class="result"> blocks.
    // Each has an <a class="result__a"> for the title/URL and
    // <a class="result__snippet"> for the snippet text.
    for result_block in html.split("class=\"result__body\"") {
        if results.len() >= max_results {
            break;
        }

        let title = extract_between(result_block, "class=\"result__a\"", "</a>")
            .map(|s| strip_html_tags(&s));
        let href = extract_attr(result_block, "class=\"result__a\"", "href");
        let snippet = extract_between(result_block, "class=\"result__snippet\"", "</a>")
            .map(|s| strip_html_tags(&s));

        if let (Some(title), Some(href)) = (title, href) {
            if title.is_empty() {
                continue;
            }
            // DuckDuckGo wraps URLs in a redirect; extract the actual URL.
            let url = extract_ddg_url(&href).unwrap_or(href);
            results.push(json!({
                "title": title.trim(),
                "url": url.trim(),
                "snippet": snippet.map(|s| s.trim().to_string()).unwrap_or_default(),
            }));
        }
    }

    results
}

/// Extract text between a start marker and end tag.
fn extract_between(html: &str, start_marker: &str, end_tag: &str) -> Option<String> {
    let start = html.find(start_marker)?;
    let after_marker = &html[start + start_marker.len()..];
    // Skip to the end of the opening tag.
    let tag_end = after_marker.find('>')?;
    let content_start = &after_marker[tag_end + 1..];
    let end = content_start.find(end_tag)?;
    Some(content_start[..end].to_string())
}

/// Extract an href attribute value from within a tag.
fn extract_attr(html: &str, marker: &str, attr: &str) -> Option<String> {
    let start = html.find(marker)?;
    let before = &html[..start];
    // Find the opening < before the marker.
    let tag_start = before.rfind('<')?;
    let tag_content =
        &html[tag_start..start + marker.len() + 200.min(html.len() - start - marker.len())];
    let attr_marker = format!("{}=\"", attr);
    let attr_start = tag_content.find(&attr_marker)?;
    let value_start = &tag_content[attr_start + attr_marker.len()..];
    let end = value_start.find('"')?;
    Some(value_start[..end].to_string())
}

/// Extract the actual URL from DuckDuckGo's redirect URL.
fn extract_ddg_url(redirect_url: &str) -> Option<String> {
    // DDG URLs look like: //duckduckgo.com/l/?uddg=https%3A%2F%2F...&rut=...
    if let Some(idx) = redirect_url.find("uddg=") {
        let encoded = &redirect_url[idx + 5..];
        let end = encoded.find('&').unwrap_or(encoded.len());
        let decoded = urlencoding::decode(&encoded[..end]).ok()?;
        Some(decoded.into_owned())
    } else if redirect_url.starts_with("http") {
        Some(redirect_url.to_string())
    } else {
        None
    }
}

/// Strip HTML tags from a string.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    // Decode common HTML entities.
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

// ─── find_files ──────────────────────────────────────────────────────────────

fn register_find_files(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.general.find_files".into(),
        description: "Search for files in the workspace by name pattern (glob or substring). \
            Returns matching file paths. Use this to discover project structure and locate files."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "File name pattern (glob like '*.rs' or substring like 'config')"
                },
                "cwd": {
                    "type": "string",
                    "description": "Directory to search in (default: current working directory)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results (default: 50)"
                }
            },
            "required": ["query"]
        }),
        category: "general".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let query = args["query"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'query' argument"))?
                .to_string();
            let cwd = args["cwd"].as_str().map(String::from).unwrap_or_else(|| {
                std::env::current_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| ".".into())
            });
            let max_results = args["max_results"].as_u64().unwrap_or(50) as usize;

            let request_id = uuid::Uuid::new_v4().to_string();

            let (tx, rx) = tokio::sync::oneshot::channel();
            {
                let mut map = pending
                    .lock()
                    .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
                map.insert(request_id.clone(), tx);
            }

            let _ = proxy.send_event(AppEvent::FindFiles {
                query: query.clone(),
                cwd,
                max_results,
                request_id: request_id.clone(),
            });

            match tokio::time::timeout(std::time::Duration::from_secs(10), rx).await {
                Ok(Ok(value)) => Ok(value),
                Ok(Err(_)) => Ok(json!({ "error": "Channel closed", "files": [] })),
                Err(_) => {
                    // Clean up pending entry on timeout.
                    if let Ok(mut map) = pending.lock() {
                        map.remove(&request_id);
                    }
                    Ok(json!({ "error": "Timed out after 10s", "files": [] }))
                }
            }
        })
    });

    registry.register(skill, handler);
}
