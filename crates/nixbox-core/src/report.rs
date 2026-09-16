//! Where the engine sends human-readable progress notes.
//!
//! The engine never prints and never touches a terminal; it hands every note
//! to a `Reporter`. The TUI collects them into its build log, the CLI writes
//! them to stderr, and tests assert on them.

/// Receives notes emitted while an [`crate::Op`] is applied.
pub trait Reporter {
    /// A change that was made, or a fact worth surfacing.
    fn info(&mut self, msg: String);

    /// Something went wrong that did not abort the operation — the caller
    /// should expect to fix it by hand.
    fn warn(&mut self, msg: String);
}

/// Discards every note.
#[derive(Debug, Default, Clone, Copy)]
pub struct SilentReporter;

impl Reporter for SilentReporter {
    fn info(&mut self, _msg: String) {}
    fn warn(&mut self, _msg: String) {}
}

/// Buffers notes as display-ready lines, prefixing warnings with `Warning: `.
#[derive(Debug, Default)]
pub struct LogReporter {
    lines: Vec<String>,
}

impl LogReporter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Consumes the reporter and returns everything it buffered.
    #[must_use]
    pub fn into_lines(self) -> Vec<String> {
        self.lines
    }

    /// Returns the buffered lines and leaves the reporter empty, so one
    /// reporter can be reused across several operations.
    pub fn drain(&mut self) -> Vec<String> {
        std::mem::take(&mut self.lines)
    }
}

impl Reporter for LogReporter {
    fn info(&mut self, msg: String) {
        self.lines.push(msg);
    }

    fn warn(&mut self, msg: String) {
        self.lines.push(format!("Warning: {msg}"));
    }
}

#[cfg(test)]
mod tests {
    use super::{LogReporter, Reporter};

    #[test]
    fn warnings_are_prefixed_and_infos_are_not() {
        let mut reporter = LogReporter::new();
        reporter.info("wrote the file".into());
        reporter.warn("could not stage it".into());

        assert_eq!(
            reporter.lines(),
            ["wrote the file", "Warning: could not stage it"]
        );
    }

    #[test]
    fn drain_empties_the_buffer() {
        let mut reporter = LogReporter::new();
        reporter.info("one".into());

        assert_eq!(reporter.drain(), vec!["one".to_string()]);
        assert!(reporter.lines().is_empty());
    }
}
