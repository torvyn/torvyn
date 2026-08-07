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
        for &kind in TemplateKind::ALL {
            let t = get_template(kind);
            assert!(!t.files.is_empty(), "Template {kind:?} has no files");
        }
    }

    /// Expand a template the way `torvyn init` does and parse the manifest it
    /// produced.
    fn scaffolded_manifest(
        kind: TemplateKind,
        project_name: &str,
    ) -> torvyn_config::ComponentManifest {
        let template = get_template(kind);
        let vars = TemplateVars::new(project_name, "0.1.0");
        let toml = template
            .files
            .iter()
            .find(|f| f.relative_path == Path::new("Torvyn.toml"))
            .map(|f| substitute(&f.content, &vars))
            .unwrap_or_else(|| panic!("template {kind:?} generates no Torvyn.toml"));

        torvyn_config::ComponentManifest::from_toml_str(&toml, "Torvyn.toml").unwrap_or_else(
            |errors| panic!("template {kind:?} generates an invalid manifest: {errors:?}\n{toml}"),
        )
    }

    /// Every template must generate a manifest the runtime accepts. A template
    /// whose manifest does not parse fails at the user's first command.
    #[test]
    fn every_template_generates_a_valid_manifest() {
        for &kind in TemplateKind::ALL {
            let manifest = scaffolded_manifest(kind, "my-project");
            assert_eq!(manifest.torvyn.name, "my-project");
        }
    }

    /// `torvyn init` prints `torvyn run` as its third step. A template that
    /// declares components but no flow cannot honour that: `run` fails with
    /// "no pipeline", telling the user to hand-write a section into a file
    /// generated seconds earlier. This is what made seven of the eight
    /// templates that once shipped unrunnable.
    #[test]
    fn every_template_that_scaffolds_components_defines_a_flow() {
        for &kind in TemplateKind::ALL {
            let manifest = scaffolded_manifest(kind, "my-project");
            if kind.scaffolds_flow() {
                assert!(
                    manifest.has_flows(),
                    "template {kind:?} declares components but no flow, so `torvyn run` fails"
                );
            } else {
                assert!(
                    manifest.components.is_empty(),
                    "template {kind:?} declares components without a flow to place them in"
                );
            }
        }
    }

    /// A flow node's `component` must name a component the manifest declares.
    /// The names are threaded through the project name and the companion
    /// components, so a substitution mistake would leave a node pointing at
    /// nothing — and `torvyn run` reporting an unresolvable component rather
    /// than a template bug.
    #[test]
    fn every_flow_node_names_a_declared_component() {
        for &kind in TemplateKind::ALL.iter().filter(|k| k.scaffolds_flow()) {
            let manifest = scaffolded_manifest(kind, "my-project");
            let declared: Vec<&str> = manifest
                .components
                .iter()
                .map(|c| c.name.as_str())
                .collect();

            for (flow_name, raw) in &manifest.flow {
                // The manifest keeps flow tables as raw TOML; the commands
                // deserialize them into `FlowDef`, and so does this, so a
                // template that generates a shape the commands cannot read
                // fails here rather than at the user's first `torvyn run`.
                let flow: torvyn_config::FlowDef = raw.clone().try_into().unwrap_or_else(|e| {
                    panic!("template {kind:?} flow \"{flow_name}\" does not deserialize: {e}")
                });
                assert!(
                    !flow.nodes.is_empty(),
                    "template {kind:?} flow \"{flow_name}\" has no nodes"
                );
                assert!(
                    !flow.edges.is_empty(),
                    "template {kind:?} flow \"{flow_name}\" has no edges, so its nodes are \
                     not connected"
                );
                for (node_name, node) in &flow.nodes {
                    assert!(
                        declared.contains(&node.component.as_str()),
                        "template {kind:?}: flow node \"{node_name}\" names component \
                         \"{}\", which the manifest does not declare (declared: {declared:?})",
                        node.component
                    );
                }
            }
        }
    }

    /// Component names must be unique, or the manifest cannot be resolved.
    /// The user's component is named after the project and the companions have
    /// fixed names, so this is what `TemplateKind::name_collides_with_companion`
    /// protects at `init` time.
    #[test]
    fn scaffolded_component_names_are_unique() {
        for &kind in TemplateKind::ALL {
            let manifest = scaffolded_manifest(kind, "my-project");
            let mut seen = std::collections::BTreeSet::new();
            for component in &manifest.components {
                assert!(
                    seen.insert(component.name.as_str()),
                    "template {kind:?} declares two components named \"{}\"",
                    component.name
                );
            }
        }
    }

    /// A component that prints without `stdio:stdout` writes into a deny-all
    /// sandbox and its output vanishes. Every scaffolded pipeline ends in a
    /// sink that prints, so every scaffolded flow must grant it.
    #[test]
    fn every_scaffolded_flow_grants_stdout_to_its_sink() {
        for &kind in TemplateKind::ALL.iter().filter(|k| k.scaffolds_flow()) {
            let manifest = scaffolded_manifest(kind, "my-project");
            let granted = manifest
                .security
                .grants
                .get("sink")
                .map(|g| g.capabilities.clone())
                .unwrap_or_default();
            assert!(
                granted.iter().any(|c| c == "stdio:stdout"),
                "template {kind:?} does not grant stdio:stdout to its sink node, so the \
                 pipeline it scaffolds produces no visible output"
            );
        }
    }

    /// Every file a template names must be somewhere the project expects it.
    /// A component declared at `components/sink` with no files there fails at
    /// `torvyn build`, long after `init` reported success.
    #[test]
    fn every_declared_component_has_scaffolded_files() {
        for &kind in TemplateKind::ALL {
            let template = get_template(kind);
            let manifest = scaffolded_manifest(kind, "my-project");
            let paths: Vec<String> = template
                .files
                .iter()
                .map(|f| f.relative_path.to_string_lossy().replace('\\', "/"))
                .collect();

            for component in &manifest.components {
                let dir = component.path.trim_end_matches('/');
                let prefix = if dir == "." {
                    String::new()
                } else {
                    format!("{dir}/")
                };
                for required in ["Cargo.toml", "src/lib.rs"] {
                    let expected = format!("{prefix}{required}");
                    assert!(
                        paths.contains(&expected),
                        "template {kind:?} declares component \"{}\" at \"{dir}\" but \
                         scaffolds no {expected}",
                        component.name
                    );
                }
            }
        }
    }
}
