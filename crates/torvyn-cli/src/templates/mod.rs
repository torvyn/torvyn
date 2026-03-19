//! Template registry and expansion for `torvyn init`.
//!
//! Templates are embedded in the binary. Each template provides a complete
//! set of files needed for a specific component pattern.

pub mod content;

use crate::cli::TemplateKind;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// A single file in a template, with its relative path and content.
#[derive(Debug, Clone)]
pub struct TemplateFile {
    /// Path relative to the project root.
    pub relative_path: PathBuf,
    /// File content with substitution tokens.
    pub content: String,
}

/// The complete set of files for a template.
#[derive(Debug, Clone)]
pub struct Template {
    /// Human-readable description.
    #[allow(dead_code)]
    pub description: String,
    /// Files to generate.
    pub files: Vec<TemplateFile>,
}

/// Substitution variables available to templates.
#[derive(Debug, Clone)]
pub struct TemplateVars {
    /// Project name (kebab-case).
    pub project_name: String,
    /// Component type (PascalCase).
    pub component_type: String,
    /// Date string.
    pub date: String,
    /// Torvyn CLI version.
    pub torvyn_version: String,
    /// Contract version.
    pub contract_version: String,
}

impl TemplateVars {
    /// Create template variables from the init arguments.
    ///
    /// COLD PATH — called once during init.
    pub fn new(project_name: &str, contract_version: &str) -> Self {
        Self {
            project_name: project_name.to_string(),
            component_type: to_pascal_case(project_name),
            date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            torvyn_version: env!("CARGO_PKG_VERSION").to_string(),
            contract_version: contract_version.to_string(),
        }
    }

    /// Build the substitution map.
    fn to_map(&self) -> HashMap<&'static str, &str> {
        let mut m = HashMap::new();
        m.insert("project_name", self.project_name.as_str());
        m.insert("component_type", self.component_type.as_str());
        m.insert("date", self.date.as_str());
        m.insert("torvyn_version", self.torvyn_version.as_str());
        m.insert("contract_version", self.contract_version.as_str());
        m
    }
}

/// Convert a kebab-case string to PascalCase.
///
/// # Examples
/// ```
/// # use torvyn_cli::templates::to_pascal_case;
/// assert_eq!(to_pascal_case("my-transform"), "MyTransform");
/// assert_eq!(to_pascal_case("hello"), "Hello");
/// assert_eq!(to_pascal_case("a-b-c"), "ABC");
/// ```
pub fn to_pascal_case(s: &str) -> String {
    s.split('-')
        .filter(|p| !p.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    upper + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect()
}

/// Apply variable substitution to a template string.
///
/// Replaces all `{{key}}` patterns with the corresponding value from `vars`.
pub fn substitute(template: &str, vars: &TemplateVars) -> String {
    let map = vars.to_map();
    let mut result = template.to_string();
    for (key, value) in &map {
        let token = format!("{{{{{key}}}}}");
        result = result.replace(&token, value);
    }
    result
}

/// Get the template for the given kind.
pub fn get_template(kind: TemplateKind) -> Template {
    match kind {
        TemplateKind::Transform => content::transform_template(),
        TemplateKind::Source => content::source_template(),
        TemplateKind::Sink => content::sink_template(),
        TemplateKind::Filter => content::filter_template(),
        TemplateKind::Router => content::router_template(),
        TemplateKind::Aggregator => content::aggregator_template(),
        TemplateKind::FullPipeline => content::full_pipeline_template(),
        TemplateKind::Empty => content::empty_template(),
    }
}

/// Expand a template into real files at the specified directory.
///
/// # Errors
/// - Returns `std::io::Error` if any file write fails.
pub fn expand_template(
    template: &Template,
    vars: &TemplateVars,
    target_dir: &Path,
) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut created_files = Vec::new();
    for tf in &template.files {
        let content = substitute(&tf.content, vars);
        let full_path = target_dir.join(&tf.relative_path);

        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&full_path, &content)?;
        created_files.push(tf.relative_path.clone());
    }
    Ok(created_files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_pascal_case_basic() {
        assert_eq!(to_pascal_case("my-transform"), "MyTransform");
    }

    #[test]
    fn test_to_pascal_case_single_word() {
        assert_eq!(to_pascal_case("hello"), "Hello");
    }

    #[test]
    fn test_to_pascal_case_multi_segment() {
        assert_eq!(to_pascal_case("a-b-c"), "ABC");
    }

    #[test]
    fn test_to_pascal_case_already_capitalized() {
        assert_eq!(to_pascal_case("My-Thing"), "MyThing");
    }

    #[test]
    fn test_substitute_basic() {
        let vars = TemplateVars::new("my-project", "0.1.0");
        let result = substitute("name = \"{{project_name}}\"", &vars);
        assert_eq!(result, "name = \"my-project\"");
    }

    #[test]
    fn test_substitute_multiple_vars() {
        let vars = TemplateVars::new("my-project", "0.1.0");
        let result = substitute("struct {{component_type}}; // v{{contract_version}}", &vars);
        assert_eq!(result, "struct MyProject; // v0.1.0");
    }

    #[test]
    fn test_substitute_unknown_token_preserved() {
        let vars = TemplateVars::new("x", "0.1.0");
        let result = substitute("{{unknown_token}}", &vars);
        assert_eq!(result, "{{unknown_token}}");
    }

    #[test]
    fn test_get_template_returns_nonempty() {
        for kind in [
            TemplateKind::Source,
            TemplateKind::Sink,
            TemplateKind::Transform,
            TemplateKind::Filter,
            TemplateKind::Empty,
            TemplateKind::FullPipeline,
        ] {
            let t = get_template(kind);
            assert!(!t.files.is_empty(), "Template {kind:?} has no files");
        }
    }
}
