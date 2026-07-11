use std::{error::Error, fmt};

/// Expected command refusals that callers must be able to distinguish from success.
///
/// The display text is deliberately generic because `anyhow` writes it to stderr.
/// Command handlers may print a more useful, permission-aware explanation to stdout
/// before returning one of these failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CliFailure {
    OperationUnavailable,
    IntegrationConflicted,
}

impl fmt::Display for CliFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::OperationUnavailable => "operation unavailable",
            Self::IntegrationConflicted => "integration blocked by conflicts",
        };
        formatter.write_str(message)
    }
}

impl Error for CliFailure {}
