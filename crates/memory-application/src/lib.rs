//! Application ports used by the memory service.

mod extractor;
mod provider;
mod ranking;
mod reranker;
mod security;
mod temporal;
mod text;

pub use extractor::{
    ExtractCandidateProposal, ExtractMemoryOutput, ExtractMemoryRequest, ExtractionDiagnostics,
    ExtractorError, MemoryExtractor,
};
pub use provider::{
    IndexError, ProjectionDelete, ProjectionHit, ProjectionLookup, ProjectionReceipt,
    ProjectionSearch, ProjectionStatus, ProjectionUpsert, RetrievalIndex,
};
pub use ranking::{
    MIN_TRANSPARENT_RELEVANCE_SCORE, lexical_relevance_score, reciprocal_rank_contribution,
    reciprocal_rank_fusion, transparent_relevance_score, weighted_reciprocal_rank_fusion,
};
pub use reranker::{
    MemoryReranker, RerankCandidate, RerankRequest, RerankedCandidate, RerankerError,
    TransparentReranker, is_memory_applicable_without_semantic_threshold,
};
pub use security::{
    SECURITY_POLICY_VERSION, SanitizedValue, SecurityAction, SecurityFinding, SecurityPipeline,
};
pub use temporal::{
    TemporalError, TemporalWindow, parse_temporal_query, query_has_temporal_intent,
};
pub use text::{CHINESE_TOKENIZER_VERSION, ChineseTokenizer, normalize_text};
