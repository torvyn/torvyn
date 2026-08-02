//! Resolving a flow node's `component` value to a loadable artifact.
//!
//! A manifest has two halves that describe the same component:
//!
//! ```toml
//! [[component]]
//! name = "source"
//! path = "components/source"
//!
//! [flow.main.nodes.source]
//! component = "source"
//! ```
//!
//! The declaration says where the source lives; the flow node says which
//! component the stage runs. Joining them is this module's job. Without it a
//! node's `component` value reaches [`load_component_bytes`] verbatim, which
//! accepts only URIs — so a manifest written the way every example and every
//! scaffolded project writes it fails with "unsupported component reference
//! scheme", naming the component as though it were a malformed URI.
//!
//! [`load_component_bytes`]: crate::instantiate

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use torvyn_config::manifest::ComponentDecl;

/// Directory, relative to the project root, where `torvyn build` places the
/// components it compiles.
///
/// Deliberately inside `.torvyn/`, which the scaffolded `.gitignore` already
/// excludes and which `torvyn bench` already uses for its reports. Keeping the
/// convention in one place is what lets `run` find an artifact from the
/// manifest alone, without parsing a component's own build files to learn what
/// its compiler decided to call the output.
pub const BUILD_DIR: &str = ".torvyn/build";

/// The file name `torvyn build` writes for a component, and `run` looks for.
#[must_use]
pub fn artifact_file_name(component_name: &str) -> String {
    format!("{component_name}.wasm")
}

/// Absolute path to a component's built artifact.
#[must_use]
pub fn artifact_path(project_root: &Path, component_name: &str) -> PathBuf {
    project_root
        .join(BUILD_DIR)
        .join(artifact_file_name(component_name))
}

/// Why a node's component could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// The node names a component the manifest does not declare.
    UnknownComponent {
        /// The name the node asked for.
        requested: String,
        /// Names the manifest does declare, for the diagnostic.
        declared: Vec<String>,
    },
    /// The component is declared but has not been built.
    NotBuilt {
        /// Component name.
        name: String,
        /// Where the artifact was expected.
        expected: PathBuf,
    },
    /// The value looks like a URI but uses a scheme that cannot be loaded.
    UnsupportedScheme {
        /// The value as written in the manifest.
        value: String,
    },
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownComponent {
                requested,
                declared,
            } => {
                write!(
                    f,
                    "no component named '{requested}' is declared in the manifest"
                )?;
                if declared.is_empty() {
                    write!(
                        f,
                        ". Add a [[component]] entry with name = \"{requested}\" and the path to \
                         its source, or reference the component directly as \
                         \"file://<path-to-.wasm>\""
                    )
                } else {
                    write!(f, ". Declared components: {}", declared.join(", "))
                }
            }
            Self::NotBuilt { name, expected } => write!(
                f,
                "component '{name}' has not been built — no artifact at {}. Run `torvyn build` \
                 first",
                expected.display()
            ),
            Self::UnsupportedScheme { value } => write!(
                f,
                "unsupported component reference scheme in '{value}'. Supported schemes are \
                 'file://<path>' and 'mock://<name>'; a bare name must match a [[component]] \
                 declaration"
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

/// Resolves flow-node component references against a project's declared
/// components.
///
/// Values that already carry a supported scheme pass through untouched, so a
/// manifest, test, or benchmark that points straight at a `.wasm` keeps
/// working exactly as before.
#[derive(Clone, Debug, Default)]
pub struct ComponentIndex {
    project_root: PathBuf,
    /// Declared component name to its source directory, relative to the
    /// project root. Retained for diagnostics.
    declared: BTreeMap<String, PathBuf>,
}

impl ComponentIndex {
    /// Build an index from a project root and its `[[component]]`
    /// declarations.
    #[must_use]
    pub fn new(project_root: impl Into<PathBuf>, declarations: &[ComponentDecl]) -> Self {
        let project_root = project_root.into();
        let declared = declarations
            .iter()
            .map(|decl| (decl.name.clone(), PathBuf::from(&decl.path)))
            .collect();
        Self {
            project_root,
            declared,
        }
    }

    /// An index that declares nothing.
    ///
    /// Resolution then accepts only URI references, which is the correct
    /// behaviour for a host driven entirely by programmatic flow definitions.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The project root paths are resolved against.
    #[must_use]
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Names of the declared components, sorted.
    #[must_use]
    pub fn declared_names(&self) -> Vec<String> {
        self.declared.keys().cloned().collect()
    }

    /// Source directory declared for a component, relative to the project
    /// root.
    #[must_use]
    pub fn source_dir(&self, name: &str) -> Option<PathBuf> {
        self.declared
            .get(name)
            .map(|relative| self.project_root.join(relative))
    }

    /// Resolve a flow node's `component` value to a reference
    /// [`instantiate_pipeline`](crate::instantiate_pipeline) can load.
    ///
    /// - A value with a supported scheme is returned unchanged.
    /// - A declared component name becomes a `file://` URI pointing at the
    ///   artifact `torvyn build` produces.
    ///
    /// # Errors
    /// Returns [`ResolveError`] when the name is not declared, when the
    /// component is declared but not yet built, or when the value carries an
    /// unrecognised scheme.
    pub fn resolve(&self, value: &str) -> Result<String, ResolveError> {
        if value.starts_with("file://") || value.starts_with("mock://") {
            return Ok(value.to_owned());
        }

        // Anything else containing a scheme separator was meant as a URI and
        // is a clearer error than "component not declared" would be.
        if value.contains("://") {
            return Err(ResolveError::UnsupportedScheme {
                value: value.to_owned(),
            });
        }

        if !self.declared.contains_key(value) {
            return Err(ResolveError::UnknownComponent {
                requested: value.to_owned(),
                declared: self.declared_names(),
            });
        }

        let artifact = artifact_path(&self.project_root, value);
        if !artifact.is_file() {
            return Err(ResolveError::NotBuilt {
                name: value.to_owned(),
                expected: artifact,
            });
        }

        Ok(format!("file://{}", artifact.display()))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(name: &str, path: &str) -> ComponentDecl {
        ComponentDecl {
            name: name.to_owned(),
            path: path.to_owned(),
            ..ComponentDecl::default()
        }
    }

    fn temp_project(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "torvyn-resolve-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp project dir");
        dir
    }

    #[test]
    fn uri_references_pass_through_untouched() {
        let index = ComponentIndex::new("/project", &[decl("source", "components/source")]);
        assert_eq!(
            index.resolve("file:///abs/path/thing.wasm").unwrap(),
            "file:///abs/path/thing.wasm"
        );
        assert_eq!(index.resolve("mock://anything").unwrap(), "mock://anything");
    }

    #[test]
    fn an_empty_index_still_accepts_uris() {
        let index = ComponentIndex::empty();
        assert_eq!(index.resolve("mock://x").unwrap(), "mock://x");
    }

    #[test]
    fn declared_name_resolves_to_the_built_artifact() {
        let root = temp_project("built");
        std::fs::create_dir_all(root.join(BUILD_DIR)).unwrap();
        std::fs::write(root.join(BUILD_DIR).join("source.wasm"), b"\0asm").unwrap();

        let index = ComponentIndex::new(&root, &[decl("source", "components/source")]);
        let resolved = index.resolve("source").expect("declared and built");
        assert_eq!(
            resolved,
            format!("file://{}", artifact_path(&root, "source").display())
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn declared_but_unbuilt_says_to_build() {
        let root = temp_project("unbuilt");
        let index = ComponentIndex::new(&root, &[decl("source", "components/source")]);

        let err = index.resolve("source").expect_err("artifact is absent");
        assert!(matches!(err, ResolveError::NotBuilt { .. }));
        let message = err.to_string();
        assert!(
            message.contains("torvyn build"),
            "the error must tell the user what to do: {message}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn undeclared_name_lists_what_is_declared() {
        let index = ComponentIndex::new(
            "/project",
            &[
                decl("source", "components/source"),
                decl("sink", "components/sink"),
            ],
        );

        let err = index.resolve("typo").expect_err("not declared");
        assert!(matches!(err, ResolveError::UnknownComponent { .. }));
        let message = err.to_string();
        assert!(message.contains("source"), "{message}");
        assert!(message.contains("sink"), "{message}");
    }

    #[test]
    fn undeclared_name_with_no_declarations_explains_both_options() {
        let index = ComponentIndex::empty();
        let message = index.resolve("source").unwrap_err().to_string();
        assert!(message.contains("[[component]]"), "{message}");
        assert!(message.contains("file://"), "{message}");
    }

    #[test]
    fn an_unrecognised_scheme_is_reported_as_a_scheme_problem() {
        let index = ComponentIndex::new("/project", &[decl("source", "components/source")]);
        let err = index.resolve("oci://registry/thing:1").unwrap_err();
        assert!(matches!(err, ResolveError::UnsupportedScheme { .. }));
    }

    #[test]
    fn source_dir_joins_against_the_project_root() {
        let index = ComponentIndex::new("/project", &[decl("source", "components/source")]);
        assert_eq!(
            index.source_dir("source"),
            Some(PathBuf::from("/project/components/source"))
        );
        assert_eq!(index.source_dir("absent"), None);
    }

    #[test]
    fn artifact_layout_is_stable() {
        // `torvyn build` writes here and `run` reads here; if these ever
        // disagree the first-run path breaks silently, so the convention is
        // pinned by a test rather than by convention alone.
        assert_eq!(artifact_file_name("greeter"), "greeter.wasm");
        assert_eq!(
            artifact_path(Path::new("/p"), "greeter"),
            PathBuf::from("/p/.torvyn/build/greeter.wasm")
        );
    }
}
