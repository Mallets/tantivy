use std::sync::Arc;

use crate::docset::{DocSet, TERMINATED};
use crate::fieldnorm::FieldNormReader;
use crate::postings::Postings;
use crate::query::bm25::Bm25Weight;
use crate::query::{Explanation, Intersection, Scorer};
use crate::schema::Field;
use crate::{DocId, Score};

use super::phrase_evaluator::{PhraseEvaluator, PhraseVerdict};

struct PostingsWithOffset<TPostings> {
    postings: TPostings,
    offset: u32,
}

impl<TPostings: Postings> PostingsWithOffset<TPostings> {
    fn new(postings: TPostings, offset: u32) -> Self {
        Self { postings, offset }
    }
}

impl<TPostings: Postings> DocSet for PostingsWithOffset<TPostings> {
    fn advance(&mut self) -> DocId {
        self.postings.advance()
    }

    fn seek(&mut self, target: DocId) -> DocId {
        self.postings.seek(target)
    }

    fn doc(&self) -> DocId {
        self.postings.doc()
    }

    fn size_hint(&self) -> u32 {
        self.postings.size_hint()
    }
}

/// Phrase scorer that delegates adjacency checks to a `PhraseEvaluator`.
///
/// Postings are loaded as `Basic` (doc IDs only, no positions on disk). All
/// phrase verification is handled by the evaluator. If the evaluator returns
/// `PhraseVerdict::Unknown`, the document is treated as a non-match.
pub(crate) struct EvaluatorPhraseScorer<TPostings: Postings> {
    intersection_docset:
        Intersection<PostingsWithOffset<TPostings>, PostingsWithOffset<TPostings>>,
    num_terms: usize,
    evaluator: Arc<dyn PhraseEvaluator>,
    field: Field,
    phrase_terms: Vec<(usize, Vec<u8>)>,
    slop: u32,
    phrase_count: u32,
    fieldnorm_reader: FieldNormReader,
    similarity_weight_opt: Option<Bm25Weight>,
}

impl<TPostings: Postings> EvaluatorPhraseScorer<TPostings> {
    pub(crate) fn new(
        term_postings_with_offset: Vec<(usize, TPostings)>,
        evaluator: Arc<dyn PhraseEvaluator>,
        field: Field,
        phrase_terms: Vec<(usize, Vec<u8>)>,
        similarity_weight_opt: Option<Bm25Weight>,
        fieldnorm_reader: FieldNormReader,
        slop: u32,
    ) -> Self {
        let num_docs = fieldnorm_reader.num_docs();
        let num_terms = term_postings_with_offset.len();
        let max_offset = term_postings_with_offset
            .iter()
            .map(|(offset, _)| *offset)
            .max()
            .unwrap_or(0);
        let postings_with_offsets: Vec<PostingsWithOffset<TPostings>> =
            term_postings_with_offset
                .into_iter()
                .map(|(offset, postings)| {
                    PostingsWithOffset::new(postings, (max_offset - offset) as u32)
                })
                .collect();
        let intersection_docset = Intersection::new(postings_with_offsets, num_docs);
        let mut scorer = EvaluatorPhraseScorer {
            intersection_docset,
            num_terms,
            evaluator,
            field,
            phrase_terms,
            slop,
            phrase_count: 0,
            fieldnorm_reader,
            similarity_weight_opt,
        };
        if scorer.doc() != TERMINATED && !scorer.phrase_match() {
            scorer.advance();
        }
        scorer
    }

    fn phrase_match(&mut self) -> bool {
        let doc_id = self.intersection_docset.doc();
        let phrase_term_refs: Vec<(usize, &[u8])> = self
            .phrase_terms
            .iter()
            .map(|(offset, bytes)| (*offset, bytes.as_slice()))
            .collect();
        match self
            .evaluator
            .phrase_matches(self.field, doc_id, &phrase_term_refs, self.slop)
        {
            PhraseVerdict::Match => {
                self.phrase_count = 1;
                true
            }
            PhraseVerdict::NoMatch | PhraseVerdict::Unknown => {
                self.phrase_count = 0;
                false
            }
        }
    }
}

impl<TPostings: Postings> DocSet for EvaluatorPhraseScorer<TPostings> {
    fn advance(&mut self) -> DocId {
        loop {
            let doc = self.intersection_docset.advance();
            if doc == TERMINATED || self.phrase_match() {
                return doc;
            }
        }
    }

    fn seek(&mut self, target: DocId) -> DocId {
        let doc = self.intersection_docset.seek(target);
        if doc == TERMINATED || self.phrase_match() {
            return doc;
        }
        self.advance()
    }

    fn doc(&self) -> DocId {
        self.intersection_docset.doc()
    }

    fn size_hint(&self) -> u32 {
        self.intersection_docset.size_hint() / (10 * self.num_terms as u32)
    }
}

impl<TPostings: Postings> Scorer for EvaluatorPhraseScorer<TPostings> {
    fn score(&mut self) -> Score {
        let doc = self.doc();
        let fieldnorm_id = self.fieldnorm_reader.fieldnorm_id(doc);
        if let Some(similarity_weight) = self.similarity_weight_opt.as_ref() {
            similarity_weight.score(fieldnorm_id, self.phrase_count)
        } else {
            1.0f32
        }
    }

    fn explain(&mut self) -> Explanation {
        Explanation::new("EvaluatorPhraseScorer", self.score())
    }
}
