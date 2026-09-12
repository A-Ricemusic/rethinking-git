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

/// Missing direct read targets and restricted targets have the same public outcome.
/// Preserve parse, I/O and integrity failures instead of turning corruption into absence.
pub(crate) fn unavailable_if_missing(error: anyhow::Error) -> anyhow::Error {
    if error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound)
    }) {
        CliFailure::OperationUnavailable.into()
    } else {
        error
    }
}
