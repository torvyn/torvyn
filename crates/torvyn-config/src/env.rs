//! Environment variable interpolation for configuration strings.
//!
//! Supports `${VAR_NAME}` syntax. Undefined variables produce errors
//! rather than silent empty strings, to prevent misconfiguration.
//!
//! Per Doc 07 Section 4.4: env vars override project manifest values
//! but are below CLI flags in precedence.

use crate::error::ConfigParseError;

/// Interpolate `${VAR_NAME}` references in a string with environment
/// variable values.
///
/// # COLD PATH — called during config loading.
///
/// # Errors
/// Returns `Err(ConfigParseError)` if a referenced environment variable
/// is not set.
///
/// # Examples
/// ```
/// # std::env::set_var("TEST_PORT", "8080");
/// use torvyn_config::interpolate_env;
///
/// let result = interpolate_env("host:${TEST_PORT}", "Torvyn.toml", "some.key").unwrap();
/// assert_eq!(result, "host:8080");
/// # std::env::remove_var("TEST_PORT");
/// ```
pub fn interpolate_env(
    input: &str,
    file: &str,
    key_path: &str,
) -> Result<String, ConfigParseError> {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut var_name = String::new();
            let mut found_close = false;

            for c2 in chars.by_ref() {
                if c2 == '}' {
                    found_close = true;
                    break;
                }
                var_name.push(c2);
            }

            if !found_close {
                // Unclosed ${, treat literally
                result.push('$');
                result.push('{');
                result.push_str(&var_name);
                continue;
            }

            if var_name.is_empty() {
                result.push_str("${}");
                continue;
            }

            match std::env::var(&var_name) {
                Ok(value) => result.push_str(&value),
                Err(_) => {
                    return Err(ConfigParseError::env_var_not_found(
                        file, key_path, &var_name,
                    ));
                }
            }
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

/// Resolve TORVYN_* environment variables into configuration overrides.
///
/// Maps `TORVYN_RUNTIME_WORKER_THREADS=8` to key path
/// `runtime.worker.threads` with value `"8"`.
///
/// # COLD PATH — called once during config loading.
///
/// # Examples
/// ```
/// # std::env::set_var("TORVYN_RUNTIME_WORKER_THREADS", "8");
/// use torvyn_config::collect_env_overrides;
///
/// let overrides = collect_env_overrides();
/// assert!(overrides.contains_key("runtime.worker.threads"));
/// assert_eq!(overrides["runtime.worker.threads"], "8");
/// # std::env::remove_var("TORVYN_RUNTIME_WORKER_THREADS");
/// ```
pub fn collect_env_overrides() -> std::collections::BTreeMap<String, String> {
    let mut overrides = std::collections::BTreeMap::new();

    for (key, value) in std::env::vars() {
        if let Some(suffix) = key.strip_prefix("TORVYN_") {
            let config_key = suffix.to_ascii_lowercase().replace('_', ".");
            // Collapse double dots from consecutive underscores
            let config_key = config_key.replace("..", ".");
            overrides.insert(config_key, value);
        }
    }

    overrides
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_interpolate_no_vars() {
        let result = interpolate_env("plain string", "f", "k").unwrap();
        assert_eq!(result, "plain string");
    }

    #[test]
    fn test_interpolate_single_var() {
        std::env::set_var("TORVYN_TEST_VAR_1", "hello");
        let result = interpolate_env("say ${TORVYN_TEST_VAR_1}", "f", "k").unwrap();
        assert_eq!(result, "say hello");
        std::env::remove_var("TORVYN_TEST_VAR_1");
    }

    #[test]
    fn test_interpolate_multiple_vars() {
        std::env::set_var("TORVYN_TEST_A", "foo");
        std::env::set_var("TORVYN_TEST_B", "bar");
        let result = interpolate_env("${TORVYN_TEST_A}-${TORVYN_TEST_B}", "f", "k").unwrap();
        assert_eq!(result, "foo-bar");
        std::env::remove_var("TORVYN_TEST_A");
        std::env::remove_var("TORVYN_TEST_B");
    }

    #[test]
    fn test_interpolate_missing_var_returns_error() {
        let result = interpolate_env("${DEFINITELY_NOT_SET_12345}", "Torvyn.toml", "some.key");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "E0705");
    }

    #[test]
    fn test_interpolate_empty_braces_literal() {
        let result = interpolate_env("${}", "f", "k").unwrap();
        assert_eq!(result, "${}");
    }

    #[test]
    fn test_interpolate_unclosed_brace_literal() {
        let result = interpolate_env("${unclosed", "f", "k").unwrap();
        assert_eq!(result, "${unclosed");
    }

    #[test]
    fn test_collect_env_overrides() {
        std::env::set_var("TORVYN_RUNTIME_WORKER_THREADS", "8");
        let overrides = collect_env_overrides();
        assert_eq!(
            overrides.get("runtime.worker.threads"),
            Some(&"8".to_owned())
        );
        std::env::remove_var("TORVYN_RUNTIME_WORKER_THREADS");
    }

    #[test]
    fn test_collect_env_overrides_ignores_non_torvyn() {
        std::env::set_var("NOT_TORVYN_VAR", "ignored");
        let overrides = collect_env_overrides();
        assert!(!overrides.contains_key("not.torvyn.var"));
        std::env::remove_var("NOT_TORVYN_VAR");
    }
}
