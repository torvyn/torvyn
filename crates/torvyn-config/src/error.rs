//! Rich configuration error types with source location and actionable messages.
//!
//! Error code range: E0700–E0799 (per Doc 09, G-08 / Doc 10, C07-2).
//!
//! These errors carry enough context for the CLI to render
//! `rustc`-quality diagnostics: file path, key path, expected value,
//! found value, and a suggested fix.

use std::fmt;
use torvyn_types::ConfigError;

/// A configuration parsing or validation error with full diagnostic context.
///
/// # Invariants
/// - `file` is always a valid filesystem path or `"<inline>"` for programmatic configs.
/// - `key_path` uses dot-separated TOML key notation (e.g., `"runtime.backpressure.default_queue_depth"`).
/// - `suggestion` is always a non-empty actionable string.
///
/// # Examples
/// ```
/// use torvyn_config::ConfigParseError;
///
/// let err = ConfigParseError::invalid_value(
///     "Torvyn.toml",
///     "runtime.scheduling.policy",
///     "random",
///     "one of: round-robin, weighted-fair, priority",
///     "Change the value to a supported scheduling policy.",
/// );
/// assert!(format!("{}", err).contains("E0703"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigParseError {
    /// Error code (E07xx).
    pub code: &'static str,
    /// Path to the configuration file.
    pub file: String,
    /// Dot-separated key path within the TOML document.
    pub key_path: String,
    /// Description of the error.
    pub message: String,
    /// What the parser expected.
    pub expected: String,
    /// What the parser found (empty if not applicable).
    pub found: String,
    /// Actionable suggestion for the user.
    pub suggestion: String,
}

impl ConfigParseError {
    /// Create an error for a file that could not be read.
    ///
    /// # COLD PATH — called during config loading.
    pub fn file_not_found(file: &str) -> Self {
        Self {
            code: "E0700",
            file: file.to_owned(),
            key_path: String::new(),
            message: format!("Configuration file not found: '{file}'"),
            expected: "a readable TOML file".to_owned(),
            found: "file does not exist or is not readable".to_owned(),
            suggestion: "Create a Torvyn.toml or specify a path with --manifest.".to_owned(),
        }
    }

    /// Create an error for invalid TOML syntax.
    ///
    /// # COLD PATH — called during config loading.
    pub fn toml_syntax(file: &str, toml_err: &toml::de::Error) -> Self {
        let span_info = toml_err
            .span()
            .map(|s| format!(" at byte offset {}..{}", s.start, s.end))
            .unwrap_or_default();

        Self {
            code: "E0701",
            file: file.to_owned(),
            key_path: String::new(),
            message: format!("Invalid TOML syntax{span_info}: {toml_err}"),
            expected: "valid TOML".to_owned(),
            found: format!("syntax error: {toml_err}"),
            suggestion: "Fix the TOML syntax. See https://toml.io for the specification."
                .to_owned(),
        }
    }

    /// Create an error for a missing required field.
    ///
    /// # COLD PATH — called during config validation.
    pub fn missing_field(file: &str, key_path: &str, context: &str) -> Self {
        Self {
            code: "E0702",
            file: file.to_owned(),
            key_path: key_path.to_owned(),
            message: format!("Missing required field '{key_path}'"),
            expected: format!("field '{key_path}' to be present in {context}"),
            found: "field is absent".to_owned(),
            suggestion: format!("Add `{key_path} = <value>` to the [{context}] section."),
        }
    }

    /// Create an error for an invalid field value.
    ///
    /// # COLD PATH — called during config validation.
    pub fn invalid_value(
        file: &str,
        key_path: &str,
        found: &str,
        expected: &str,
        suggestion: &str,
    ) -> Self {
        Self {
            code: "E0703",
            file: file.to_owned(),
            key_path: key_path.to_owned(),
            message: format!("Invalid value for '{key_path}'"),
            expected: expected.to_owned(),
            found: found.to_owned(),
            suggestion: suggestion.to_owned(),
        }
    }

    /// Create an error for a semantic validation failure.
    ///
    /// # COLD PATH — called during cross-field validation.
    pub fn semantic(file: &str, key_path: &str, message: &str, suggestion: &str) -> Self {
        Self {
            code: "E0704",
            file: file.to_owned(),
            key_path: key_path.to_owned(),
            message: message.to_owned(),
            expected: String::new(),
            found: String::new(),
            suggestion: suggestion.to_owned(),
        }
    }

    /// Create an error for environment variable interpolation failure.
    ///
    /// # COLD PATH — called during env var resolution.
    pub fn env_var_not_found(file: &str, key_path: &str, var_name: &str) -> Self {
        Self {
            code: "E0705",
            file: file.to_owned(),
            key_path: key_path.to_owned(),
            message: format!("Environment variable '{var_name}' referenced but not set"),
            expected: format!("environment variable '{var_name}' to be defined"),
            found: "variable is not set".to_owned(),
            suggestion: format!(
                "Set the environment variable: export {var_name}=<value>, \
                 or remove the ${{{{ {var_name} }}}} reference from your configuration."
            ),
        }
    }

    /// Create an error for duplicate keys in the configuration.
    ///
    /// # COLD PATH — called during config validation.
    pub fn duplicate_key(file: &str, key_path: &str) -> Self {
        Self {
            code: "E0706",
            file: file.to_owned(),
            key_path: key_path.to_owned(),
            message: format!("Duplicate key '{key_path}'"),
            expected: "each key to appear at most once".to_owned(),
            found: "key appears more than once".to_owned(),
            suggestion: "Remove the duplicate entry.".to_owned(),
        }
    }
}

impl fmt::Display for ConfigParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] ", self.code)?;
        write!(f, "{}", self.message)?;

        if !self.file.is_empty() {
            write!(f, "\n  --> {}", self.file)?;
        }
        if !self.key_path.is_empty() {
            write!(f, " :: {}", self.key_path)?;
        }
        if !self.expected.is_empty() {
            write!(f, "\n  expected: {}", self.expected)?;
        }
        if !self.found.is_empty() {
            write!(f, "\n  found:    {}", self.found)?;
        }
        if !self.suggestion.is_empty() {
            write!(f, "\n  help: {}", self.suggestion)?;
        }

        Ok(())
    }
}

impl std::error::Error for ConfigParseError {}

impl From<ConfigParseError> for ConfigError {
    fn from(e: ConfigParseError) -> Self {
        match e.code {
            "E0700" => ConfigError::FileNotFound { path: e.file },
            "E0701" => ConfigError::ParseError {
                path: e.file,
                message: e.message,
            },
            "E0702" => ConfigError::MissingField {
                field: e.key_path,
                context: e.file,
            },
            _ => ConfigError::InvalidValue {
                field: e.key_path,
                value: e.found,
                reason: e.message,
            },
        }
    }
}

/// A collection of configuration errors, supporting batch reporting.
///
/// Configuration validation collects all errors rather than failing on the first,
/// so the user can fix multiple issues in one pass.
///
/// # Examples
/// ```
/// use torvyn_config::{ConfigParseError, ConfigErrors};
///
/// let mut errors = ConfigErrors::new();
/// assert!(errors.is_empty());
///
/// errors.push(ConfigParseError::missing_field("Torvyn.toml", "torvyn.name", "torvyn"));
/// assert!(!errors.is_empty());
/// assert_eq!(errors.len(), 1);
/// ```
#[derive(Clone, Debug, Default)]
pub struct ConfigErrors {
    errors: Vec<ConfigParseError>,
}

impl ConfigErrors {
    /// Create an empty error collection.
    ///
    /// # COLD PATH — called once per parse operation.
    pub fn new() -> Self {
        Self { errors: Vec::new() }
    }

    /// Add an error to the collection.
    ///
    /// # COLD PATH — called per validation failure.
    pub fn push(&mut self, error: ConfigParseError) {
        self.errors.push(error);
    }

    /// Returns `true` if no errors have been collected.
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    /// Returns the number of collected errors.
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    /// Consume the collection and return the errors as a `Vec`.
    pub fn into_vec(self) -> Vec<ConfigParseError> {
        self.errors
    }

    /// Iterate over the collected errors.
    pub fn iter(&self) -> std::slice::Iter<'_, ConfigParseError> {
        self.errors.iter()
    }

    /// If any errors were collected, return them as `Err`. Otherwise `Ok(())`.
    ///
    /// # COLD PATH — called once after all validation completes.
    ///
    /// # Errors
    /// Returns `Err(Vec<ConfigParseError>)` if any errors have been collected.
    pub fn into_result(self) -> Result<(), Vec<ConfigParseError>> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors)
        }
    }
}

impl fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, err) in self.errors.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
                writeln!(f)?;
            }
            write!(f, "{err}")?;
        }
        Ok(())
    }
}

impl IntoIterator for ConfigErrors {
    type Item = ConfigParseError;
    type IntoIter = std::vec::IntoIter<ConfigParseError>;

    fn into_iter(self) -> Self::IntoIter {
        self.errors.into_iter()
    }
}

impl<'a> IntoIterator for &'a ConfigErrors {
    type Item = &'a ConfigParseError;
    type IntoIter = std::slice::Iter<'a, ConfigParseError>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_not_found_display_includes_code_and_path() {
        let err = ConfigParseError::file_not_found("/app/Torvyn.toml");
        let msg = format!("{err}");
        assert!(msg.contains("E0700"));
        assert!(msg.contains("/app/Torvyn.toml"));
        assert!(msg.contains("help:"));
    }

    #[test]
    fn test_missing_field_display_includes_key_path() {
        let err = ConfigParseError::missing_field("Torvyn.toml", "torvyn.name", "torvyn");
        let msg = format!("{err}");
        assert!(msg.contains("E0702"));
        assert!(msg.contains("torvyn.name"));
        assert!(msg.contains("torvyn"));
    }

    #[test]
    fn test_invalid_value_display_includes_expected_and_found() {
        let err = ConfigParseError::invalid_value(
            "Torvyn.toml",
            "runtime.scheduling.policy",
            "random",
            "one of: round-robin, weighted-fair, priority",
            "Use a supported policy name.",
        );
        let msg = format!("{err}");
        assert!(msg.contains("E0703"));
        assert!(msg.contains("random"));
        assert!(msg.contains("round-robin"));
    }

    #[test]
    fn test_config_parse_error_converts_to_config_error() {
        let parse_err = ConfigParseError::file_not_found("/app/Torvyn.toml");
        let config_err: ConfigError = parse_err.into();
        assert!(matches!(config_err, ConfigError::FileNotFound { .. }));
    }

    #[test]
    fn test_config_errors_empty() {
        let errors = ConfigErrors::new();
        assert!(errors.is_empty());
        assert_eq!(errors.len(), 0);
        assert!(errors.into_result().is_ok());
    }

    #[test]
    fn test_config_errors_collect_and_report() {
        let mut errors = ConfigErrors::new();
        errors.push(ConfigParseError::missing_field("f.toml", "a.b", "a"));
        errors.push(ConfigParseError::missing_field("f.toml", "c.d", "c"));
        assert_eq!(errors.len(), 2);
        let result = errors.into_result();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().len(), 2);
    }

    #[test]
    fn test_config_errors_display_separates_errors() {
        let mut errors = ConfigErrors::new();
        errors.push(ConfigParseError::missing_field("f.toml", "a", "x"));
        errors.push(ConfigParseError::missing_field("f.toml", "b", "y"));
        let msg = format!("{errors}");
        // Should contain both error codes
        assert_eq!(msg.matches("E0702").count(), 2);
    }

    #[test]
    fn test_env_var_not_found_display() {
        let err = ConfigParseError::env_var_not_found(
            "Torvyn.toml",
            "runtime.worker_threads",
            "MY_THREADS",
        );
        let msg = format!("{err}");
        assert!(msg.contains("E0705"));
        assert!(msg.contains("MY_THREADS"));
        assert!(msg.contains("export"));
    }

    #[test]
    fn test_duplicate_key_display() {
        let err = ConfigParseError::duplicate_key("Torvyn.toml", "flow.main");
        let msg = format!("{err}");
        assert!(msg.contains("E0706"));
        assert!(msg.contains("flow.main"));
    }

    #[test]
    fn test_config_errors_into_iter() {
        let mut errors = ConfigErrors::new();
        errors.push(ConfigParseError::missing_field("f.toml", "a", "x"));
        errors.push(ConfigParseError::missing_field("f.toml", "b", "y"));
        let collected: Vec<_> = errors.into_iter().collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn test_semantic_error_display() {
        let err = ConfigParseError::semantic(
            "pipeline.toml",
            "flow.main",
            "Flow 'main' has 1 node(s), but at least 2 are required",
            "Add at least a source and a sink node.",
        );
        let msg = format!("{err}");
        assert!(msg.contains("E0704"));
        assert!(msg.contains("flow.main"));
    }

    #[test]
    fn test_toml_syntax_error_converts_to_config_error() {
        let parse_err = ConfigParseError {
            code: "E0701",
            file: "test.toml".to_owned(),
            key_path: String::new(),
            message: "Invalid TOML".to_owned(),
            expected: "valid TOML".to_owned(),
            found: "bad syntax".to_owned(),
            suggestion: "Fix syntax.".to_owned(),
        };
        let config_err: ConfigError = parse_err.into();
        assert!(matches!(config_err, ConfigError::ParseError { .. }));
    }

    #[test]
    fn test_missing_field_converts_to_config_error() {
        let parse_err = ConfigParseError::missing_field("f.toml", "torvyn.name", "torvyn");
        let config_err: ConfigError = parse_err.into();
        assert!(matches!(config_err, ConfigError::MissingField { .. }));
    }

    #[test]
    fn test_invalid_value_converts_to_config_error() {
        let parse_err =
            ConfigParseError::invalid_value("f.toml", "key", "bad", "good", "fix it");
        let config_err: ConfigError = parse_err.into();
        assert!(matches!(config_err, ConfigError::InvalidValue { .. }));
    }
}
