//! Application ports used by the memory service.

mod extractor;
mod provider;
mod ranking;
mod reranker;

pub use extractor::{
    ExtractCandidateProposal, ExtractMemoryRequest, ExtractorError, MemoryExtractor,
};
pub use provider::{
    IndexError, ProjectionDelete, ProjectionHit, ProjectionLookup, ProjectionReceipt,
    ProjectionSearch, ProjectionStatus, ProjectionUpsert, RetrievalIndex,
};
pub use ranking::{
    MIN_TRANSPARENT_RELEVANCE_SCORE, lexical_relevance_score, reciprocal_rank_fusion,
    transparent_relevance_score,
};
pub use reranker::{
    MemoryReranker, RerankCandidate, RerankRequest, RerankedCandidate, RerankerError,
    TransparentReranker, is_memory_applicable_without_semantic_threshold,
};
