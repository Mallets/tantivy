use crate::schema::Field;
use crate::DocId;

/// Trait for custom phrase matching logic that overrides position-based evaluation
/// for a subset of documents.
///
/// When a `SegmentReader` provides a `PhraseEvaluator`, the `PhraseWeight` builds
/// a hybrid scorer: docs below `templated_doc_count()` delegate adjacency to
/// `phrase_matches()`; docs at or above that threshold fall back to standard
/// position-intersection logic.
pub trait PhraseEvaluator: Send + Sync {
    /// Number of documents covered by the evaluator.
    /// Docs with `doc_id >= templated_doc_count()` use standard position matching.
    fn templated_doc_count(&self) -> u32;

    /// Returns `true` if `doc_id` contains the given phrase terms in order.
    ///
    /// Only called for `doc_id < templated_doc_count()`.
    ///
    /// `phrase_terms` is a slice of `(offset, term_bytes)` pairs where `offset`
    /// is the position of the term within the phrase (0-based, may have gaps for
    /// slop queries).
    fn phrase_matches(
        &self,
        field: Field,
        doc_id: DocId,
        phrase_terms: &[(usize, &[u8])],
        slop: u32,
    ) -> bool;
}
