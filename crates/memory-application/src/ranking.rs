use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::ProjectionHit;

const RRF_K: f64 = 60.0;
const LEXICAL_WEIGHT: f64 = 0.65;
const RETRIEVAL_WEIGHT: f64 = 0.35;
pub const MIN_TRANSPARENT_RELEVANCE_SCORE: f64 = 0.40;

pub fn lexical_relevance_score(query: &str, content: &str) -> f64 {
    let query = normalized_chars(query);
    let content = normalized_chars(content);
    if query.is_empty() || content.is_empty() {
        return 0.0;
    }
    let query_bigrams = ngrams(&query, 2);
    let content_bigrams = ngrams(&content, 2);
    let bigram_overlap = set_overlap_coefficient(&query_bigrams, &content_bigrams);
    let query_chars = query.iter().copied().collect::<HashSet<_>>();
    let content_chars = content.iter().copied().collect::<HashSet<_>>();
    let character_recall =
        query_chars.intersection(&content_chars).count() as f64 / query_chars.len() as f64;
    let exact_bonus = if content
        .windows(query.len())
        .any(|window| window == query.as_slice())
    {
        1.0
    } else {
        0.0
    };
    (0.55 * bigram_overlap + 0.30 * character_recall + 0.15 * exact_bonus).clamp(0.0, 1.0)
}

pub fn transparent_relevance_score(query: &str, content: &str, retrieval_score: f64) -> f64 {
    (LEXICAL_WEIGHT * lexical_relevance_score(query, content)
        + RETRIEVAL_WEIGHT * retrieval_score.clamp(0.0, 1.0))
    .clamp(0.0, 1.0)
}

pub fn reciprocal_rank_fusion(sources: &[Vec<ProjectionHit>], limit: usize) -> Vec<ProjectionHit> {
    let source_count = sources.len().max(1) as f64;
    let maximum_score = source_count / (RRF_K + 1.0);
    let mut fused = HashMap::<Uuid, (ProjectionHit, f64)>::new();
    for source in sources {
        let mut seen = HashSet::new();
        for (index, hit) in source.iter().enumerate() {
            if !seen.insert(hit.memory_version_id) {
                continue;
            }
            let contribution = 1.0 / (RRF_K + index as f64 + 1.0);
            let entry = fused
                .entry(hit.memory_version_id)
                .or_insert_with(|| (hit.clone(), 0.0));
            entry.1 += contribution;
        }
    }
    let mut hits = fused
        .into_values()
        .map(|(mut hit, score)| {
            hit.provider_score = (score / maximum_score).clamp(0.0, 1.0);
            hit
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .provider_score
            .total_cmp(&left.provider_score)
            .then_with(|| left.memory_record_id.cmp(&right.memory_record_id))
            .then_with(|| left.memory_version_id.cmp(&right.memory_version_id))
    });
    hits.truncate(limit);
    hits
}

fn normalized_chars(value: &str) -> Vec<char> {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|value| value.is_alphanumeric())
        .collect()
}

fn ngrams(value: &[char], width: usize) -> HashSet<Vec<char>> {
    if value.len() < width {
        return HashSet::from([value.to_vec()]);
    }
    value
        .windows(width)
        .map(<[char]>::to_vec)
        .collect::<HashSet<_>>()
}

fn set_overlap_coefficient<T: Eq + std::hash::Hash>(left: &HashSet<T>, right: &HashSet<T>) -> f64 {
    let denominator = left.len().min(right.len());
    if denominator == 0 {
        0.0
    } else {
        left.intersection(right).count() as f64 / denominator as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_score_is_language_agnostic_and_deterministic() {
        let exact = lexical_relevance_score("先讲原理", "回答技术问题时先讲原理，再给代码");
        let partial = lexical_relevance_score("先讲原理", "先说明核心原理");
        let irrelevant = lexical_relevance_score("先讲原理", "数据库使用 PostgreSQL");
        assert!(exact > partial);
        assert!(partial > irrelevant);
        assert_eq!(
            exact,
            lexical_relevance_score("先讲原理", "回答技术问题时先讲原理，再给代码")
        );
        assert_eq!(lexical_relevance_score("", "content"), 0.0);
    }

    #[test]
    fn transparent_score_can_override_bad_provider_order_with_exact_evidence() {
        let relevant =
            transparent_relevance_score("缓存问题先查权威账本", "回答缓存问题先查权威账本", 0.4);
        let irrelevant =
            transparent_relevance_score("缓存问题先查权威账本", "无关的数据库迁移说明", 1.0);
        assert!(relevant > irrelevant);
        assert!(relevant >= MIN_TRANSPARENT_RELEVANCE_SCORE);
        assert!(irrelevant < MIN_TRANSPARENT_RELEVANCE_SCORE);
    }

    #[test]
    fn rank_fusion_rewards_cross_source_agreement() {
        let shared_version = Uuid::new_v4();
        let provider_only_version = Uuid::new_v4();
        let local_only_version = Uuid::new_v4();
        let hit = |version_id: Uuid, provider_object_id: &str| ProjectionHit {
            provider_object_id: provider_object_id.into(),
            memory_record_id: Uuid::new_v4(),
            memory_version_id: version_id,
            head_revision: 0,
            generation: "generation".into(),
            content_hash: format!("sha256:{version_id}"),
            provider_score: 0.5,
        };
        let fused = reciprocal_rank_fusion(
            &[
                vec![
                    hit(provider_only_version, "provider-only"),
                    hit(shared_version, "provider-shared"),
                ],
                vec![
                    hit(local_only_version, "local-only"),
                    hit(shared_version, "local-shared"),
                ],
            ],
            10,
        );
        assert_eq!(fused[0].memory_version_id, shared_version);
        assert!(fused[0].provider_score > fused[1].provider_score);
        assert_eq!(fused.len(), 3);
    }
}
