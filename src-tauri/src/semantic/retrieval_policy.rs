pub(crate) const RRF_K: f64 = 60.0;
pub(crate) const RRF_LEXICAL_WEIGHT: f64 = 1.15;

pub(crate) fn hybrid_fusion_score(lexical_rank: Option<u32>, semantic_rank: Option<u32>) -> f64 {
    lexical_rank
        .map(|rank| RRF_LEXICAL_WEIGHT / (RRF_K + f64::from(rank)))
        .unwrap_or_default()
        + semantic_rank
            .map(|rank| 1.0 / (RRF_K + f64::from(rank)))
            .unwrap_or_default()
}

pub(crate) fn semantic_passage_score(scores: &[f64]) -> Option<f64> {
    scores.first().copied().map(|best| {
        let supporting = scores.get(1).copied().unwrap_or(best);
        best * 0.82 + supporting * 0.18
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_hybrid_policy_matches_milestone_one_constants() {
        let expected = 1.15 / 61.0 + 1.0 / 62.0;
        assert_eq!(hybrid_fusion_score(Some(1), Some(2)), expected);
        assert_eq!(hybrid_fusion_score(None, None), 0.0);
    }

    #[test]
    fn frozen_semantic_passage_policy_uses_best_two_scores() {
        assert_eq!(semantic_passage_score(&[]), None);
        assert_eq!(semantic_passage_score(&[0.8]), Some(0.8));
        assert_eq!(semantic_passage_score(&[0.8, 0.6]), Some(0.764));
    }
}
