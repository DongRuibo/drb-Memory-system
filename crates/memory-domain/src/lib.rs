//! Domain types and invariants for the LazyTT memory ledger.
//!
//! This crate intentionally has no HTTP, database, model, or Mem0 dependency.

mod candidate;
mod event_time;
mod memory;
mod scope;
mod structured_content;

pub use candidate::{CandidateDecision, CandidateState, DecisionActor, DecisionOutcome};
pub use event_time::{EventTime, EventTimeError, EventTimePrecision};
pub use memory::{
    CANONICAL_MEMORY_TYPE_NAMES, HeadState, MEMORY_TAXONOMY_VERSION, MemoryFamily, MemoryHead,
    MemoryType, MemoryTypeError, MemoryTypeMapping,
};
pub use scope::{MemoryScope, ScopeKind};
pub use structured_content::{
    MAX_STRUCTURED_CONTENT_BYTES, STRUCTURED_CONTENT_VERSION, StructuredContent,
    StructuredContentError, schema_name_for,
};
