use async_trait::async_trait;
use memory_domain::{MemoryType, PreferenceApplicability};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::{
    MIN_TRANSPARENT_RELEVANCE_SCORE, query_has_temporal_intent, transparent_relevance_score,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankCandidate {
    pub memory_version_id: Uuid,
    pub content: String,
    pub memory_type: String,
    #[serde(default)]
    pub preference_applicability: PreferenceApplicability,
    pub retrieval_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankRequest {
    pub query: String,
    pub candidates: Vec<RerankCandidate>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankedCandidate {
    pub memory_version_id: Uuid,
    pub relevance_score: f64,
    pub reason_code: String,
}

#[derive(Debug, Error)]
pub enum RerankerError {
    #[error("reranker configuration is invalid: {0}")]
    Config(String),
    #[error("reranker request failed: {0}")]
    Transport(String),
    #[error("reranker response violated the contract: {0}")]
    InvalidResponse(String),
}

#[async_trait]
pub trait MemoryReranker: Send + Sync {
    fn version(&self) -> &str;

    async fn rerank(&self, request: RerankRequest)
    -> Result<Vec<RerankedCandidate>, RerankerError>;
}

/// Only explicitly global preferences govern answer behavior in every turn.
/// Unmarked and contextual preferences require topical relevance. An explicit request for the
/// user's profile, current project, or past experience must not be vetoed only
/// because a small reranker scores an indirect Chinese reference too low.
pub fn is_memory_applicable_without_semantic_threshold(
    query: &str,
    memory_type: &str,
    applicability: PreferenceApplicability,
) -> bool {
    if memory_type == MemoryType::Preference.as_str() {
        return applicability == PreferenceApplicability::Global;
    }
    let normalized = query.to_lowercase();
    match memory_type {
        value if value == MemoryType::ProfileFact.as_str() => {
            normalized.contains("我的")
                && [
                    "工作角色",
                    "角色",
                    "技术栈",
                    "编程语言",
                    "常用语言",
                    "技术背景",
                    "对我的了解",
                ]
                .iter()
                .any(|marker| normalized.contains(marker))
        }
        value if value == MemoryType::SemanticFact.as_str() => {
            normalized.contains("项目")
                && ["架构", "数据库", "权威存储", "权威事实", "项目事实"]
                    .iter()
                    .any(|marker| normalized.contains(marker))
        }
        value if value == MemoryType::EpisodicSummary.as_str() => {
            query_has_temporal_intent(&normalized)
                || [
                    "踩坑",
                    "过去经验",
                    "历史经验",
                    "以前处理",
                    "曾经处理",
                    "处理经验",
                ]
                .iter()
                .any(|marker| normalized.contains(marker))
        }
        _ => false,
    }
}

pub struct TransparentReranker;

#[async_trait]
impl MemoryReranker for TransparentReranker {
    fn version(&self) -> &str {
        "transparent-lexical-v2-preference-applicability"
    }

    async fn rerank(
        &self,
        request: RerankRequest,
    ) -> Result<Vec<RerankedCandidate>, RerankerError> {
        if request.query.trim().is_empty() || request.limit == 0 || request.limit > 50 {
            return Err(RerankerError::Config(
                "query must be non-empty and limit must be between 1 and 50".into(),
            ));
        }
        let mut ranked = request
            .candidates
            .into_iter()
            .filter_map(|candidate| {
                let relevance_score = transparent_relevance_score(
                    &request.query,
                    &candidate.content,
                    candidate.retrieval_score,
                );
                let intent_applicable = is_memory_applicable_without_semantic_threshold(
                    &request.query,
                    &candidate.memory_type,
                    candidate.preference_applicability,
                );
                (relevance_score >= MIN_TRANSPARENT_RELEVANCE_SCORE || intent_applicable).then(
                    || RerankedCandidate {
                        memory_version_id: candidate.memory_version_id,
                        relevance_score,
                        reason_code: if intent_applicable {
                            "TRANSPARENT_POLICY_APPLICABILITY"
                        } else {
                            "TRANSPARENT_LEXICAL_AND_RETRIEVAL"
                        }
                        .into(),
                    },
                )
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .relevance_score
                .total_cmp(&left.relevance_score)
                .then_with(|| left.memory_version_id.cmp(&right.memory_version_id))
        });
        ranked.truncate(request.limit);
        Ok(ranked)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn transparent_reranker_filters_irrelevant_candidates() {
        let relevant_id = Uuid::new_v4();
        let result = TransparentReranker
            .rerank(RerankRequest {
                query: "缓存问题先查权威账本".into(),
                candidates: vec![
                    RerankCandidate {
                        memory_version_id: Uuid::new_v4(),
                        content: "无关的数据库迁移说明".into(),
                        memory_type: "semantic_fact".into(),
                        preference_applicability: Default::default(),
                        retrieval_score: 1.0,
                    },
                    RerankCandidate {
                        memory_version_id: relevant_id,
                        content: "回答缓存问题先查权威账本".into(),
                        memory_type: "preference".into(),
                        preference_applicability: Default::default(),
                        retrieval_score: 0.4,
                    },
                ],
                limit: 5,
            })
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].memory_version_id, relevant_id);
    }

    #[tokio::test]
    async fn transparent_reranker_retains_an_explicit_global_preference_with_low_topical_score() {
        let preference_id = Uuid::new_v4();
        let result = TransparentReranker
            .rerank(RerankRequest {
                query: "解释 PostgreSQL Outbox".into(),
                candidates: vec![RerankCandidate {
                    memory_version_id: preference_id,
                    content: "回答时先给结论".into(),
                    memory_type: "preference".into(),
                    preference_applicability: PreferenceApplicability::Global,
                    retrieval_score: 0.0,
                }],
                limit: 5,
            })
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].memory_version_id, preference_id);
        assert_eq!(result[0].reason_code, "TRANSPARENT_POLICY_APPLICABILITY");
    }

    #[tokio::test]
    async fn contextual_preferences_do_not_bypass_relevance() {
        let result = TransparentReranker
            .rerank(RerankRequest {
                query: "解释 PostgreSQL Outbox".into(),
                candidates: vec![RerankCandidate {
                    memory_version_id: Uuid::new_v4(),
                    content: "食物里不要香菜".into(),
                    memory_type: "preference".into(),
                    preference_applicability: PreferenceApplicability::Contextual,
                    retrieval_score: 0.0,
                }],
                limit: 5,
            })
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn transparent_reranker_retains_explicitly_requested_memory_categories() {
        let result = TransparentReranker
            .rerank(RerankRequest {
                query: "结合我的工作角色、项目权威数据库和以前处理 Outbox 的经验回答".into(),
                candidates: vec![
                    RerankCandidate {
                        memory_version_id: Uuid::new_v4(),
                        content: "用户是Agent研发工程师。".into(),
                        memory_type: "profile_fact".into(),
                        preference_applicability: Default::default(),
                        retrieval_score: 0.0,
                    },
                    RerankCandidate {
                        memory_version_id: Uuid::new_v4(),
                        content: "当前项目使用PostgreSQL作为权威数据库。".into(),
                        memory_type: "semantic_fact".into(),
                        preference_applicability: Default::default(),
                        retrieval_score: 0.0,
                    },
                    RerankCandidate {
                        memory_version_id: Uuid::new_v4(),
                        content: "用户曾通过event_id去重解决Outbox重复消费。".into(),
                        memory_type: "episodic_summary".into(),
                        preference_applicability: Default::default(),
                        retrieval_score: 0.0,
                    },
                ],
                limit: 5,
            })
            .await
            .unwrap();

        assert_eq!(result.len(), 3);
        assert!(
            result
                .iter()
                .all(|item| item.reason_code == "TRANSPARENT_POLICY_APPLICABILITY")
        );
    }

    #[tokio::test]
    async fn transparent_reranker_retains_temporally_selected_episode() {
        let episode_id = Uuid::new_v4();
        let result = TransparentReranker
            .rerank(RerankRequest {
                query: "昨天处理了什么？".into(),
                candidates: vec![RerankCandidate {
                    memory_version_id: episode_id,
                    content: "修复了队列重复消费".into(),
                    memory_type: "episodic_summary".into(),
                    preference_applicability: Default::default(),
                    retrieval_score: 1.0,
                }],
                limit: 5,
            })
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].memory_version_id, episode_id);
        assert_eq!(result[0].reason_code, "TRANSPARENT_POLICY_APPLICABILITY");
    }
}
