//! Domain types and invariants for the LazyTT memory ledger.
//!
//! This crate intentionally has no HTTP, database, model, or Mem0 dependency.

mod candidate;
mod memory;
mod scope;

pub use candidate::{CandidateDecision, CandidateState, DecisionActor, DecisionOutcome};
pub use memory::{
    CANONICAL_MEMORY_TYPE_NAMES, HeadState, MEMORY_TAXONOMY_VERSION, MemoryFamily, MemoryHead,
    MemoryType, MemoryTypeError, MemoryTypeMapping,
};
pub use scope::{MemoryScope, ScopeKind};
