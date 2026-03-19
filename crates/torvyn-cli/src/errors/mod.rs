//! CLI error types and exit code mapping.
//!
//! The [`CliError`] type wraps all possible error conditions from every
//! subcommand and maps them to user-facing error output and exit codes.

pub mod diagnostic;

use crate::output::OutputContext;

/// Top-level CLI error type.
///
/// ## Invariants
/// - Every variant maps to a specific exit code.
/// - Every variant carries enough context for a four-part error message.
///
/// ## Error codes (per Doc 10 C07-2, Doc 09 G-08)
/// - E0001–E0099: General CLI errors
/// - E0100–E0199: Contract errors
/// - E0200–E0299: Linking errors
/// - E0300–E0399: Resource errors
/// - E0400–E0499: Reactor errors
/// - E0500–E0599: Security errors
/// - E0600–E0699: Packaging errors
/// - E0700–E0799: Configuration errors
#[derive(Debug)]
pub enum CliError {
    /// Configuration file not found or invalid.
    Config {
        /// What went wrong.
        detail: String,
        /// File path (if applicable).
        file: Option<String>,
        /// How to fix it.
        suggestion: String,
    },
    /// WIT contract validation failure.
    Contract {
        /// What went wrong.
        detail: String,
        /// Detailed diagnostic messages.
        diagnostics: Vec<String>,
    },
    /// Pipeline linking/composition failure.
    Link {
        /// What went wrong.
        detail: String,
        /// Detailed diagnostic messages.
        diagnostics: Vec<String>,
    },
    /// Runtime execution failure.
    Runtime {
        /// What went wrong.
        detail: String,
        /// Additional context.
        context: Option<String>,
    },
    /// Packaging or publish failure.
    Packaging {
        /// What went wrong.
        detail: String,
        /// How to fix it.
        suggestion: String,
    },
    /// Security/capability failure.
    Security {
        /// What went wrong.
        detail: String,
        /// How to fix it.
        suggestion: String,
    },
    /// Environment issue (missing tool, wrong version).
    #[allow(dead_code)]
    Environment {
        /// What went wrong.
        detail: String,
        /// How to fix it.
        fix: String,
    },
    /// Filesystem error (can't create directory, can't write file).
    Io {
        /// What went wrong.
        detail: String,
        /// File path (if applicable).
        path: Option<String>,
    },
    /// Generic internal error (should not normally reach users).
    #[allow(dead_code)]
    Internal {
        /// What went wrong.
        detail: String,
    },
    /// Command not yet implemented.
    #[allow(dead_code)]
    NotImplemented {
        /// Command name.
        command: String,
    },
}

impl CliError {
    /// Map this error to a process exit code.
    ///
    /// - Returns 1 for command failures (validation, runtime, packaging).
    /// - Returns 2 for usage errors (bad config, missing files).
    /// - Returns 3 for environment errors (missing tools).
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Config { .. } => 2,
            Self::Contract { .. } => 1,
            Self::Link { .. } => 1,
            Self::Runtime { .. } => 1,
            Self::Packaging { .. } => 1,
            Self::Security { .. } => 1,
            Self::Environment { .. } => 3,
            Self::Io { .. } => 2,
            Self::Internal { .. } => 1,
            Self::NotImplemented { .. } => 1,
        }
    }

    /// Render this error to the terminal.
    ///
    /// COLD PATH — called at most once per invocation.
    pub fn render(&self, ctx: &OutputContext) {
        use crate::cli::OutputFormat;
        match ctx.format {
            OutputFormat::Json => {
                let err_obj = self.to_json_value();
                crate::output::json::print_json(&err_obj);
            }
            OutputFormat::Human => {
                diagnostic::render_cli_error(ctx, self);
            }
        }
    }

    /// Convert to a JSON-serializable value.
    fn to_json_value(&self) -> serde_json::Value {
        match self {
            Self::Config {
                detail,
                file,
                suggestion,
            } => serde_json::json!({
                "error": true,
                "category": "config",
                "detail": detail,
                "file": file,
                "suggestion": suggestion,
            }),
            Self::Contract {
                detail,
                diagnostics,
            } => serde_json::json!({
                "error": true,
                "category": "contract",
                "detail": detail,
                "diagnostics": diagnostics,
            }),
            Self::Link {
                detail,
                diagnostics,
            } => serde_json::json!({
                "error": true,
                "category": "link",
                "detail": detail,
                "diagnostics": diagnostics,
            }),
            Self::Runtime { detail, context } => serde_json::json!({
                "error": true,
                "category": "runtime",
                "detail": detail,
                "context": context,
            }),
            Self::Packaging { detail, suggestion } => serde_json::json!({
                "error": true,
                "category": "packaging",
                "detail": detail,
                "suggestion": suggestion,
            }),
            Self::Security { detail, suggestion } => serde_json::json!({
                "error": true,
                "category": "security",
                "detail": detail,
                "suggestion": suggestion,
            }),
            Self::Environment { detail, fix } => serde_json::json!({
                "error": true,
                "category": "environment",
                "detail": detail,
                "fix": fix,
            }),
            Self::Io { detail, path } => serde_json::json!({
                "error": true,
                "category": "io",
                "detail": detail,
                "path": path,
            }),
            Self::Internal { detail } => serde_json::json!({
                "error": true,
                "category": "internal",
                "detail": detail,
            }),
            Self::NotImplemented { command } => serde_json::json!({
                "error": true,
                "category": "not_implemented",
                "detail": format!("Command '{command}' is not yet implemented (Part B)"),
            }),
        }
    }
}

impl From<torvyn_types::TorvynError> for CliError {
    fn from(err: torvyn_types::TorvynError) -> Self {
        match err {
            torvyn_types::TorvynError::Config(e) => CliError::Config {
                detail: e.to_string(),
                file: None,
                suggestion: "Check your Torvyn.toml for errors.".into(),
            },
            torvyn_types::TorvynError::Contract(e) => CliError::Contract {
                detail: e.to_string(),
                diagnostics: vec![],
            },
            torvyn_types::TorvynError::Link(e) => CliError::Link {
                detail: e.to_string(),
                diagnostics: vec![],
            },
            torvyn_types::TorvynError::Resource(e) => CliError::Runtime {
                detail: e.to_string(),
                context: Some("resource error".into()),
            },
            torvyn_types::TorvynError::Reactor(e) => CliError::Runtime {
                detail: e.to_string(),
                context: Some("reactor error".into()),
            },
            torvyn_types::TorvynError::Security(e) => CliError::Security {
                detail: e.to_string(),
                suggestion: "Check capability grants in your security configuration.".into(),
            },
            torvyn_types::TorvynError::Packaging(e) => CliError::Packaging {
                detail: e.to_string(),
                suggestion: "Check your packaging configuration.".into(),
            },
            torvyn_types::TorvynError::Process(e) => CliError::Runtime {
                detail: e.to_string(),
                context: Some("component process error".into()),
            },
            torvyn_types::TorvynError::Engine(e) => CliError::Runtime {
                detail: e.to_string(),
                context: Some("engine error".into()),
            },
            torvyn_types::TorvynError::Io(e) => CliError::Io {
                detail: e.to_string(),
                path: None,
            },
        }
    }
}

impl From<std::io::Error> for CliError {
    fn from(err: std::io::Error) -> Self {
        CliError::Io {
            detail: err.to_string(),
            path: None,
        }
    }
}
