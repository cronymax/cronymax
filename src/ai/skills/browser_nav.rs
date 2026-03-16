use super::browser::{inject_and_await, wrap_js};

use crate::ai::skills::{Skill, SkillHandler, SkillRegistry};
use crate::ai::stream::{AppEvent, PendingResultMap};
use serde_json::{Value, json};
use std::sync::Arc;
use winit::event_loop::EventLoopProxy;

pub(super) fn register_browser_scroll(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.browser.scroll".into(),
        description: "Scroll a webpage in a webview. \
            Can scroll by direction/amount or scroll a specific element into view."
            .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "webview_id": {
                    "type": "integer",
                    "description": "The webview tab ID. Use 0 for the currently active webview."
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "Scroll direction. Default: 'down'"
                },
                "amount": {
                    "type": "integer",
                    "description": "Pixels to scroll. Default: viewport height"
                },
                "selector": {
                    "type": "string",
                    "description": "If provided, scroll this element into view instead of directional scrolling"
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
            let direction = args["direction"].as_str().unwrap_or("down");
            let amount = args["amount"].as_i64();
            let selector = args["selector"]
                .as_str()
                .map(|s| s.replace('\\', "\\\\").replace('\'', "\\'"));

            let request_id = uuid::Uuid::new_v4().to_string();

            let body_js = if let Some(sel) = &selector {
                format!(
                    r#"var el = document.querySelector('{sel}');
if (!el) {{
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: '{rid}', result: null, error: 'Element not found: {sel}' }}
  }});
}} else {{
  el.scrollIntoView({{ behavior: 'smooth', block: 'center' }});
  window.__CRONYMAX_IPC__.postMessage({{
    type: 'script_result',
    payload: {{ request_id: '{rid}', result: JSON.stringify({{
      scrolled: true,
      scroll_x: Math.round(window.scrollX),
      scroll_y: Math.round(window.scrollY),
      page_height: document.documentElement.scrollHeight
    }}), error: null }}
  }});
}}"#,
                    sel = sel,
                    rid = request_id,
                )
            } else {
                let (dx, dy) = match direction {
                    "up" => (0, -(amount.unwrap_or(0))),
                    "down" => (0, amount.unwrap_or(0)),
                    "left" => (-(amount.unwrap_or(0)), 0),
                    "right" => (amount.unwrap_or(0), 0),
                    _ => (0, amount.unwrap_or(0)),
                };
                // If amount is 0 (not specified), use viewport height.
                let amount_js = if amount.is_none() {
                    match direction {
                        "up" => "var dx = 0; var dy = -window.innerHeight;".to_string(),
                        "left" => "var dx = -window.innerWidth; var dy = 0;".to_string(),
                        "right" => "var dx = window.innerWidth; var dy = 0;".to_string(),
                        _ => "var dx = 0; var dy = window.innerHeight;".to_string(),
                    }
                } else {
                    format!("var dx = {}; var dy = {};", dx, dy)
                };
                format!(
                    r#"{amount_js}
window.scrollBy({{ left: dx, top: dy, behavior: 'smooth' }});
window.__CRONYMAX_IPC__.postMessage({{
  type: 'script_result',
  payload: {{ request_id: '{rid}', result: JSON.stringify({{
    scrolled: true,
    scroll_x: Math.round(window.scrollX),
    scroll_y: Math.round(window.scrollY),
    page_height: document.documentElement.scrollHeight
  }}), error: null }}
}});"#,
                    amount_js = amount_js,
                    rid = request_id,
                )
            };

            let script = wrap_js(&request_id, &body_js);
            inject_and_await(&proxy, &pending, webview_id, script, request_id, 10).await
        })
    });

    registry.register(skill, handler);
}

// ─── browser_query_selector ─────────────────────────────────────────────────

pub(super) fn register_browser_query_selector(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.browser.query_selector".into(),
        description:
            "Query the DOM using a CSS selector and return information about matching elements."
                .into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "webview_id": { "type": "integer", "description": "Webview tab ID (0 = active)." },
                "selector": { "type": "string", "description": "CSS selector to query." },
                "max_results": { "type": "integer", "description": "Maximum elements to return (default: 10)." }
            },
            "required": ["selector"]
        }),
        category: "browser".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let webview_id = args["webview_id"].as_u64().unwrap_or(0) as u32;
            let selector = args["selector"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector'"))?
                .to_string();
            let max_results = args["max_results"].as_u64().unwrap_or(10);
            let request_id = uuid::Uuid::new_v4().to_string();
            let body_js = format!(
                r#"var els = document.querySelectorAll({sel});
var results = [];
var max = {max};
for (var i = 0; i < Math.min(els.length, max); i++) {{
  var el = els[i];
  var rect = el.getBoundingClientRect();
  results.push({{
    tag: el.tagName.toLowerCase(),
    id: el.id || null,
    classes: el.className || null,
    text: (el.textContent || '').substring(0, 200).trim(),
    href: el.href || null,
    type: el.type || null,
    visible: rect.width > 0 && rect.height > 0,
    rect: {{ x: Math.round(rect.x), y: Math.round(rect.y), w: Math.round(rect.width), h: Math.round(rect.height) }}
  }});
}}
window.__CRONYMAX_IPC__.postMessage({{
  type: 'script_result',
  payload: {{ request_id: '{rid}', result: JSON.stringify({{ elements: results, count: results.length, total: els.length }}), error: null }}
}});"#,
                sel = json!(selector),
                max = max_results,
                rid = request_id,
            );
            let script = wrap_js(&request_id, &body_js);
            inject_and_await(&proxy, &pending, webview_id, script, request_id, 10).await
        })
    });

    registry.register(skill, handler);
}

// ─── browser_submit_form ────────────────────────────────────────────────────

pub(super) fn register_browser_submit_form(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.browser.submit_form".into(),
        description: "Submit a form by finding either the form element or a submit button matching the CSS selector.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "webview_id": { "type": "integer", "description": "Webview tab ID (0 = active)." },
                "selector": { "type": "string", "description": "CSS selector for the form or an element inside it." }
            },
            "required": ["selector"]
        }),
        category: "browser".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let webview_id = args["webview_id"].as_u64().unwrap_or(0) as u32;
            let selector = args["selector"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector'"))?
                .to_string();
            let request_id = uuid::Uuid::new_v4().to_string();
            let body_js = format!(
                r#"var el = document.querySelector({sel});
var submitted = false;
var form_action = null;
if (el) {{
  var form = el.tagName === 'FORM' ? el : el.closest('form');
  if (form) {{
    form_action = form.action || null;
    form.submit();
    submitted = true;
  }} else {{
    var btn = el.querySelector('[type=submit]') || el;
    btn.click();
    submitted = true;
  }}
}}
window.__CRONYMAX_IPC__.postMessage({{
  type: 'script_result',
  payload: {{ request_id: '{rid}', result: JSON.stringify({{ submitted: submitted, form_action: form_action }}), error: null }}
}});"#,
                sel = json!(selector),
                rid = request_id,
            );
            let script = wrap_js(&request_id, &body_js);
            inject_and_await(&proxy, &pending, webview_id, script, request_id, 10).await
        })
    });

    registry.register(skill, handler);
}

// ─── browser_wait_for ───────────────────────────────────────────────────────

pub(super) fn register_browser_wait_for(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.browser.wait_for".into(),
        description: "Wait for an element matching a CSS selector to appear in the DOM. Polls until found or timeout.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "webview_id": { "type": "integer", "description": "Webview tab ID (0 = active)." },
                "selector": { "type": "string", "description": "CSS selector to wait for." },
                "timeout_ms": { "type": "integer", "description": "Max wait time in ms (default: 5000)." }
            },
            "required": ["selector"]
        }),
        category: "browser".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let webview_id = args["webview_id"].as_u64().unwrap_or(0) as u32;
            let selector = args["selector"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector'"))?
                .to_string();
            let timeout_ms = args["timeout_ms"].as_u64().unwrap_or(5000);
            let request_id = uuid::Uuid::new_v4().to_string();
            let body_js = format!(
                r#"var start = Date.now();
var timeout = {timeout};
var sel = {sel};
function check() {{
  var el = document.querySelector(sel);
  if (el) {{
    window.__CRONYMAX_IPC__.postMessage({{
      type: 'script_result',
      payload: {{ request_id: '{rid}', result: JSON.stringify({{ found: true, elapsed_ms: Date.now() - start }}), error: null }}
    }});
  }} else if (Date.now() - start > timeout) {{
    window.__CRONYMAX_IPC__.postMessage({{
      type: 'script_result',
      payload: {{ request_id: '{rid}', result: JSON.stringify({{ found: false, elapsed_ms: Date.now() - start }}), error: null }}
    }});
  }} else {{
    setTimeout(check, 100);
  }}
}}
check();"#,
                timeout = timeout_ms,
                sel = json!(selector),
                rid = request_id,
            );
            let rust_timeout = timeout_ms / 1000 + 3;
            let script = wrap_js(&request_id, &body_js);
            inject_and_await(
                &proxy,
                &pending,
                webview_id,
                script,
                request_id,
                rust_timeout,
            )
            .await
        })
    });

    registry.register(skill, handler);
}

// ─── browser_snapshot ───────────────────────────────────────────────────────

pub(super) fn register_browser_snapshot(
    registry: &mut SkillRegistry,
    proxy: EventLoopProxy<AppEvent>,
    pending_results: PendingResultMap,
) {
    let skill = Skill {
        name: "cronymax.browser.snapshot".into(),
        description: "Capture a visual snapshot of the current webview as an SVG data URL with element dimensions.".into(),
        parameters_schema: json!({
            "type": "object",
            "properties": {
                "webview_id": { "type": "integer", "description": "Webview tab ID (0 = active)." },
                "selector": { "type": "string", "description": "CSS selector for a specific element to capture (default: body)." }
            }
        }),
        category: "browser".into(),
    };

    let handler: SkillHandler = Arc::new(move |args: Value| {
        let proxy = proxy.clone();
        let pending = pending_results.clone();
        Box::pin(async move {
            let webview_id = args["webview_id"].as_u64().unwrap_or(0) as u32;
            let selector = args["selector"].as_str().unwrap_or("body").to_string();
            let request_id = uuid::Uuid::new_v4().to_string();
            let body_js = format!(
                r#"var target = document.querySelector({sel}) || document.body;
var rect = target.getBoundingClientRect();
var svgStr = '<svg xmlns="http://www.w3.org/2000/svg" width="' + Math.round(rect.width) + '" height="' + Math.round(rect.height) + '">' +
  '<foreignObject width="100%" height="100%">' +
  '<div xmlns="http://www.w3.org/1999/xhtml">' + target.outerHTML + '</div>' +
  '</foreignObject></svg>';
var dataUrl = 'data:image/svg+xml;base64,' + btoa(unescape(encodeURIComponent(svgStr)));
window.__CRONYMAX_IPC__.postMessage({{
  type: 'script_result',
  payload: {{ request_id: '{rid}', result: JSON.stringify({{
    base64: dataUrl,
    width: Math.round(rect.width),
    height: Math.round(rect.height)
  }}), error: null }}
}});"#,
                sel = json!(selector),
                rid = request_id,
            );
            let script = wrap_js(&request_id, &body_js);
            inject_and_await(&proxy, &pending, webview_id, script, request_id, 15).await
        })
    });

    registry.register(skill, handler);
}
