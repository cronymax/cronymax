/**
 * tools.js — ToolBridge: executes tools via cefQuery tool.exec channel.
 *
 * Also holds the OpenAI tool definitions that the agent loop exposes to the
 * LLM (task 6.1).
 */

class ToolBridge {
  /**
   * Invoke a named tool with a JSON-string input.
   * @param {string} name
   * @param {string|object} input  JSON string or object
   * @returns {Promise<{ok:boolean, output?:string, error?:string}>}
   */
  async invoke(name, input) {
    const inputStr = typeof input === "string" ? input : JSON.stringify(input);
    const payload = JSON.stringify({ name, input: inputStr });
    try {
      const raw = await window.aiDesktop.send("tool.exec", payload);
      return JSON.parse(raw);
    } catch (e) {
      return { ok: false, error: String(e) };
    }
  }
}

// ---------------------------------------------------------------------------
// OpenAI tool definitions exposed to the LLM
// ---------------------------------------------------------------------------

const TOOL_DEFINITIONS = [
  {
    type: "function",
    function: {
      name: "file_read",
      description: "Read the contents of a file from the workspace.",
      parameters: {
        type: "object",
        properties: {
          path: {
            type: "string",
            description: "Absolute or workspace-relative file path.",
          },
        },
        required: ["path"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "file_write",
      description: "Write or overwrite a file in the workspace.",
      parameters: {
        type: "object",
        properties: {
          path: { type: "string", description: "File path to write." },
          content: { type: "string", description: "New file content." },
        },
        required: ["path", "content"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "terminal_exec",
      description:
        "Execute a shell command in the active terminal (sandboxed).",
      parameters: {
        type: "object",
        properties: {
          command: { type: "string", description: "Shell command to run." },
        },
        required: ["command"],
      },
    },
  },
  {
    type: "function",
    function: {
      name: "browser_get_active_page",
      description:
        "Return the URL and visible text content of the active browser tab.",
      parameters: { type: "object", properties: {} },
    },
  },
];

// Map from LLM tool name → native tool name
const TOOL_NAME_MAP = {
  file_read: "file.read",
  file_write: "file.write",
  terminal_exec: "terminal.execSandboxed",
  browser_get_active_page: null, // handled specially via browser.get_active_page
};

window.toolBridge = new ToolBridge();
window.TOOL_DEFINITIONS = TOOL_DEFINITIONS;
window.TOOL_NAME_MAP = TOOL_NAME_MAP;
