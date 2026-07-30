//! Domain types and invariants for the LazyTT memory ledger.
//!
//! This crate intentionally has no HTTP, database, model, or Mem0 dependency.

mod candidate;
mod memory;
mod scope;

pub use candidate::{CandidateDecision, CandidateState, DecisionActor, DecisionOutcome};
pub use memory::{HeadState, MemoryHead, MemoryType};
pub use scope::{MemoryScope, ScopeKind};
