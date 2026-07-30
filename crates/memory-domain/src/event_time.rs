use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Precision of the real-world event represented by a governed memory.
///
/// This is deliberately separate from query-window precision such as
/// `rolling_days`: it describes evidence about an event, not how a user
/// phrased a retrieval query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventTimePrecision {
    Instant,
    Minute,
    Hour,
    PartOfDay,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl EventTimePrecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Instant => "instant",
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::PartOfDay => "part_of_day",
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Quarter => "quarter",
            Self::Year => "year",
        }
    }
}

/// A source-supported, half-open interval for when an episodic event occurred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventTime {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub timezone: String,
    pub precision: EventTimePrecision,
}

impl EventTime {
    pub fn new(
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        timezone: impl Into<String>,
        precision: EventTimePrecision,
    ) -> Result<Self, EventTimeError> {
        let value = Self {
            start,
            end,
            timezone: timezone.into(),
            precision,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), EventTimeError> {
        if self.start >= self.end {
            return Err(EventTimeError::InvalidInterval);
        }
        if self.timezone.is_empty()
            || self.timezone.len() > 64
            || self.timezone.trim() != self.timezone
            || self.timezone.chars().any(char::is_control)
            || self.timezone.parse::<Tz>().is_err()
        {
            return Err(EventTimeError::InvalidTimezone(self.timezone.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EventTimeError {
    #[error("event time must be a non-empty half-open interval")]
    InvalidInterval,
    #[error("event time timezone is not a valid bounded IANA timezone: {0:?}")]
    InvalidTimezone(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_time_requires_a_valid_half_open_interval_and_timezone() {
        let start = "2026-07-01T00:00:00Z".parse().unwrap();
        let end = "2026-07-02T00:00:00Z".parse().unwrap();
        let value = EventTime::new(start, end, "Asia/Shanghai", EventTimePrecision::Day).unwrap();
        assert_eq!(value.precision.as_str(), "day");

        assert_eq!(
            EventTime::new(end, start, "Asia/Shanghai", EventTimePrecision::Day).unwrap_err(),
            EventTimeError::InvalidInterval
        );
        assert_eq!(
            EventTime::new(start, end, "Mars/Olympus", EventTimePrecision::Day).unwrap_err(),
            EventTimeError::InvalidTimezone("Mars/Olympus".into())
        );
    }
}
