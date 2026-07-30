use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Pending,
    NeedsReview,
    Accepted,
    Rejected,
}

impl CandidateState {
    pub fn decide(self, outcome: DecisionOutcome) -> Result<Self, CandidateTransitionError> {
        match (self, outcome) {
            (Self::Pending | Self::NeedsReview, DecisionOutcome::Accept) => Ok(Self::Accepted),
            (Self::Pending | Self::NeedsReview, DecisionOutcome::Reject) => Ok(Self::Rejected),
            (Self::Pending, DecisionOutcome::NeedsReview) => Ok(Self::NeedsReview),
            _ => Err(CandidateTransitionError::TerminalOrInvalid {
                from: self,
                outcome,
            }),
        }
    }

    pub fn is_indexable(self) -> bool {
        matches!(self, Self::Accepted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionOutcome {
    Accept,
    Reject,
    NeedsReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionActor {
    pub actor_type: String,
    pub actor_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateDecision {
    pub id: Uuid,
    pub candidate_id: Uuid,
    pub outcome: DecisionOutcome,
    pub reason_code: String,
    pub actor: DecisionActor,
    pub policy_version: String,
    pub decided_at: DateTime<Utc>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CandidateTransitionError {
    #[error("candidate transition from {from:?} using {outcome:?} is invalid")]
    TerminalOrInvalid {
        from: CandidateState,
        outcome: DecisionOutcome,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepted_candidate_is_indexable() {
        assert!(!CandidateState::Pending.is_indexable());
        assert!(!CandidateState::NeedsReview.is_indexable());
        assert!(!CandidateState::Rejected.is_indexable());
        assert!(CandidateState::Accepted.is_indexable());
    }

    #[test]
    fn final_decision_cannot_be_reversed() {
        let accepted = CandidateState::Pending
            .decide(DecisionOutcome::Accept)
            .expect("pending may be accepted");

        assert!(accepted.decide(DecisionOutcome::Reject).is_err());
    }
}
