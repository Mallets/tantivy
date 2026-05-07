use crate::schema::Field;
use crate::DocId;

/// Result of a custom phrase evaluator's adjacency check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhraseVerdict {
    /// The evaluator confirmed the phrase matches in this document.
    Match,
    /// The evaluator confirmed the phrase does NOT match in this document.
    NoMatch,
    /// The evaluator cannot determine the result for this document;
    /// the scorer should fall back to standard position-based matching.
    Unknown,
}

/// Trait for custom phrase matching logic that can override position-based evaluation.
///
/// When a `SegmentReader` provides a `PhraseEvaluator`, the `PhraseWeight` builds
/// an evaluator-based scorer that first consults `phrase_matches()`. Documents that
/// receive `PhraseVerdict::Unknown` are checked via the standard position-based path.
pub trait PhraseEvaluator: Send + Sync {
    /// Checks whether `doc_id` contains the given phrase terms in order.
    ///
    /// `phrase_terms` is a slice of `(offset, term_bytes)` pairs where `offset`
    /// is the position of the term within the phrase (0-based, may have gaps for
    /// slop queries).
    ///
    /// Return `Match` or `NoMatch` when the evaluator can determine the result,
    /// or `Unknown` to let the scorer fall back to position-based checking.
    fn phrase_matches(
        &self,
        field: Field,
        doc_id: DocId,
        phrase_terms: &[(usize, &[u8])],
        slop: u32,
    ) -> PhraseVerdict;
}
