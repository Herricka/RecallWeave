//! Validation for records that arrive from the outside world.
//!
//! A `memory-pack` export or a JSONL segment can be truncated, hand-edited or
//! produced by an older version of the format. Importing such data silently
//! corrupts the chain, so every record is checked before it is admitted.

use std::fmt;

/// Reasons a record can be refused.
#[derive(Debug, PartialEq, Eq)]
pub enum Rejection {
    /// The record body was empty or only whitespace.
    Empty,
    /// A field that must be present was missing or empty.
    MissingField(&'static str),
    /// A numeric field was not a finite number.
    NotFinite(&'static str),
    /// The stored hash does not match the recomputed one.
    HashMismatch,
    /// The record type is not part of the format.
    UnknownKind(String),
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rejection::Empty => write!(f, "record is empty"),
            Rejection::MissingField(field) => write!(f, "missing required field `{field}`"),
            Rejection::NotFinite(field) => write!(f, "`{field}` is not a finite number"),
            Rejection::HashMismatch => write!(f, "stored hash does not match the record contents"),
            Rejection::UnknownKind(kind) => write!(f, "unknown record kind `{kind}`"),
        }
    }
}

impl std::error::Error for Rejection {}

/// A single record as it appears on the wire, before it becomes a typed memory.
#[derive(Debug, Clone, Default)]
pub struct RawRecord {
    pub kind: String,
    pub id: String,
    pub body: String,
    pub score: Option<f64>,
    pub recorded_at: Option<i64>,
}

/// Validate a raw record, rejecting anything that would poison the chain.
pub fn validate(record: &RawRecord) -> Result<(), Rejection> {
    if record.kind.trim().is_empty() && record.id.trim().is_empty() && record.body.trim().is_empty() {
        return Err(Rejection::Empty);
    }
    if record.kind.trim().is_empty() {
        return Err(Rejection::MissingField("kind"));
    }
    if record.id.trim().is_empty() {
        return Err(Rejection::MissingField("id"));
    }
    if record.body.trim().is_empty() {
        return Err(Rejection::MissingField("body"));
    }
    if !matches!(record.kind.as_str(), "assert" | "supersede" | "tombstone" | "revise") {
        return Err(Rejection::UnknownKind(record.kind.clone()));
    }
    if let Some(score) = record.score {
        if !score.is_finite() {
            return Err(Rejection::NotFinite("score"));
        }
        if !(0.0..=1.0).contains(&score) {
            return Err(Rejection::NotFinite("score"));
        }
    }
    if let Some(ts) = record.recorded_at {
        if ts < 0 {
            return Err(Rejection::NotFinite("recorded_at"));
        }
    }
    Ok(())
}

/// Validate a whole segment, returning the index of the first bad record.
pub fn validate_segment(records: &[RawRecord]) -> Result<(), (usize, Rejection)> {
    for (i, record) in records.iter().enumerate() {
        validate(record).map_err(|e| (i, e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> RawRecord {
        RawRecord {
            kind: "assert".into(),
            id: "mem-1".into(),
            body: "the user prefers metric units".into(),
            score: Some(0.8),
            recorded_at: Some(1_700_000_000),
        }
    }

    #[test]
    fn accepts_a_well_formed_record() {
        assert_eq!(validate(&good()), Ok(()));
    }

    #[test]
    fn rejects_an_empty_record() {
        assert_eq!(validate(&RawRecord::default()), Err(Rejection::Empty));
    }

    #[test]
    fn rejects_missing_fields() {
        let mut r = good();
        r.id = String::new();
        assert_eq!(validate(&r), Err(Rejection::MissingField("id")));

        let mut r = good();
        r.body = "   ".into();
        assert_eq!(validate(&r), Err(Rejection::MissingField("body")));
    }

    #[test]
    fn rejects_unknown_kinds() {
        let mut r = good();
        r.kind = "drop-table".into();
        assert_eq!(validate(&r), Err(Rejection::UnknownKind("drop-table".into())));
    }

    #[test]
    fn rejects_non_finite_scores() {
        let mut r = good();
        r.score = Some(f64::NAN);
        assert_eq!(validate(&r), Err(Rejection::NotFinite("score")));

        let mut r = good();
        r.score = Some(4.2);
        assert_eq!(validate(&r), Err(Rejection::NotFinite("score")));
    }

    #[test]
    fn reports_the_first_bad_index_in_a_segment() {
        let mut bad = good();
        bad.body = String::new();
        let segment = vec![good(), good(), bad, good()];
        match validate_segment(&segment) {
            Err((2, Rejection::MissingField("body"))) => {}
            other => panic!("expected index 2, got {other:?}"),
        }
    }

    #[test]
    fn accepts_an_empty_segment() {
        assert_eq!(validate_segment(&[]), Ok(()));
    }
}


/// A segment-scoped rejection: which file the bad record came from, and where.
#[derive(Debug, PartialEq, Eq)]
pub struct SegmentReport {
    pub path: String,
    pub index: usize,
    pub reason: Rejection,
}

impl std::fmt::Display for SegmentReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: record {}: {}", self.path, self.index, self.reason)
    }
}

impl std::error::Error for SegmentReport {}

/// Validate one named segment, attaching the path to any rejection.
pub fn validate_named(path: &str, records: &[RawRecord]) -> Result<(), SegmentReport> {
    validate_segment(records).map_err(|(index, reason)| SegmentReport {
        path: path.to_string(),
        index,
        reason,
    })
}

#[cfg(test)]
mod segment_tests {
    use super::*;

    fn good() -> RawRecord {
        RawRecord {
            kind: "assert".into(),
            id: "mem-1".into(),
            body: "prefers metric units".into(),
            score: Some(0.5),
            recorded_at: Some(1),
        }
    }

    #[test]
    fn names_the_failing_segment() {
        let mut bad = good();
        bad.kind = String::new();
        let segment = vec![good(), bad];
        let err = validate_named("pack/2026-08.jsonl", &segment).unwrap_err();
        assert_eq!(err.path, "pack/2026-08.jsonl");
        assert_eq!(err.index, 1);
    }

    #[test]
    fn accepts_a_named_segment() {
        assert!(validate_named("pack/ok.jsonl", &[good()]).is_ok());
    }
}
