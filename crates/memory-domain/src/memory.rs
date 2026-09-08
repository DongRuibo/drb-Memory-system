use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Preference,
    ProfileFact,
    SemanticFact,
    EpisodicSummary,
}

/// Cognitive family of a governed, canonical memory.
///
/// `MemoryType` remains the stable policy-facing classification because preference, profile, and
/// general facts have different allowed effects and privacy rules. `MemoryFamily` makes their
/// cognitive relationship explicit without flattening Working Memory or executable procedures
/// into the canonical memory ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryFamily {
    Semantic,
    Episodic,
}

pub const CANONICAL_MEMORY_TYPE_NAMES: [&str; 4] = [
    "preference",
    "profile_fact",
    "semantic_fact",
    "episodic_summary",
];
pub const MEMORY_TAXONOMY_VERSION: &str = "memory-taxonomy-v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryTypeMapping {
    Canonical(MemoryType),
    Alias(MemoryType),
    ProceduralCandidate,
}

impl MemoryType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::ProfileFact => "profile_fact",
            Self::SemanticFact => "semantic_fact",
            Self::EpisodicSummary => "episodic_summary",
        }
    }

    pub const fn family(self) -> MemoryFamily {
        match self {
            Self::Preference | Self::ProfileFact | Self::SemanticFact => MemoryFamily::Semantic,
            Self::EpisodicSummary => MemoryFamily::Episodic,
        }
    }

    /// Maps both projects' historical names into the unified taxonomy.
    ///
    /// A procedural proposal is deliberately not returned as a canonical Memory Type: it must
    /// remain in a separate review or Skill/Procedure pipeline and can never gain execution
    /// authority by being stored as memory.
    pub fn map_input(value: &str) -> Result<MemoryTypeMapping, MemoryTypeError> {
        match value {
            "preference" => Ok(MemoryTypeMapping::Canonical(Self::Preference)),
            "profile_fact" => Ok(MemoryTypeMapping::Canonical(Self::ProfileFact)),
            "semantic_fact" => Ok(MemoryTypeMapping::Canonical(Self::SemanticFact)),
            "episodic_summary" => Ok(MemoryTypeMapping::Canonical(Self::EpisodicSummary)),
            "project_fact" | "semantic" => Ok(MemoryTypeMapping::Alias(Self::SemanticFact)),
            "profile" => Ok(MemoryTypeMapping::Alias(Self::ProfileFact)),
            "episodic" => Ok(MemoryTypeMapping::Alias(Self::EpisodicSummary)),
            "procedural" | "procedural_candidate" => Ok(MemoryTypeMapping::ProceduralCandidate),
            _ => Err(MemoryTypeError::Unknown(value.to_owned())),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MemoryTypeError {
    #[error("unknown memory type {0:?}")]
    Unknown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadState {
    Active,
    Contested,
    Expired,
    Forgotten,
    Erased,
}

impl HeadState {
    pub fn is_readable(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryHead {
    pub record_id: Uuid,
    pub current_version_id: Option<Uuid>,
    pub state: HeadState,
    pub revision: u64,
}

impl MemoryHead {
    pub fn activate(
        &mut self,
        expected_revision: u64,
        version_id: Uuid,
    ) -> Result<(), HeadTransitionError> {
        self.guard_revision(expected_revision)?;
        if matches!(self.state, HeadState::Forgotten | HeadState::Erased) {
            return Err(HeadTransitionError::TerminalState(self.state));
        }

        self.current_version_id = Some(version_id);
        self.state = HeadState::Active;
        self.revision += 1;
        Ok(())
    }

    pub fn contest(&mut self, expected_revision: u64) -> Result<(), HeadTransitionError> {
        self.guard_revision(expected_revision)?;
        if matches!(self.state, HeadState::Forgotten | HeadState::Erased) {
            return Err(HeadTransitionError::TerminalState(self.state));
        }

        self.state = HeadState::Contested;
        self.revision += 1;
        Ok(())
    }

    pub fn forget(&mut self, expected_revision: u64) -> Result<(), HeadTransitionError> {
        self.guard_revision(expected_revision)?;
        if self.state == HeadState::Erased {
            return Err(HeadTransitionError::TerminalState(self.state));
        }

        self.state = HeadState::Forgotten;
        self.revision += 1;
        Ok(())
    }

    fn guard_revision(&self, expected: u64) -> Result<(), HeadTransitionError> {
        if self.revision != expected {
            return Err(HeadTransitionError::RevisionConflict {
                expected,
                actual: self.revision,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HeadTransitionError {
    #[error("head revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("head state {0:?} is terminal for this operation")]
    TerminalState(HeadState),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_head() -> MemoryHead {
        MemoryHead {
            record_id: Uuid::new_v4(),
            current_version_id: Some(Uuid::new_v4()),
            state: HeadState::Active,
            revision: 4,
        }
    }

    #[test]
    fn concurrent_revision_has_single_winner() {
        let mut head = active_head();
        head.activate(4, Uuid::new_v4()).expect("first writer wins");

        assert_eq!(
            head.activate(4, Uuid::new_v4()),
            Err(HeadTransitionError::RevisionConflict {
                expected: 4,
                actual: 5,
            })
        );
    }

    #[test]
    fn forgotten_head_is_not_readable_or_reactivated() {
        let mut head = active_head();
        head.forget(4).expect("forget succeeds");

        assert!(!head.state.is_readable());
        assert_eq!(
            head.activate(5, Uuid::new_v4()),
            Err(HeadTransitionError::TerminalState(HeadState::Forgotten))
        );
    }

    #[test]
    fn both_projects_map_to_four_canonical_memory_types() {
        assert_eq!(
            MemoryType::map_input("project_fact"),
            Ok(MemoryTypeMapping::Alias(MemoryType::SemanticFact))
        );
        assert_eq!(
            MemoryType::map_input("semantic"),
            Ok(MemoryTypeMapping::Alias(MemoryType::SemanticFact))
        );
        assert_eq!(
            MemoryType::map_input("episodic"),
            Ok(MemoryTypeMapping::Alias(MemoryType::EpisodicSummary))
        );
        assert_eq!(
            MemoryType::map_input("profile"),
            Ok(MemoryTypeMapping::Alias(MemoryType::ProfileFact))
        );
        assert_eq!(
            MemoryType::map_input("procedural"),
            Ok(MemoryTypeMapping::ProceduralCandidate)
        );
        assert_eq!(
            CANONICAL_MEMORY_TYPE_NAMES,
            [
                "preference",
                "profile_fact",
                "semantic_fact",
                "episodic_summary"
            ]
        );
    }

    #[test]
    fn canonical_policy_types_roll_up_to_two_cognitive_families() {
        assert_eq!(MemoryType::Preference.family(), MemoryFamily::Semantic);
        assert_eq!(MemoryType::ProfileFact.family(), MemoryFamily::Semantic);
        assert_eq!(MemoryType::SemanticFact.family(), MemoryFamily::Semantic);
        assert_eq!(MemoryType::EpisodicSummary.family(), MemoryFamily::Episodic);
    }
}
