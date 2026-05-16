//! LLM provider abstraction (task 5.2).
//!
//! The runtime owns the agent loop, so it also owns the LLM HTTP
//! contract. This module defines the provider-facing abstraction:
//!
//! * [`messages`] — chat history primitives + tool definitions.
//! * [`provider`] — the [`provider::LlmProvider`] trait + streaming
//!   event types.
//! * [`openai`] — concrete OpenAI-chat-compatible streaming client
//!   (`POST /chat/completions` with `stream: true` + tool calls).
//! * [`anthropic`] — native Anthropic Messages API streaming client
//!   (`POST /v1/messages` with SSE content blocks).
//! * [`capabilities`] — dynamic model capability probing + cache +
//!   static fallback table for `ThinkingSupport` resolution.
//! * [`mock`] — scripted in-process provider used by tests and as a
//!   safe default when no real provider is configured.
//!
//! The agent loop in [`crate::agent_loop`] consumes
//! [`provider::LlmEvent`] streams; nothing else in the crate touches
//! the wire format.

pub mod anthropic;
pub mod capabilities;
pub mod config;
pub mod copilot_auth;
pub mod copilot_cache;
pub mod factory;
pub mod messages;
pub mod migration;
pub mod mock;
pub mod openai;
pub mod provider;
pub mod registry;

pub use config::LlmConfig;
pub use factory::{DefaultLlmProviderFactory, LlmProviderFactory};
mod stream;

pub use anthropic::{AnthropicConfig, AnthropicProvider, ANTHROPIC_API_VERSION};
pub use capabilities::{CapabilityResolver, ModelCapabilities, ThinkingSupport};
pub use messages::{
    ChatMessage, ChatRole, FinishReason, LlmRequest, ThinkingConfig, ToolCall, ToolDef,
};
pub use mock::{MockLlmFactory, MockLlmProvider, MockScript, ScriptStep};
pub use openai::{OpenAiConfig, OpenAiProvider};
pub use provider::{LlmEvent, LlmProvider, LlmStream};
pub use registry::{LlmProviderEntry, LlmProviderKind, LlmProviderRegistry};
