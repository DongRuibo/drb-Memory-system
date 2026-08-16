use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const EPISODE_CONSOLIDATION_TURN_THRESHOLD: u64 = 10;
pub const MAX_COMPLETED_TURN_USER_BYTES: usize = 20_000;
pub const MAX_COMPLETED_TURN_ASSISTANT_BYTES: usize = 40_000;
pub const MAX_VERIFIED_TOOL_RESULTS: usize = 16;
pub const MAX_VERIFIED_TOOL_RESULT_BYTES: usize = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeState {
    Active,
    Closed,
}

impl EpisodeState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

impl TurnOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationTrigger {
    Threshold,
    Manual,
    SessionEnd,
}

impl ConsolidationTrigger {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Threshold => "threshold",
            Self::Manual => "manual",
            Self::SessionEnd => "session_end",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationStatus {
    Pending,
    Leased,
    Completed,
    DeadLetter,
}

impl ConsolidationStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Leased => "leased",
            Self::Completed => "completed",
            Self::DeadLetter => "dead_letter",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedToolResult {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: String,
}

impl VerifiedToolResult {
    pub fn validate(&self) -> Result<(), EpisodeError> {
        validate_identifier("tool_call_id", &self.tool_call_id)?;
        validate_identifier("tool_name", &self.tool_name)?;
        validate_bounded_text(
            "verified tool result",
            &self.content,
            MAX_VERIFIED_TOOL_RESULT_BYTES,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedTurn {
    pub episode_id: Uuid,
    pub turn_id: Uuid,
    pub task_id: Uuid,
    pub attempt_id: Uuid,
    pub sequence_no: u64,
    pub user_content: String,
    pub assistant_content: String,
    pub verified_tool_results: Vec<VerifiedToolResult>,
    pub outcome: TurnOutcome,
    pub completed_at: DateTime<Utc>,
    pub timezone: String,
}

impl CompletedTurn {
    pub fn validate(&self) -> Result<(), EpisodeError> {
        if self.sequence_no == 0 {
            return Err(EpisodeError::InvalidSequence);
        }
        validate_bounded_text(
            "user content",
            &self.user_content,
            MAX_COMPLETED_TURN_USER_BYTES,
        )?;
        validate_bounded_text(
            "assistant content",
            &self.assistant_content,
            MAX_COMPLETED_TURN_ASSISTANT_BYTES,
        )?;
        if self.verified_tool_results.len() > MAX_VERIFIED_TOOL_RESULTS {
            return Err(EpisodeError::TooManyToolResults);
        }
        for result in &self.verified_tool_results {
            result.validate()?;
        }
        validate_timezone(&self.timezone)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Episode {
    pub id: Uuid,
    pub state: EpisodeState,
    pub last_turn_sequence: u64,
    pub scheduled_through_sequence: u64,
    pub last_consolidated_turn_sequence: u64,
}

impl Episode {
    pub fn validate(&self) -> Result<(), EpisodeError> {
        if self.last_consolidated_turn_sequence > self.scheduled_through_sequence
            || self.scheduled_through_sequence > self.last_turn_sequence
        {
            return Err(EpisodeError::InvalidCoverage);
        }
        Ok(())
    }

    pub fn should_schedule_threshold(&self) -> bool {
        self.last_turn_sequence
            .saturating_sub(self.scheduled_through_sequence)
            >= EPISODE_CONSOLIDATION_TURN_THRESHOLD
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EpisodeError {
    #[error("completed turn sequence must be positive")]
    InvalidSequence,
    #[error("episode consolidation coverage is invalid")]
    InvalidCoverage,
    #[error("{field} must contain 1 to {maximum} bytes")]
    InvalidText { field: &'static str, maximum: usize },
    #[error("completed turn has too many verified tool results")]
    TooManyToolResults,
    #[error("identifier {0} is invalid")]
    InvalidIdentifier(&'static str),
    #[error("timezone is invalid")]
    InvalidTimezone,
}

fn validate_bounded_text(
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), EpisodeError> {
    if value.trim().is_empty() || value.len() > maximum {
        return Err(EpisodeError::InvalidText { field, maximum });
    }
    Ok(())
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), EpisodeError> {
    if value.trim().is_empty()
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(EpisodeError::InvalidIdentifier(field));
    }
    Ok(())
}

fn validate_timezone(value: &str) -> Result<(), EpisodeError> {
    if value.is_empty()
        || value.len() > 64
        || value.trim() != value
        || value.parse::<chrono_tz::Tz>().is_err()
    {
        return Err(EpisodeError::InvalidTimezone);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn episode_coverage_is_monotonic_and_bounded() {
        let valid = Episode {
            id: Uuid::new_v4(),
            state: EpisodeState::Active,
            last_turn_sequence: 20,
            scheduled_through_sequence: 20,
            last_consolidated_turn_sequence: 10,
        };
        assert_eq!(valid.validate(), Ok(()));

        let invalid = Episode {
            last_consolidated_turn_sequence: 21,
            ..valid
        };
        assert_eq!(invalid.validate(), Err(EpisodeError::InvalidCoverage));
    }

    #[test]
    fn threshold_uses_only_unscheduled_completed_turns() {
        let episode = Episode {
            id: Uuid::new_v4(),
            state: EpisodeState::Active,
            last_turn_sequence: 19,
            scheduled_through_sequence: 10,
            last_consolidated_turn_sequence: 10,
        };
        assert!(!episode.should_schedule_threshold());

        let episode = Episode {
            last_turn_sequence: 20,
            ..episode
        };
        assert!(episode.should_schedule_threshold());
    }
}
