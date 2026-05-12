//! BM25 search for in-memory namespace entries.
//!
//! `Bm25Index` is rebuilt whenever the namespace is loaded or an entry is
//! written. Scores use the standard BM25 formula with `k1 = 1.5`, `b = 0.75`.

use std::collections::HashMap;

/// A single ranked search result.
#[derive(Debug, Clone)]
pub struct RankedResult {
    /// The entry key.
    pub key: String,
    /// BM25 relevance score (higher is more relevant).
    pub score: f64,
}

/// Tokenise a piece of text into normalised terms.
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_ascii_lowercase())
        .collect()
}

/// In-memory BM25 index over a flat collection of text entries.
///
/// Rebuild by calling [`Bm25Index::build`]. The index is rebuilt on every
/// write so query latency reflects the latest namespace state. Namespaces
/// are small enough (< 1 k entries) that the rebuild cost is negligible.
#[derive(Debug, Default)]
pub struct Bm25Index {
    /// `doc_id` → `{ term → frequency }`
    term_freqs: HashMap<String, HashMap<String, usize>>,
    /// `term → number of documents containing it`
    doc_freqs: HashMap<String, usize>,
    /// Per-document token counts (length).
    doc_lengths: HashMap<String, usize>,
    /// Corpus average document length.
    avg_dl: f64,
    /// Total number of indexed documents.
    n_docs: usize,
}

const K1: f64 = 1.5;
const B: f64 = 0.75;

impl Bm25Index {
    /// Build an index from `(key, text)` pairs.
    pub fn build<'a>(entries: impl Iterator<Item = (&'a str, &'a str)>) -> Self {
        let mut term_freqs: HashMap<String, HashMap<String, usize>> = HashMap::new();
        let mut doc_lengths: HashMap<String, usize> = HashMap::new();

        for (key, text) in entries {
            let tokens = tokenize(text);
            let len = tokens.len();
            doc_lengths.insert(key.to_owned(), len);
            let tf = term_freqs.entry(key.to_owned()).or_default();
            for tok in tokens {
                *tf.entry(tok).or_insert(0) += 1;
            }
        }

        let n_docs = doc_lengths.len();
        let total_len: usize = doc_lengths.values().sum();
        let avg_dl = if n_docs == 0 {
            0.0
        } else {
            total_len as f64 / n_docs as f64
        };

        let mut doc_freqs: HashMap<String, usize> = HashMap::new();
        for tf in term_freqs.values() {
            for term in tf.keys() {
                *doc_freqs.entry(term.clone()).or_insert(0) += 1;
            }
        }

        Self { term_freqs, doc_freqs, doc_lengths, avg_dl, n_docs }
    }

    /// Rank all indexed documents against `query`. Returns results sorted by
    /// descending BM25 score. Documents with score ≤ 0 are omitted.
    pub fn search(&self, query: &str) -> Vec<RankedResult> {
        if self.n_docs == 0 {
            return vec![];
        }
        let query_terms = tokenize(query);
        let mut scores: HashMap<&str, f64> = HashMap::new();

        for term in &query_terms {
            let df = *self.doc_freqs.get(term).unwrap_or(&0);
            if df == 0 {
                continue;
            }
            let idf = ((self.n_docs as f64 - df as f64 + 0.5) / (df as f64 + 0.5) + 1.0).ln();
            for (doc_key, tf_map) in &self.term_freqs {
                let tf = *tf_map.get(term).unwrap_or(&0);
                if tf == 0 {
                    continue;
                }
                let dl = *self.doc_lengths.get(doc_key.as_str()).unwrap_or(&0);
                let norm = 1.0 - B + B * dl as f64 / self.avg_dl.max(1.0);
                let tf_score = tf as f64 * (K1 + 1.0) / (tf as f64 + K1 * norm);
                *scores.entry(doc_key.as_str()).or_insert(0.0) += idf * tf_score;
            }
        }

        let mut results: Vec<RankedResult> = scores
            .into_iter()
            .filter(|(_, s)| *s > 0.0)
            .map(|(key, score)| RankedResult { key: key.to_owned(), score })
            .collect();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_ranking() {
        let entries = vec![
            ("doc1", "the quick brown fox jumps over the lazy dog"),
            ("doc2", "a quick fox"),
            ("doc3", "something completely different"),
        ];
        let idx = Bm25Index::build(entries.iter().map(|(k, v)| (*k, *v)));
        let results = idx.search("quick fox");
        assert!(!results.is_empty());
        // doc2 has higher density of query terms
        assert_eq!(results[0].key, "doc2");
    }

    #[test]
    fn empty_index() {
        let idx = Bm25Index::build(std::iter::empty());
        assert!(idx.search("anything").is_empty());
    }
}
