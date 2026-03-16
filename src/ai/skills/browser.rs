//! Browser automation skills — click, extract text, fill form, scroll.
//!
//! Each skill injects JavaScript into a webview via `evaluate_script()` and
//! receives the result through the IPC bridge (`WebviewToRust::ScriptResult`).
#![allow(dead_code)]

use super::browser_nav::*;

use std::sync::Arc;

use serde_json::{Value, json};
use winit::event_loop::EventLoopProxy;

use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};
use crate::ai::stream::{AppEvent, PendingResultMap};

/// Register all browser automation skills.
pub fn register_browser_skills(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    register_browser_click(registry, proxy.clone(), pending_results.clone());
    register_browser_extract_text(registry, proxy.clone(), pending_results.clone());
    register_browser_fill_form(registry, proxy.clone(), pending_results.clone());
    register_browser_scroll(registry, proxy.clone(), pending_results.clone());
    register_browser_query_selector(registry, proxy.clone(), pending_results.clone());
    register_browser_submit_form(registry, proxy.clone(), pending_results.clone());
    register_browser_wait_for(registry, proxy.clone(), pending_results.clone());
    register_browser_snapshot(registry, proxy, pending_results);
}

// ─── Helper: inject script and await result ─────────────────────────────────

/// Build a self-executing JS wrapper that runs `body_js` inside a try-catch
/// and posts the result (or error) via `window.__CRONYMAX_IPC__.postMessage`.
pub(super) fn wrap_js(request_id: &str, body_js: &str) -> String {
    format!(
        r#"(function() {{
  try {{
    {body_js}
  }} catch(e) {{
    window.__CRONYMAX_IPC__.postMessage({{
      type: 'script_result',
      payload: {{ request_id: '{request_id}', result: null, error: e.message }}
    }});
  }}
}})();"#,
        body_js = body_js,
        request_id = request_id,
    )
}

/// Send a JS script to a webview and await the IPC result.
pub(super) async fn inject_and_await(
    proxy: &EventLoopProxy<AppEvent>,
    pending: &PendingResultMap,
    webview_id: u32,
    script: String,
    request_id: String,
    timeout_secs: u64,
) -> anyhow::Result<Value> {
    let (tx, rx) = tokio::sync::oneshot::channel();

    {
        let mut map = pending
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;
        map.insert(request_id.clone(), tx);
    }

    let _ = proxy.send_event(AppEvent::InjectScript {
        webview_id,
        script,
        request_id: request_id.clone(),
    });

    match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok(value)) => {
            // Check if the value contains an error field.
            if let Some(err) = value.get("error").and_then(|e| e.as_str()) {
                Ok(json!({ "error": err }))
            } else {
                // Parse the nested result string as JSON if possible.
                let result_str = value.get("result").and_then(|r| r.as_str()).unwrap_or("{}");
                match serde_json::from_str::<Value>(result_str) {
                    Ok(parsed) => Ok(parsed),
                    Err(_) => Ok(json!({ "result": result_str })),
                }
            }
        }
        Ok(Err(_)) => Ok(json!({ "error": "Result channel closed unexpectedly" })),
        Err(_) => {
            if let Ok(mut map) = pending.lock() {
                map.remove(&request_id);
            }
            Ok(json!({
                "error": format!("Browser script timed out after {}s", timeout_secs)
            }))
        }
    }
}

// ─── browser_click ──────────────────────────────────────────────────────────

fn register_browser_click(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.browser.click".into(),
        description: "Click an element on a webpage in a webview. \
            Specify either a CSS selector or text content to identify the element."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "webview_id": {
                    "type": "integer",
                    "description": "The webview tab ID. Use 0 for the currently active webview."
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector for the element to click (e.g., '#submit-btn', '.nav-link')"
                },
                "text": {
                    "type": "string",
                    "description": "Text content to match if no CSS selector is provided."
                }
            },
            "required": ["webview_id"]
        }),
        category: "browser".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let webview_id = args["webview_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'webview_id'"))?
                as u32;
            let selector = args["selector"].as_str().map(|s| s.to_string());
            let text = args["text"].as_str().map(|s| s.to_string());

            if selector.is_none() && text.is_none() {
                return Ok(json!({ "error": "Either 'selector' or 'text' must be provided" }));
            }

            let request_id = uuid::Uuid::new_v4().to_string();

            let body_js = if let Some(sel) = &selector {
                let sel_escaped = sel.replace('\\', "\\\\").replace('\'', "\\'");
                format!(
                    r#"var el = document.querySelector('{sel}');
if (!el) {{
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: '{rid}', result: null, error: 'Element not found: {sel}' }}
  }});
}} else {{
  el.scrollIntoView({{ block: 'center' }});
  el.click();
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: '{rid}', result: JSON.stringify({{ clicked: true, tag: el.tagName, text: (el.textContent || '').substring(0, 100) }}), error: null }}
  }});
}}"#,
                    sel = sel_escaped,
                    rid = request_id,
                )
            } else {
                let text_val = text.as_deref().unwrap_or("");
                let text_escaped = text_val.replace('\\', "\\\\").replace('\'', "\\'");
                format!(
                    r#"var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
var el = null;
while (walker.nextNode()) {{
  if (walker.currentNode.textContent && walker.currentNode.textContent.trim().includes('{text}') && walker.currentNode.children.length === 0) {{
    el = walker.currentNode;
    break;
  }}
}}
if (!el) {{
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: '{rid}', result: null, error: 'No element found with text: {text}' }}
  }});
}} else {{
  el.scrollIntoView({{ block: 'center' }});
  el.click();
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: '{rid}', result: JSON.stringify({{ clicked: true, tag: el.tagName, text: (el.textContent || '').substring(0, 100) }}), error: null }}
  }});
}}"#,
                    text = text_escaped,
                    rid = request_id,
                )
            };

            let script = wrap_js(&request_id, &body_js);
            inject_and_await(&proxy, &pending, webview_id, script, request_id, 10).await
        })
    });

    registry.register(skill, handler);
}

// ─── browser_extract_text ───────────────────────────────────────────────────

fn register_browser_extract_text(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.browser.extract_text".into(),
        description: "Extract text content from a webpage loaded in a webview. \
            Can return plain text, simplified markdown, or raw HTML."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "webview_id": {
                    "type": "integer",
                    "description": "The webview tab ID. Use 0 for the currently active webview."
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector to scope extraction (default: 'body')"
                },
                "format": {
                    "type": "string",
                    "enum": ["text", "markdown", "html"],
                    "description": "Output format. Default: 'text'"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return. Default: 8000"
                }
            },
            "required": ["webview_id"]
        }),
        category: "browser".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let webview_id = args["webview_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'webview_id'"))?
                as u32;
            let selector = args["selector"]
                .as_str()
                .unwrap_or("body")
                .replace('\\', "\\\\")
                .replace('\'', "\\'");
            let format = args["format"].as_str().unwrap_or("text");
            let max_chars = args["max_chars"].as_u64().unwrap_or(8000);

            let request_id = uuid::Uuid::new_v4().to_string();

            let extract_expr = match format {
                "html" => format!("el.innerHTML.substring(0, {})", max_chars),
                "markdown" => {
                    // Simple DOM-to-markdown conversion inline.
                    format!(
                        r#"(function() {{
  function toMd(node) {{
    if (node.nodeType === 3) return node.textContent;
    if (node.nodeType !== 1) return '';
    var tag = node.tagName.toLowerCase();
    var children = Array.from(node.childNodes).map(toMd).join('');
    if (tag === 'h1') return '# ' + children.trim() + '\n\n';
    if (tag === 'h2') return '## ' + children.trim() + '\n\n';
    if (tag === 'h3') return '### ' + children.trim() + '\n\n';
    if (tag === 'h4') return '#### ' + children.trim() + '\n\n';
    if (tag === 'p') return children.trim() + '\n\n';
    if (tag === 'br') return '\n';
    if (tag === 'a') return '[' + children.trim() + '](' + (node.href || '') + ')';
    if (tag === 'strong' || tag === 'b') return '**' + children.trim() + '**';
    if (tag === 'em' || tag === 'i') return '*' + children.trim() + '*';
    if (tag === 'code') return '`' + children.trim() + '`';
    if (tag === 'pre') return '```\n' + children.trim() + '\n```\n\n';
    if (tag === 'li') return '- ' + children.trim() + '\n';
    if (tag === 'ul' || tag === 'ol') return children + '\n';
    if (tag === 'img') return '![' + (node.alt || '') + '](' + (node.src || '') + ')';
    return children;
  }}
  return toMd(el).substring(0, {max_chars});
}})()"#,
                        max_chars = max_chars,
                    )
                }
                _ => format!("el.innerText.substring(0, {})", max_chars),
            };

            let body_js = format!(
                r#"var el = document.querySelector('{sel}');
if (!el) {{
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: '{rid}', result: null, error: 'Element not found: {sel}' }}
  }});
}} else {{
  var content = {extract};
  var truncated = content.length >= {max_chars};
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: '{rid}', result: JSON.stringify({{
      content: content,
      truncated: truncated,
      char_count: content.length,
      title: document.title || '',
      url: window.location.href || ''
    }}), error: null }}
  }});
}}"#,
                sel = selector,
                rid = request_id,
                extract = extract_expr,
                max_chars = max_chars,
            );

            let script = wrap_js(&request_id, &body_js);
            inject_and_await(&proxy, &pending, webview_id, script, request_id, 10).await
        })
    });

    registry.register(skill, handler);
}

// ─── browser_fill_form ──────────────────────────────────────────────────────

fn register_browser_fill_form(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.browser.fill_form".into(),
        description: "Set the value of a form input, textarea, or select element on a webpage. \
            Dispatches proper input/change events so JavaScript frameworks detect the change."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "webview_id": {
                    "type": "integer",
                    "description": "The webview tab ID. Use 0 for the currently active webview."
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector for the input element"
                },
                "value": {
                    "type": "string",
                    "description": "Value to set"
                }
            },
            "required": ["webview_id", "selector", "value"]
        }),
        category: "browser".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let webview_id = args["webview_id"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Missing 'webview_id'"))?
                as u32;
            let selector = args["selector"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector'"))?
                .replace('\\', "\\\\")
                .replace('\'', "\\'");
            let value = args["value"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'value'"))?
                .replace('\\', "\\\\")
                .replace('\'', "\\'");

            let request_id = uuid::Uuid::new_v4().to_string();

            let body_js = format!(
                r#"var el = document.querySelector('{sel}');
if (!el) {{
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: '{rid}', result: null, error: 'Element not found: {sel}' }}
  }});
}} else {{
  var nativeSetter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype, 'value'
  );
  if (!nativeSetter) nativeSetter = Object.getOwnPropertyDescriptor(
    window.HTMLTextAreaElement.prototype, 'value'
  );
  if (nativeSetter && nativeSetter.set) {{
    nativeSetter.set.call(el, '{val}');
  }} else {{
    el.value = '{val}';
  }}
  el.dispatchEvent(new Event('input', {{ bubbles: true }}));
  el.dispatchEvent(new Event('change', {{ bubbles: true }}));
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: '{rid}', result: JSON.stringify({{ filled: true, tag: el.tagName, selector: '{sel}' }}), error: null }}
  }});
}}"#,
                sel = selector,
                val = value,
                rid = request_id,
            );

            let script = wrap_js(&request_id, &body_js);
            inject_and_await(&proxy, &pending, webview_id, script, request_id, 10).await
        })
    });

    registry.register(skill, handler);
}

// ─── browser_scroll ─────────────────────────────────────────────────────────
