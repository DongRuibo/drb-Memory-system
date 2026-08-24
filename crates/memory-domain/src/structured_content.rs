use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use thiserror::Error;

use crate::MemoryType;

pub const STRUCTURED_CONTENT_VERSION: u16 = 1;
pub const MAX_STRUCTURED_CONTENT_BYTES: usize = 4_096;

/// A strictly validated auxiliary representation of one canonical Memory Version.
///
/// The readable `content` remains canonical. This envelope is versioned so projections and
/// clients can consume stable fields without treating arbitrary model JSON as trusted data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuredContent {
    pub schema_name: String,
    pub schema_version: u16,
    pub payload: Value,
}

impl StructuredContent {
    pub fn new(
        schema_name: impl Into<String>,
        schema_version: u16,
        payload: Value,
        memory_type: MemoryType,
    ) -> Result<Self, StructuredContentError> {
        let value = Self {
            schema_name: schema_name.into(),
            schema_version,
            payload,
        };
        value.validate_for(memory_type)?;
        Ok(value)
    }

    pub fn validate_for(&self, memory_type: MemoryType) -> Result<(), StructuredContentError> {
        if self.schema_version != STRUCTURED_CONTENT_VERSION {
            return Err(StructuredContentError::UnsupportedVersion(
                self.schema_version,
            ));
        }
        let expected_schema = schema_name_for(memory_type);
        if self.schema_name != expected_schema {
            return Err(StructuredContentError::SchemaTypeMismatch {
                expected: expected_schema,
                actual: self.schema_name.clone(),
            });
        }
        let encoded = serde_json::to_vec(self)
            .map_err(|error| StructuredContentError::InvalidPayload(error.to_string()))?;
        if encoded.len() > MAX_STRUCTURED_CONTENT_BYTES {
            return Err(StructuredContentError::TooLarge(encoded.len()));
        }
        let payload = self
            .payload
            .as_object()
            .ok_or(StructuredContentError::PayloadMustBeObject)?;
        match memory_type {
            MemoryType::Preference => {
                validate_object(payload, &["subject", "value"], &["qualifier"], &[])
            }
            MemoryType::ProfileFact => {
                validate_object(payload, &["subject", "attribute", "value"], &[], &[])
            }
            MemoryType::SemanticFact => {
                validate_object(payload, &["subject", "predicate", "object"], &[], &[])
            }
            MemoryType::EpisodicSummary => {
                validate_object(payload, &["event_kind"], &["outcome"], &["participants"])
            }
        }
    }
}

pub const fn schema_name_for(memory_type: MemoryType) -> &'static str {
    match memory_type {
        MemoryType::Preference => "lazytt.preference",
        MemoryType::ProfileFact => "lazytt.profile_fact",
        MemoryType::SemanticFact => "lazytt.semantic_fact",
        MemoryType::EpisodicSummary => "lazytt.episodic_summary",
    }
}

fn validate_object(
    payload: &Map<String, Value>,
    required_strings: &[&str],
    optional_strings: &[&str],
    optional_string_arrays: &[&str],
) -> Result<(), StructuredContentError> {
    let allowed = required_strings
        .iter()
        .chain(optional_strings)
        .chain(optional_string_arrays)
        .copied()
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = payload.keys().find(|key| !allowed.contains(key.as_str())) {
        return Err(StructuredContentError::UnknownField(unknown.clone()));
    }
    for field in required_strings {
        let value = payload
            .get(*field)
            .ok_or_else(|| StructuredContentError::MissingField((*field).into()))?;
        validate_text(field, value)?;
    }
    for field in optional_strings {
        if let Some(value) = payload.get(*field) {
            validate_text(field, value)?;
        }
    }
    for field in optional_string_arrays {
        if let Some(value) = payload.get(*field) {
            let values = value.as_array().ok_or_else(|| {
                StructuredContentError::InvalidField((*field).into(), "must be an array".into())
            })?;
            if values.is_empty() || values.len() > 8 {
                return Err(StructuredContentError::InvalidField(
                    (*field).into(),
                    "must contain 1 to 8 values".into(),
                ));
            }
            for value in values {
                validate_text(field, value)?;
            }
        }
    }
    Ok(())
}

fn validate_text(field: &str, value: &Value) -> Result<(), StructuredContentError> {
    let text = value.as_str().ok_or_else(|| {
        StructuredContentError::InvalidField(field.into(), "must be a string".into())
    })?;
    if text.is_empty()
        || text.len() > 256
        || text.trim() != text
        || text.chars().any(char::is_control)
    {
        return Err(StructuredContentError::InvalidField(
            field.into(),
            "must be trimmed, non-empty, at most 256 bytes, and contain no control characters"
                .into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StructuredContentError {
    #[error("structured content schema version {0} is unsupported")]
    UnsupportedVersion(u16),
    #[error("structured content schema must be {expected:?} for this memory type, got {actual:?}")]
    SchemaTypeMismatch {
        expected: &'static str,
        actual: String,
    },
    #[error("structured content payload must be an object")]
    PayloadMustBeObject,
    #[error("structured content payload is missing required field {0:?}")]
    MissingField(String),
    #[error("structured content payload contains unknown field {0:?}")]
    UnknownField(String),
    #[error("structured content field {0:?} {1}")]
    InvalidField(String, String),
    #[error("structured content is {0} bytes, exceeding the 4096 byte limit")]
    TooLarge(usize),
    #[error("structured content payload is invalid: {0}")]
    InvalidPayload(String),
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn validates_each_v1_schema_and_rejects_type_drift() {
        let preference = StructuredContent::new(
            "lazytt.preference",
            1,
            json!({"subject": "user", "value": "先给结论", "qualifier": "技术问题"}),
            MemoryType::Preference,
        )
        .unwrap();
        assert_eq!(preference.schema_version, 1);
        assert!(preference.validate_for(MemoryType::ProfileFact).is_err());

        StructuredContent::new(
            "lazytt.profile_fact",
            1,
            json!({"subject": "user", "attribute": "role", "value": "Agent研发工程师"}),
            MemoryType::ProfileFact,
        )
        .unwrap();
        StructuredContent::new(
            "lazytt.semantic_fact",
            1,
            json!({"subject": "LazyTT", "predicate": "authority_store", "object": "PostgreSQL"}),
            MemoryType::SemanticFact,
        )
        .unwrap();
        StructuredContent::new(
            "lazytt.episodic_summary",
            1,
            json!({
                "event_kind": "deployment_failure",
                "participants": ["user", "LazyTT"],
                "outcome": "通过 Outbox 重放恢复"
            }),
            MemoryType::EpisodicSummary,
        )
        .unwrap();
    }

    #[test]
    fn rejects_unknown_nested_shape_and_unbounded_text() {
        assert!(matches!(
            StructuredContent::new(
                "lazytt.preference",
                1,
                json!({"subject": "user", "value": "先给结论", "command": "执行工具"}),
                MemoryType::Preference,
            ),
            Err(StructuredContentError::UnknownField(_))
        ));
        assert!(matches!(
            StructuredContent::new(
                "lazytt.episodic_summary",
                1,
                json!({"event_kind": "incident", "participants": []}),
                MemoryType::EpisodicSummary,
            ),
            Err(StructuredContentError::InvalidField(_, _))
        ));
    }
}
