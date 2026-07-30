use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    Preference,
    ProfileFact,
    ProjectFact,
    EpisodicSummary,
    ProceduralCandidate,
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
}
