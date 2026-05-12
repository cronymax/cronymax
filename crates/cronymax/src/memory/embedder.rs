//! Embedding support for the memory manager.
//!
//! The `Embedder` trait is intentionally minimal — the runtime only needs to
//! check whether embeddings are available (for the hybrid BM25+cosine path)
//! and to generate embedding vectors for indexing.
//!
//! The default implementation is `NoopEmbedder`, which signals that the pure
//! BM25 path should be used. A `ProviderEmbedder` backed by the active LLM
//! API is planned for a follow-up (deferred to task 3.3 extensions).

use async_trait::async_trait;

/// An embedding vector (f32 for compact storage).
pub type EmbeddingVec = Vec<f32>;

/// Compute semantic embeddings for text fragments.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Returns `true` if this embedder can produce real vectors. When
    /// `false`, callers should fall back to BM25-only ranking.
    fn is_available(&self) -> bool;

    /// Generate an embedding vector for a single piece of text.
    ///
    /// Returns `None` when `is_available()` is `false`.
    async fn embed(&self, text: &str) -> Option<EmbeddingVec>;
}

/// A no-op embedder that signals "no embeddings available". This is the
/// default. All ranking is done via BM25 in this mode.
#[derive(Debug, Default)]
pub struct NoopEmbedder;

#[async_trait]
impl Embedder for NoopEmbedder {
    fn is_available(&self) -> bool {
        false
    }

    async fn embed(&self, _text: &str) -> Option<EmbeddingVec> {
        None
    }
}
