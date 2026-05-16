//! Prompt template variable renderer.
//!
//! Resolves `${…}` placeholders in a template string using the provided
//! [`VarContext`]. Unknown variables are left as-is and logged at `warn`
//! level. After all substitutions, runs of three or more consecutive blank
//! lines are collapsed to two.
//!
//! ## Quick start
//!
//! ```ignore
//! use cronymax::prompt::{renderer::render, vars::VarContext};
//!
//! let ctx = VarContext::builder()
//!     .workspace_root(workspace_root.clone())
//!     .agent_name("crony")
//!     .tools(vec!["bash".into(), "search".into()])
//!     .build();
//!
//! let rendered = render(&agent_def.system_prompt, &ctx);
//! ```
//!
//! ## Supported variables
//!
//! | Variable | Resolved value |
//! |---|---|
//! | `${workspace/dir}` | Absolute workspace root path |
//! | `${workspace/name}` | Last path segment of workspace root |
//! | `${workspace/git/branch}` | Current git branch (`git rev-parse --abbrev-ref HEAD`) |
//! | `${agent/name}` | Name of the active agent |
//! | `${date}` | Current date in `YYYY-MM-DD` format |
//! | `${tools}` | Newline-separated list of available tool names |
//! | `${agents}` | Newline-separated list of workspace agent names |
//! | `${memory/key}` | Value of `key` from the read namespace |
//! | `${memory/summary}` | One-line summary of the read namespace |
//! | `${memory/ns/key}` | Value of `key` from named namespace `ns` |
//! | `${<user_var>}` | User-defined variable from `AgentDef.vars` |

use std::time::{SystemTime, UNIX_EPOCH};

use tracing::warn;

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use crate::{memory::MemoryManager, runtime::state::MemoryNamespaceId};

/// All context needed to expand variables in a prompt template.
///
/// Build with [`VarContext::builder()`] and then pass to
/// [`super::renderer::render()`].
#[derive(Default)]
pub struct VarContext {
    /// Absolute workspace root path.
    pub workspace_root: Option<PathBuf>,
    /// Human-readable workspace name (last segment of `workspace_root`).
    pub workspace_name: Option<String>,
    /// Current git branch, resolved lazily.
    pub git_branch: Option<String>,
    /// Name of the active agent.
    pub agent_name: Option<String>,
    /// Tool names available to the agent.
    pub tools: Vec<String>,
    /// Workspace agent names for the `${agents}` variable.
    pub agents: Vec<String>,
    /// Namespace to read memory from.
    pub read_namespace: Option<MemoryNamespaceId>,
    /// Shared memory manager for `${memory/…}` resolution.
    pub memory_manager: Option<Arc<MemoryManager>>,
    /// User-defined variables from `AgentDef.vars`.
    pub user_vars: HashMap<String, String>,
}

impl VarContext {
    pub fn builder() -> VarContextBuilder {
        VarContextBuilder::default()
    }
}

/// Fluent builder for [`VarContext`].
#[derive(Default)]
pub struct VarContextBuilder(VarContext);

impl VarContextBuilder {
    pub fn workspace_root(mut self, root: PathBuf) -> Self {
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_owned();
        self.0.workspace_name = Some(name);
        self.0.workspace_root = Some(root);
        self
    }

    pub fn agent_name(mut self, name: impl Into<String>) -> Self {
        self.0.agent_name = Some(name.into());
        self
    }

    pub fn tools(mut self, tools: Vec<String>) -> Self {
        self.0.tools = tools;
        self
    }

    pub fn agents(mut self, agents: Vec<String>) -> Self {
        self.0.agents = agents;
        self
    }

    pub fn read_namespace(mut self, ns: MemoryNamespaceId) -> Self {
        self.0.read_namespace = Some(ns);
        self
    }

    pub fn memory_manager(mut self, mgr: Arc<MemoryManager>) -> Self {
        self.0.memory_manager = Some(mgr);
        self
    }

    pub fn user_vars(mut self, vars: HashMap<String, String>) -> Self {
        self.0.user_vars = vars;
        self
    }

    pub fn build(self) -> VarContext {
        self.0
    }
}

static TRIPLE_NEWLINE: &str = "\n\n\n";

/// Render a prompt template, substituting all `${…}` variables from `ctx`.
///
/// This is a **synchronous** function. Memory variables are resolved from
/// the in-memory cache (no I/O). For freshly-loaded namespaces where the
/// cache is cold, call `MemoryManager::read()` before rendering.
pub fn render(template: &str, ctx: &VarContext) -> String {
    let mut result = template.to_owned();

    // ── Builtin workspace vars ──────────────────────────────────────────────
    if let Some(root) = &ctx.workspace_root {
        result = result.replace("${workspace/dir}", &root.to_string_lossy());
    }
    if let Some(name) = &ctx.workspace_name {
        result = result.replace("${workspace/name}", name);
    }

    // git branch — attempt to resolve if not already known.
    let branch = ctx.git_branch.as_deref().or(None);
    let branch_str = if let Some(b) = branch {
        b.to_owned()
    } else if let Some(root) = &ctx.workspace_root {
        resolve_git_branch(root).unwrap_or_else(|| "unknown".to_owned())
    } else {
        "unknown".to_owned()
    };
    result = result.replace("${workspace/git/branch}", &branch_str);

    // ── Builtin agent / date / tools / agents vars ──────────────────────────
    if let Some(agent_name) = &ctx.agent_name {
        result = result.replace("${agent/name}", agent_name);
    }

    let today = today_date();
    result = result.replace("${date}", &today);

    let tools_str = ctx.tools.join("\n");
    result = result.replace("${tools}", &tools_str);

    let agents_str = ctx.agents.join("\n");
    result = result.replace("${agents}", &agents_str);

    // ── Memory vars (sync — reads from cache) ───────────────────────────────
    // `${memory/summary}` and `${memory/<key>}` use the configured read namespace;
    // `${memory/<ns>/<key>}` reads from a named namespace.
    if result.contains("${memory/") {
        result = resolve_memory_vars(result, ctx);
    }

    // ── User vars (one-level recursive builtin expansion) ───────────────────
    for (k, v) in &ctx.user_vars {
        let placeholder = format!("${{{}}}", k);
        // Expand any builtin vars inside the user value.
        let expanded_v = render_builtins_only(v, ctx);
        result = result.replace(&placeholder, &expanded_v);
    }

    // ── Warn on any remaining unresolved vars ───────────────────────────────
    let mut search_from = 0;
    while let Some(start) = result[search_from..].find("${") {
        let abs_start = search_from + start;
        if let Some(end) = result[abs_start..].find('}') {
            let var = &result[abs_start..abs_start + end + 1];
            warn!(var, "prompt_renderer: unresolved variable left in template");
            search_from = abs_start + end + 1;
        } else {
            break;
        }
    }

    // ── Collapse 3+ consecutive newlines to 2 ──────────────────────────────
    while result.contains(TRIPLE_NEWLINE) {
        result = result.replace(TRIPLE_NEWLINE, "\n\n");
    }

    result
}

// ── Helper: resolve memory vars from the in-memory cache ──────────────────

fn resolve_memory_vars(mut template: String, ctx: &VarContext) -> String {
    // `${memory/summary}` — one-line namespace summary.
    if template.contains("${memory/summary}") {
        let summary = if let (Some(mgr), Some(ns)) = (&ctx.memory_manager, &ctx.read_namespace) {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(mgr.get_summary(&ns.0))
            })
        } else {
            String::new()
        };
        template = template.replace("${memory/summary}", &summary);
    }

    // `${memory/<key>}` and `${memory/<ns>/<key>}` (distinguished by second slash).
    let mut new_template = String::with_capacity(template.len());
    let mut remaining = template.as_str();
    while let Some(start) = remaining.find("${memory/") {
        new_template.push_str(&remaining[..start]);
        let after = &remaining[start + 2..]; // skip "${"
        if let Some(end) = after.find('}') {
            let var_inner = &after[..end]; // "memory/<rest>"
            let rest = &var_inner["memory/".len()..]; // "<rest>"

            // Skip `summary` — already handled above.
            if rest == "summary" {
                new_template.push_str("${memory/summary}");
                remaining = &remaining[start + 2 + end + 1..];
                continue;
            }

            // If `rest` contains a `/`, it's a named-namespace read: `${memory/<ns>/<key>}`.
            let value = if let Some(slash) = rest.find('/') {
                let ns = &rest[..slash];
                let key = &rest[slash + 1..];
                if let Some(mgr) = &ctx.memory_manager {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(mgr.read(ns, key))
                    })
                    .map(|e| e.value.clone())
                    .unwrap_or_default()
                } else {
                    String::new()
                }
            } else {
                // Default-namespace read: `${memory/<key>}`.
                if let (Some(mgr), Some(ns)) = (&ctx.memory_manager, &ctx.read_namespace) {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(mgr.read(&ns.0, rest))
                    })
                    .map(|e| e.value.clone())
                    .unwrap_or_default()
                } else {
                    String::new()
                }
            };
            new_template.push_str(&value);
            remaining = &remaining[start + 2 + end + 1..];
        } else {
            // Malformed — pass through.
            new_template.push_str(&remaining[start..start + 2]);
            remaining = &remaining[start + 2..];
        }
    }
    new_template.push_str(remaining);
    new_template
}

// ── Helper: resolve only builtin vars (for user-var recursive expansion) ───

fn render_builtins_only(template: &str, ctx: &VarContext) -> String {
    let mut result = template.to_owned();
    if let Some(root) = &ctx.workspace_root {
        result = result.replace("${workspace/dir}", &root.to_string_lossy());
    }
    if let Some(name) = &ctx.workspace_name {
        result = result.replace("${workspace/name}", name);
    }
    if let Some(agent_name) = &ctx.agent_name {
        result = result.replace("${agent/name}", agent_name);
    }
    result = result.replace("${date}", &today_date());
    let tools_str = ctx.tools.join("\n");
    result = result.replace("${tools}", &tools_str);
    let agents_str = ctx.agents.join("\n");
    result = result.replace("${agents}", &agents_str);
    result
}

// ── Helper: today's date as YYYY-MM-DD ─────────────────────────────────────

fn today_date() -> String {
    // Use SystemTime for a dependency-free date string.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86400;
    // Simple Gregorian calendar implementation (accurate for years ≥ 1970).
    let mut year = 1970u32;
    let mut remaining_days = days as u32;
    loop {
        let leap = is_leap(year);
        let days_in_year = if leap { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days: [u32; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u32;
    for &md in &month_days {
        if remaining_days < md {
            break;
        }
        remaining_days -= md;
        month += 1;
    }
    let day = remaining_days + 1;
    format!("{year:04}-{month:02}-{day:02}")
}

fn is_leap(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

// ── Helper: resolve git branch via git2 ────────────────────────────────────

fn resolve_git_branch(workspace_root: &std::path::Path) -> Option<String> {
    git2::Repository::discover(workspace_root)
        .ok()
        .and_then(|repo| {
            repo.head()
                .ok()
                .and_then(|head| head.shorthand().map(str::to_owned))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ctx_with_workspace(name: &str) -> VarContext {
        VarContext {
            workspace_root: Some(PathBuf::from(format!("/home/user/{name}"))),
            workspace_name: Some(name.to_owned()),
            ..Default::default()
        }
    }

    #[test]
    fn renders_workspace_vars() {
        let ctx = ctx_with_workspace("myproject");
        let out = render("Root: ${workspace/dir}, Name: ${workspace/name}", &ctx);
        assert!(out.contains("/home/user/myproject"));
        assert!(out.contains("Name: myproject"));
    }

    #[test]
    fn renders_date() {
        let ctx = VarContext::default();
        let out = render("Date: ${date}", &ctx);
        // Date should be in YYYY-MM-DD format.
        assert!(out.contains("Date: 20"));
    }

    #[test]
    fn renders_tools_list() {
        let ctx = VarContext {
            tools: vec!["bash".to_owned(), "search".to_owned()],
            ..Default::default()
        };
        let out = render("Tools: ${tools}", &ctx);
        assert_eq!(out, "Tools: bash\nsearch");
    }

    #[test]
    fn unknown_var_passthrough() {
        let ctx = VarContext::default();
        let out = render("Hello ${unknown_var}", &ctx);
        assert!(out.contains("${unknown_var}"));
    }

    #[test]
    fn collapses_triple_newlines() {
        let ctx = VarContext::default();
        let out = render("a\n\n\n\nb", &ctx);
        assert_eq!(out, "a\n\nb");
    }

    #[test]
    fn user_var_with_builtin_expansion() {
        let ctx = VarContext {
            workspace_name: Some("proj".to_owned()),
            workspace_root: Some(PathBuf::from("/home/x/proj")),
            user_vars: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "greeting".to_owned(),
                    "Hello from ${workspace/name}".to_owned(),
                );
                m
            },
            ..Default::default()
        };
        let out = render("${greeting}", &ctx);
        assert_eq!(out, "Hello from proj");
    }

    #[test]
    fn empty_memory_summary() {
        // Without a memory manager, ${memory/summary} should resolve to "".
        let ctx = VarContext::default();
        let out = render("Summary: ${memory/summary}", &ctx);
        assert_eq!(out, "Summary: ");
    }
}
