//! Diagnostics surfaced from config parse/validate passes.
//!
//! A [`Diagnostic`] describes one problem (or warning) found while loading
//! `config.toml`. Errors mean the file was rejected outright (last-good
//! config stays in effect); warnings mean the file was applied but something
//! in it deserves attention (e.g. an unrecognized key).

use std::fmt;

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The config file was rejected; the previous valid config remains active.
    Error,
    /// The config file was applied, but this aspect deserves attention.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Severity::Error => write!(f, "error"),
            Severity::Warning => write!(f, "warning"),
        }
    }
}

/// A single diagnostic message produced while loading a config file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    /// 1-based (line, column) into the source file, when known.
    pub span: Option<(usize, usize)>,
    /// The config key this diagnostic concerns, when applicable (e.g. an
    /// unknown key warning, or a validation failure on a specific field).
    pub key: Option<String>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Error,
            message: message.into(),
            span: None,
            key: None,
        }
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Diagnostic {
            severity: Severity::Warning,
            message: message.into(),
            span: None,
            key: None,
        }
    }

    pub fn with_span(mut self, line: usize, col: usize) -> Self {
        self.span = Some((line, col));
        self
    }

    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.severity)?;
        if let Some(key) = &self.key {
            write!(f, " [{key}]")?;
        }
        if let Some((line, col)) = self.span {
            write!(f, " at {line}:{col}")?;
        }
        write!(f, ": {}", self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_all_parts() {
        let d = Diagnostic::error("bad value").with_key("font-size").with_span(3, 7);
        assert_eq!(d.to_string(), "error [font-size] at 3:7: bad value");
    }

    #[test]
    fn display_minimal() {
        let d = Diagnostic::warning("unknown key");
        assert_eq!(d.to_string(), "warning: unknown key");
    }
}
