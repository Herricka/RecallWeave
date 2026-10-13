

use std::fmt;

/// Why an export could not be written.
#[derive(Debug, PartialEq, Eq)]
pub enum ExportError {
    /// The store holds no records; writing an empty pack would restore as a
    /// memory that silently forgot everything.
    Empty,
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::Empty => write!(
                f,
                "refusing to export an empty memory pack: an empty pack restores as \
                 an agent with no memories, which looks like a successful restore"
            ),
        }
    }
}

impl std::error::Error for ExportError {}

/// Guard an export before it is written.
pub fn ensure_exportable(record_count: usize) -> Result<(), ExportError> {
    if record_count == 0 {
        return Err(ExportError::Empty);
    }
    Ok(())
}

#[cfg(test)]
mod export_guard_tests {
    use super::*;

    #[test]
    fn refuses_an_empty_pack() {
        assert_eq!(ensure_exportable(0), Err(ExportError::Empty));
    }

    #[test]
    fn allows_a_populated_pack() {
        assert_eq!(ensure_exportable(1), Ok(()));
    }

    #[test]
    fn explains_the_failure() {
        let msg = ExportError::Empty.to_string();
        assert!(msg.contains("refusing to export"));
    }
}
