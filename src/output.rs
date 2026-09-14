//! One post-command JSON envelope, with records emitted at permission-checked sites.
use super::*;
use std::cell::RefCell;

const TEXT_LIMIT: usize = 1_048_576;
#[derive(Default)]
struct Buffer {
    records: Vec<serde_json::Value>,
    text: String,
    truncated: bool,
}
thread_local! {
    static BUFFER: RefCell<Option<Buffer>> = const { RefCell::new(None) };
}
pub(super) fn enabled() -> bool {
    BUFFER.with(|b| b.borrow().is_some())
}
pub(super) fn begin() {
    BUFFER.with(|buffer| *buffer.borrow_mut() = Some(Buffer::default()));
}
pub(super) fn text(message: String) {
    BUFFER.with(|buffer| {
        let mut buffer = buffer.borrow_mut();
        if let Some(buffer) = buffer.as_mut() {
            let mut count = message
                .len()
                .min(TEXT_LIMIT.saturating_sub(buffer.text.len()));
            while !message.is_char_boundary(count) {
                count -= 1;
            }
            buffer.text.push_str(&message[..count]);
            buffer.truncated |= count < message.len();
        } else {
            std::print!("{message}");
        }
    });
}
pub(super) fn record(kind: &str, data: serde_json::Value) {
    BUFFER.with(|buffer| {
        if let Some(buffer) = buffer.borrow_mut().as_mut() {
            buffer
                .records
                .push(serde_json::json!({"kind":kind,"data":data}));
        }
    });
}
pub(super) fn finish(command: &str, result: &Result<()>) -> Result<()> {
    use std::io::Write;
    let mut buffer = BUFFER
        .with(|buffer| buffer.borrow_mut().take())
        .unwrap_or_default();
    let conflict = result
        .as_ref()
        .err()
        .and_then(|e| e.downcast_ref::<CliFailure>())
        == Some(&CliFailure::IntegrationConflicted);
    if result.is_err() && !conflict {
        // Output accumulated before failed publication is not a committed result.
        buffer = Buffer::default();
    }
    let error = result.as_ref().err().map(|error| serde_json::json!({
        "kind": if conflict { "conflicts" } else if error.downcast_ref::<CliFailure>() == Some(&CliFailure::OperationUnavailable) { "unavailable" } else if error.downcast_ref::<CliFailure>() == Some(&CliFailure::StaleSnapshot) { "stale_snapshot" } else { "command_failed" },
        "message": format!("{error:#}"),
    }));
    let value = serde_json::json!({
        "schema_version":1, "command":command, "ok":result.is_ok(),
        "exit_code":if result.is_ok() {0} else {1},
        "records":buffer.records, "text":buffer.text, "text_truncated":buffer.truncated,
        "error":error,
    });
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    std::io::stdout()
        .lock()
        .write_all(&bytes)
        .context("failed to write JSON outcome")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_cap_keeps_valid_utf8_and_does_not_drop_records() {
        begin();
        text("x".repeat(TEXT_LIMIT - 1));
        text("é".to_owned());
        record("result", serde_json::json!({"id":"saved"}));
        BUFFER.with(|buffer| {
            let buffer = buffer.borrow_mut().take().unwrap();
            assert_eq!(buffer.text.len(), TEXT_LIMIT - 1);
            assert!(buffer.truncated);
            assert_eq!(buffer.records[0]["data"]["id"], "saved");
        });
    }
}
