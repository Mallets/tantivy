use std::cmp::Ordering;
use std::sync::Arc;

use crate::docset::{DocSet, TERMINATED};
use crate::fieldnorm::FieldNormReader;
use crate::postings::Postings;
use crate::query::bm25::Bm25Weight;
use crate::query::{Explanation, Intersection, Scorer};
use crate::schema::Field;
use crate::{DocId, Score};

use super::phrase_evaluator::PhraseEvaluator;

struct PostingsWithOffset<TPostings> {
    offset: u32,
    postings: TPostings,
}

impl<TPostings: Postings> PostingsWithOffset<TPostings> {
    fn new(postings: TPostings, offset: u32) -> Self {
        Self { offset, postings }
    }

    fn positions(&mut self, output: &mut Vec<u32>) {
        self.postings.positions_with_offset(self.offset, output)
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

/// Hybrid phrase scorer: delegates adjacency to the evaluator for templated docs
/// and falls back to standard position intersection for outlier docs.
pub(crate) struct EvaluatorPhraseScorer<TPostings: Postings> {
    intersection_docset:
        Intersection<PostingsWithOffset<TPostings>, PostingsWithOffset<TPostings>>,
    num_terms: usize,
    evaluator: Arc<dyn PhraseEvaluator>,
    field: Field,
    phrase_terms: Vec<(usize, Vec<u8>)>,
    slop: u32,
    left_positions: Vec<u32>,
    right_positions: Vec<u32>,
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
        let max_offset = term_postings_with_offset
            .iter()
            .map(|&(offset, _)| offset)
            .max()
            .unwrap_or(0);
        let num_terms = term_postings_with_offset.len();
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
            left_positions: Vec::with_capacity(100),
            right_positions: Vec::with_capacity(100),
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
        let doc = self.intersection_docset.doc();
        if doc < self.evaluator.templated_doc_count() {
            let matched = self.evaluator_matches();
            self.phrase_count = u32::from(matched);
            matched
        } else {
            self.position_match()
        }
    }

    fn evaluator_matches(&self) -> bool {
        let doc_id = self.intersection_docset.doc();
        let phrase_term_refs: Vec<(usize, &[u8])> = self
            .phrase_terms
            .iter()
            .map(|(offset, bytes)| (*offset, bytes.as_slice()))
            .collect();
        self.evaluator
            .phrase_matches(self.field, doc_id, &phrase_term_refs, self.slop)
    }

    fn position_match(&mut self) -> bool {
        if self.similarity_weight_opt.is_some() {
            let count = self.compute_phrase_count();
            self.phrase_count = count;
            count > 0
        } else {
            self.phrase_exists()
        }
    }

    fn phrase_exists(&mut self) -> bool {
        self.compute_phrase_match();
        if self.slop > 0 {
            intersection_exists_with_slop(
                &self.left_positions,
                &self.right_positions,
                self.slop,
            )
        } else {
            intersection_exists(&self.left_positions, &self.right_positions)
        }
    }

    fn compute_phrase_count(&mut self) -> u32 {
        self.compute_phrase_match();
        if self.slop > 0 {
            intersection_count_with_slop(
                &mut self.left_positions,
                &self.right_positions,
                self.slop,
                false,
            ) as u32
        } else {
            intersection_count(&self.left_positions, &self.right_positions) as u32
        }
    }

    fn compute_phrase_match(&mut self) {
        self.intersection_docset
            .docset_mut_specialized(0)
            .positions(&mut self.left_positions);
        for i in 1..self.num_terms - 1 {
            self.intersection_docset
                .docset_mut_specialized(i)
                .positions(&mut self.right_positions);
            if self.slop > 0 {
                intersection_count_with_slop(
                    &mut self.left_positions,
                    &self.right_positions,
                    self.slop,
                    true,
                );
            } else {
                intersection(&mut self.left_positions, &self.right_positions);
            }
            if self.left_positions.is_empty() {
                return;
            }
        }
        self.intersection_docset
            .docset_mut_specialized(self.num_terms - 1)
            .positions(&mut self.right_positions);
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

fn intersection_exists(left: &[u32], right: &[u32]) -> bool {
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.len() && right_index < right.len() {
        let left_val = left[left_index];
        let right_val = right[right_index];
        match left_val.cmp(&right_val) {
            Ordering::Less => left_index += 1,
            Ordering::Equal => return true,
            Ordering::Greater => right_index += 1,
        }
    }
    false
}

fn intersection(left: &mut Vec<u32>, right: &[u32]) {
    let mut left_index = 0;
    let mut right_index = 0;
    let mut count = 0;
    let left_len = left.len();
    let right_len = right.len();
    while left_index < left_len && right_index < right_len {
        let left_val = left[left_index];
        let right_val = right[right_index];
        match left_val.cmp(&right_val) {
            Ordering::Less => left_index += 1,
            Ordering::Equal => {
                left[count] = left_val;
                count += 1;
                left_index += 1;
                right_index += 1;
            }
            Ordering::Greater => right_index += 1,
        }
    }
    left.truncate(count);
}

fn intersection_count(left: &[u32], right: &[u32]) -> usize {
    let mut left_index = 0;
    let mut right_index = 0;
    let mut count = 0;
    while left_index < left.len() && right_index < right.len() {
        let left_val = left[left_index];
        let right_val = right[right_index];
        match left_val.cmp(&right_val) {
            Ordering::Less => left_index += 1,
            Ordering::Equal => {
                count += 1;
                left_index += 1;
                right_index += 1;
            }
            Ordering::Greater => right_index += 1,
        }
    }
    count
}

fn intersection_exists_with_slop(left: &[u32], right: &[u32], slop: u32) -> bool {
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.len() && right_index < right.len() {
        let left_val = left[left_index];
        let right_val = right[right_index];
        if left_val.abs_diff(right_val) <= slop {
            return true;
        } else if left_val < right_val {
            left_index += 1;
        } else {
            right_index += 1;
        }
    }
    false
}

#[inline]
fn intersection_count_with_slop(
    left: &mut Vec<u32>,
    right: &[u32],
    slop: u32,
    update_left: bool,
) -> usize {
    let mut left_index = 0;
    let mut right_index = 0;
    let mut count = 0;
    let left_len = left.len();
    let right_len = right.len();
    while left_index < left_len && right_index < right_len {
        let left_val = left[left_index];
        let right_val = right[right_index];
        let distance = left_val.abs_diff(right_val);
        if distance <= slop {
            while left_index + 1 < left_len {
                let next_left_val = left[left_index + 1];
                if next_left_val > right_val {
                    break;
                }
                left_index += 1;
            }
            if update_left {
                left[count] = right_val;
            }
            count += 1;
            left_index += 1;
            right_index += 1;
        } else if left_val < right_val {
            left_index += 1;
        } else {
            right_index += 1;
        }
    }
    if update_left {
        left.truncate(count);
    }
    count
}
