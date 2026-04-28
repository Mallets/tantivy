use crate::schema::Field;
use crate::DocId;

/// Trait for custom phrase matching logic that overrides position-based evaluation.
///
/// When a `SegmentReader` provides a `PhraseEvaluator`, the `PhraseWeight` builds
/// an evaluator-based scorer that delegates phrase adjacency checks to
/// `phrase_matches()` for all documents.
pub trait PhraseEvaluator: Send + Sync {
    /// Returns `true` if `doc_id` contains the given phrase terms in order.
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
