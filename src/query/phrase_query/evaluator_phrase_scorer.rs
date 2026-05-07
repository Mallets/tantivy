use std::sync::Arc;

use crate::docset::{DocSet, TERMINATED};
use crate::fieldnorm::FieldNormReader;
use crate::postings::Postings;
use crate::query::bm25::Bm25Weight;
use crate::query::{Explanation, Intersection, Scorer};
use crate::schema::Field;
use crate::{DocId, Score};

use super::phrase_evaluator::{PhraseEvaluator, PhraseVerdict};
use super::phrase_scorer::{intersection, intersection_exists};

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

/// Phrase scorer that first delegates adjacency checks to a `PhraseEvaluator`.
///
/// When the evaluator returns `PhraseVerdict::Unknown` (e.g. for outlier docs),
/// the scorer falls back to standard position-based phrase matching using the
/// loaded postings.
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
    left_positions: Vec<u32>,
    right_positions: Vec<u32>,
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
            left_positions: Vec::with_capacity(100),
            right_positions: Vec::with_capacity(100),
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
            PhraseVerdict::NoMatch => {
                self.phrase_count = 0;
                false
            }
            PhraseVerdict::Unknown => {
                let matched = self.position_based_phrase_match();
                self.phrase_count = u32::from(matched);
                matched
            }
        }
    }

    /// Standard position-based phrase adjacency check, used as fallback when
    /// the evaluator returns `Unknown`.
    fn position_based_phrase_match(&mut self) -> bool {
        if self.num_terms < 2 {
            return true;
        }
        let first = self.intersection_docset.docset_mut_specialized(0);
        first
            .postings
            .positions_with_offset(first.offset, &mut self.left_positions);

        for i in 1..self.num_terms - 1 {
            let ds = self.intersection_docset.docset_mut_specialized(i);
            ds.postings
                .positions_with_offset(ds.offset, &mut self.right_positions);
            intersection(&mut self.left_positions, &self.right_positions);
            if self.left_positions.is_empty() {
                return false;
            }
        }

        let last = self
            .intersection_docset
            .docset_mut_specialized(self.num_terms - 1);
        last.postings
            .positions_with_offset(last.offset, &mut self.right_positions);

        intersection_exists(&self.left_positions, &self.right_positions)
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
