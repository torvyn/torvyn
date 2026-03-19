//! WIT file loading and parsing.
//!
//! This module wraps the `wit-parser` crate (from the Bytecode Alliance's
//! `wasm-tools` project) to provide Torvyn-specific error handling and
//! a stable internal API.
//!
//! # Architecture
//!
//! The `WitParser` trait defines the interface that the rest of the crate
//! uses. `WitParserImpl` provides the concrete implementation backed by
//! `wit-parser`. This abstraction isolates the crate from `wit-parser`'s
//! rapidly-evolving API (major version bumps on every `wasm-tools` release).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::errors::{DiagnosticBuilder, ErrorCode, ValidationResult};

// COLD PATH — parsing happens during `torvyn check`, not on the hot path.

/// Parsed representation of a WIT package.
///
/// This is a simplified, stable view of a WIT package extracted from
/// `wit-parser::Resolve`. It contains only the information needed by
/// Torvyn's validation and compatibility logic.
///
/// Invariants:
/// - `name` is a valid WIT package name (namespace:name).
/// - `version` is a valid semver version if present.
/// - All interface and world names are non-empty.
#[derive(Debug, Clone)]
pub struct ParsedPackage {
    /// Full package name (e.g., "torvyn:streaming").
    pub name: String,
    /// Package version (e.g., "0.1.0"). None if unversioned.
    pub version: Option<semver::Version>,
    /// Interfaces defined in this package, keyed by name.
    pub interfaces: HashMap<String, ParsedInterface>,
    /// Worlds defined in this package, keyed by name.
    pub worlds: HashMap<String, ParsedWorld>,
    /// Source files that contributed to this package.
    pub source_files: Vec<PathBuf>,
}

/// A parsed WIT interface.
///
/// Invariants:
/// - `name` is non-empty and a valid WIT identifier.
/// - Function names within `functions` are unique.
#[derive(Debug, Clone)]
pub struct ParsedInterface {
    /// Interface name.
    pub name: String,
    /// Functions defined in this interface, keyed by name.
    pub functions: HashMap<String, ParsedFunction>,
    /// Type definitions in this interface.
    pub types: HashMap<String, ParsedTypeDef>,
    /// Resource types defined in this interface.
    pub resources: Vec<String>,
}

/// A parsed WIT function signature.
#[derive(Debug, Clone)]
pub struct ParsedFunction {
    /// Function name.
    pub name: String,
    /// Parameter types as WIT type strings.
    pub params: Vec<ParsedParam>,
    /// Return type as a WIT type string. None for void functions.
    pub result: Option<ParsedType>,
}

/// A parsed function parameter.
#[derive(Debug, Clone)]
pub struct ParsedParam {
    /// Parameter name.
    pub name: String,
    /// Parameter type.
    pub typ: ParsedType,
}

/// A simplified representation of a WIT type.
///
/// This captures enough information for compatibility checking without
/// reproducing the full `wit-parser` type system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedType {
    /// A primitive type (u8, u16, u32, u64, s8, s16, s32, s64, f32, f64, bool, char, string).
    Primitive(String),
    /// A named type reference (record, variant, enum, flags, resource).
    Named(String),
    /// list\<T\>
    List(Box<ParsedType>),
    /// option\<T\>
    Option(Box<ParsedType>),
    /// result\<T, E\>
    Result {
        /// The ok type, if any.
        ok: Option<Box<ParsedType>>,
        /// The error type, if any.
        err: Option<Box<ParsedType>>,
    },
    /// tuple\<T...\>
    Tuple(Vec<ParsedType>),
    /// borrow\<T\>
    Borrow(Box<ParsedType>),
    /// own\<T\> (explicit ownership)
    Own(Box<ParsedType>),
}

/// A parsed type definition.
#[derive(Debug, Clone)]
pub struct ParsedTypeDef {
    /// Type name.
    pub name: String,
    /// Kind of type definition.
    pub kind: ParsedTypeDefKind,
}

/// Kind of type definition.
#[derive(Debug, Clone)]
pub enum ParsedTypeDefKind {
    /// A record with named fields.
    Record(Vec<ParsedField>),
    /// A variant with named cases.
    Variant(Vec<ParsedCase>),
    /// An enum with named cases (no payloads).
    Enum(Vec<String>),
    /// A flags type.
    Flags(Vec<String>),
    /// A resource type.
    Resource,
    /// A type alias.
    Alias(ParsedType),
}

/// A record field.
#[derive(Debug, Clone)]
pub struct ParsedField {
    /// Field name.
    pub name: String,
    /// Field type.
    pub typ: ParsedType,
}

/// A variant case.
#[derive(Debug, Clone)]
pub struct ParsedCase {
    /// Case name.
    pub name: String,
    /// Case payload type, if any.
    pub typ: Option<ParsedType>,
}

/// A parsed WIT world.
///
/// Invariants:
/// - `name` is non-empty and a valid WIT identifier.
#[derive(Debug, Clone)]
pub struct ParsedWorld {
    /// World name.
    pub name: String,
    /// Imported interfaces, keyed by interface path (e.g., "torvyn:streaming/types").
    pub imports: HashMap<String, WorldImportExport>,
    /// Exported interfaces, keyed by interface path.
    pub exports: HashMap<String, WorldImportExport>,
}

/// An import or export in a world.
#[derive(Debug, Clone)]
pub enum WorldImportExport {
    /// A named interface import/export.
    Interface(String),
    /// A function import/export (rare in Torvyn).
    Function(ParsedFunction),
}

/// Trait defining the WIT parsing interface.
///
/// This abstracts over the concrete `wit-parser` implementation to
/// enable testing with mock parsers and isolation from API churn.
pub trait WitParser: Send + Sync {
    /// Parse all WIT files in a directory and return the parsed packages.
    ///
    /// # Preconditions
    /// - `dir` is a valid directory path containing `.wit` files.
    ///
    /// # Postconditions
    /// - On success: returns all packages found in the directory.
    /// - On failure: returns a `ValidationResult` with parse error diagnostics.
    ///
    /// # Errors
    /// - `ErrorCode::WitSyntaxError` if any file has syntax errors.
    /// - `ErrorCode::WitUnresolvedReference` if types/interfaces cannot be resolved.
    /// - `ErrorCode::WitPackageDeclaration` if package declarations are missing.
    ///
    /// # COLD PATH
    fn parse_directory(&self, dir: &Path) -> Result<Vec<ParsedPackage>, ValidationResult>;

    /// Parse a single WIT file and return the parsed package.
    ///
    /// # COLD PATH
    fn parse_file(&self, file: &Path) -> Result<Vec<ParsedPackage>, ValidationResult>;

    /// Parse WIT content from a string (for testing).
    ///
    /// # COLD PATH
    fn parse_str(&self, name: &str, contents: &str)
        -> Result<Vec<ParsedPackage>, ValidationResult>;
}

/// Concrete `wit-parser`-backed implementation.
///
/// LLI DEVIATION: Adapted from LLI v0.245 API assumptions to actual
/// `wit-parser` API. Key differences:
/// - `push_dir` returns `(PackageId, PackageSourceMap)`, not `(Vec<PackageId>, SourceMap)`
/// - `Function.params` is `Vec<Param>` with `.name`/`.ty` fields
/// - `WorldItem::Interface` has named fields `{ id, stability, span }`
#[cfg(feature = "wit-parser-backend")]
pub struct WitParserImpl;

#[cfg(feature = "wit-parser-backend")]
impl WitParserImpl {
    /// Create a new WIT parser instance.
    pub fn new() -> Self {
        Self
    }

    /// Convert a `wit_parser::Resolve` into our stable `ParsedPackage` representation.
    ///
    /// # COLD PATH — allocations OK
    fn convert_resolve(
        &self,
        resolve: &wit_parser::Resolve,
        package_ids: &[wit_parser::PackageId],
        source_files: Vec<PathBuf>,
    ) -> Vec<ParsedPackage> {
        let mut packages = Vec::new();

        for &pkg_id in package_ids {
            let pkg = &resolve.packages[pkg_id];
            let name = format!("{}:{}", pkg.name.namespace, pkg.name.name);
            let version = pkg.name.version.clone();

            let mut interfaces = HashMap::new();
            for (iface_name, &iface_id) in &pkg.interfaces {
                let iface = &resolve.interfaces[iface_id];
                let parsed_iface = self.convert_interface(resolve, iface);
                interfaces.insert(iface_name.clone(), parsed_iface);
            }

            let mut worlds = HashMap::new();
            for (world_name, &world_id) in &pkg.worlds {
                let world = &resolve.worlds[world_id];
                let parsed_world = self.convert_world(resolve, world);
                worlds.insert(world_name.clone(), parsed_world);
            }

            packages.push(ParsedPackage {
                name,
                version,
                interfaces,
                worlds,
                source_files: source_files.clone(),
            });
        }

        packages
    }

    /// Convert a `wit_parser::Interface` to `ParsedInterface`.
    fn convert_interface(
        &self,
        resolve: &wit_parser::Resolve,
        iface: &wit_parser::Interface,
    ) -> ParsedInterface {
        let name = iface.name.clone().unwrap_or_default();
        let mut functions = HashMap::new();
        let mut types = HashMap::new();
        let mut resources = Vec::new();

        for (func_name, func) in &iface.functions {
            functions.insert(func_name.clone(), self.convert_function(resolve, func));
        }

        for (type_name, &type_id) in &iface.types {
            let type_def = &resolve.types[type_id];
            match &type_def.kind {
                wit_parser::TypeDefKind::Resource => {
                    resources.push(type_name.clone());
                }
                _ => {
                    types.insert(type_name.clone(), self.convert_type_def(resolve, type_def));
                }
            }
        }

        ParsedInterface {
            name,
            functions,
            types,
            resources,
        }
    }

    /// Convert a `wit_parser::Function` to `ParsedFunction`.
    // LLI DEVIATION: In wit-parser v0.227, Function.params is Vec<(String, Type)>,
    // not the Vec<Param> struct found in later versions.
    fn convert_function(
        &self,
        resolve: &wit_parser::Resolve,
        func: &wit_parser::Function,
    ) -> ParsedFunction {
        let params = func
            .params
            .iter()
            .map(|(name, ty)| ParsedParam {
                name: name.clone(),
                typ: self.convert_type(resolve, ty),
            })
            .collect();

        let result = func
            .result
            .as_ref()
            .map(|typ| self.convert_type(resolve, typ));

        ParsedFunction {
            name: func.name.clone(),
            params,
            result,
        }
    }

    /// Convert a `wit_parser::Type` to our `ParsedType`.
    fn convert_type(&self, resolve: &wit_parser::Resolve, typ: &wit_parser::Type) -> ParsedType {
        match typ {
            wit_parser::Type::Bool => ParsedType::Primitive("bool".into()),
            wit_parser::Type::U8 => ParsedType::Primitive("u8".into()),
            wit_parser::Type::U16 => ParsedType::Primitive("u16".into()),
            wit_parser::Type::U32 => ParsedType::Primitive("u32".into()),
            wit_parser::Type::U64 => ParsedType::Primitive("u64".into()),
            wit_parser::Type::S8 => ParsedType::Primitive("s8".into()),
            wit_parser::Type::S16 => ParsedType::Primitive("s16".into()),
            wit_parser::Type::S32 => ParsedType::Primitive("s32".into()),
            wit_parser::Type::S64 => ParsedType::Primitive("s64".into()),
            wit_parser::Type::F32 => ParsedType::Primitive("f32".into()),
            wit_parser::Type::F64 => ParsedType::Primitive("f64".into()),
            wit_parser::Type::Char => ParsedType::Primitive("char".into()),
            wit_parser::Type::String => ParsedType::Primitive("string".into()),
            // LLI DEVIATION: ErrorContext variant added in wit-parser v0.227+
            wit_parser::Type::ErrorContext => ParsedType::Named("error-context".into()),
            wit_parser::Type::Id(id) => {
                let type_def = &resolve.types[*id];
                match &type_def.kind {
                    wit_parser::TypeDefKind::List(inner) => {
                        ParsedType::List(Box::new(self.convert_type(resolve, inner)))
                    }
                    wit_parser::TypeDefKind::Option(inner) => {
                        ParsedType::Option(Box::new(self.convert_type(resolve, inner)))
                    }
                    wit_parser::TypeDefKind::Result(r) => ParsedType::Result {
                        ok: r
                            .ok
                            .as_ref()
                            .map(|t| Box::new(self.convert_type(resolve, t))),
                        err: r
                            .err
                            .as_ref()
                            .map(|t| Box::new(self.convert_type(resolve, t))),
                    },
                    wit_parser::TypeDefKind::Tuple(t) => ParsedType::Tuple(
                        t.types
                            .iter()
                            .map(|inner| self.convert_type(resolve, inner))
                            .collect(),
                    ),
                    wit_parser::TypeDefKind::Handle(handle) => match handle {
                        wit_parser::Handle::Borrow(id) => {
                            let inner_name = resolve.types[*id]
                                .name
                                .clone()
                                .unwrap_or_else(|| format!("type-{}", id.index()));
                            ParsedType::Borrow(Box::new(ParsedType::Named(inner_name)))
                        }
                        wit_parser::Handle::Own(id) => {
                            let inner_name = resolve.types[*id]
                                .name
                                .clone()
                                .unwrap_or_else(|| format!("type-{}", id.index()));
                            ParsedType::Own(Box::new(ParsedType::Named(inner_name)))
                        }
                    },
                    _ => {
                        let name = type_def
                            .name
                            .clone()
                            .unwrap_or_else(|| format!("anon-type-{}", id.index()));
                        ParsedType::Named(name)
                    }
                }
            }
        }
    }

    /// Convert a `wit_parser::TypeDef` to `ParsedTypeDef`.
    fn convert_type_def(
        &self,
        resolve: &wit_parser::Resolve,
        type_def: &wit_parser::TypeDef,
    ) -> ParsedTypeDef {
        let name = type_def.name.clone().unwrap_or_default();
        let kind = match &type_def.kind {
            wit_parser::TypeDefKind::Record(r) => ParsedTypeDefKind::Record(
                r.fields
                    .iter()
                    .map(|f| ParsedField {
                        name: f.name.clone(),
                        typ: self.convert_type(resolve, &f.ty),
                    })
                    .collect(),
            ),
            wit_parser::TypeDefKind::Variant(v) => ParsedTypeDefKind::Variant(
                v.cases
                    .iter()
                    .map(|c| ParsedCase {
                        name: c.name.clone(),
                        typ: c.ty.as_ref().map(|t| self.convert_type(resolve, t)),
                    })
                    .collect(),
            ),
            wit_parser::TypeDefKind::Enum(e) => {
                ParsedTypeDefKind::Enum(e.cases.iter().map(|c| c.name.clone()).collect())
            }
            wit_parser::TypeDefKind::Flags(fl) => {
                ParsedTypeDefKind::Flags(fl.flags.iter().map(|f| f.name.clone()).collect())
            }
            wit_parser::TypeDefKind::Resource => ParsedTypeDefKind::Resource,
            _ => {
                // Type alias or other
                ParsedTypeDefKind::Alias(ParsedType::Primitive("unknown".into()))
            }
        };

        ParsedTypeDef { name, kind }
    }

    /// Convert a `wit_parser::World` to `ParsedWorld`.
    // LLI DEVIATION: WorldItem::Interface has named fields { id, stability, span }
    fn convert_world(
        &self,
        resolve: &wit_parser::Resolve,
        world: &wit_parser::World,
    ) -> ParsedWorld {
        let name = world.name.clone();
        let mut imports = HashMap::new();
        let mut exports = HashMap::new();

        for (key, item) in &world.imports {
            let key_name = self.world_key_to_string(resolve, key);
            let entry = match item {
                wit_parser::WorldItem::Interface { id, .. } => {
                    let iface = &resolve.interfaces[*id];
                    let iface_name = iface.name.clone().unwrap_or_default();
                    WorldImportExport::Interface(iface_name)
                }
                wit_parser::WorldItem::Function(func) => {
                    WorldImportExport::Function(self.convert_function(resolve, func))
                }
                wit_parser::WorldItem::Type(..) => continue,
            };
            imports.insert(key_name, entry);
        }

        for (key, item) in &world.exports {
            let key_name = self.world_key_to_string(resolve, key);
            let entry = match item {
                wit_parser::WorldItem::Interface { id, .. } => {
                    let iface = &resolve.interfaces[*id];
                    let iface_name = iface.name.clone().unwrap_or_default();
                    WorldImportExport::Interface(iface_name)
                }
                wit_parser::WorldItem::Function(func) => {
                    WorldImportExport::Function(self.convert_function(resolve, func))
                }
                wit_parser::WorldItem::Type(..) => continue,
            };
            exports.insert(key_name, entry);
        }

        ParsedWorld {
            name,
            imports,
            exports,
        }
    }

    /// Convert a `wit_parser::WorldKey` to a string.
    fn world_key_to_string(
        &self,
        resolve: &wit_parser::Resolve,
        key: &wit_parser::WorldKey,
    ) -> String {
        match key {
            wit_parser::WorldKey::Name(name) => name.clone(),
            wit_parser::WorldKey::Interface(id) => {
                let iface = &resolve.interfaces[*id];
                let pkg = iface
                    .package
                    .map(|pid| {
                        let p = &resolve.packages[pid];
                        format!("{}:{}", p.name.namespace, p.name.name)
                    })
                    .unwrap_or_default();
                let iface_name = iface.name.clone().unwrap_or_default();
                if pkg.is_empty() {
                    iface_name
                } else {
                    format!("{}/{}", pkg, iface_name)
                }
            }
        }
    }

    /// Convert a wit-parser error into our diagnostic format.
    // LLI DEVIATION: wit-parser returns anyhow::Error; we accept any Display+Debug
    // to avoid requiring anyhow as a direct dependency. The actual error type from
    // wit-parser is anyhow::Error which implements Display.
    fn convert_parse_error(
        &self,
        error: &dyn std::fmt::Display,
        source_hint: &Path,
    ) -> ValidationResult {
        let mut result = ValidationResult::new();

        let error_msg = format!("{:#}", error);

        result.push(
            DiagnosticBuilder::error(ErrorCode::WitSyntaxError, &error_msg)
                .location(source_hint, 1, 0, "error while parsing this file")
                .help("check the WIT file syntax against the WIT specification")
                .build(),
        );

        result
    }
}

#[cfg(feature = "wit-parser-backend")]
impl Default for WitParserImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "wit-parser-backend")]
impl WitParser for WitParserImpl {
    // LLI DEVIATION: push_dir returns (PackageId, PackageSourceMap) not (Vec<PackageId>, SourceMap).
    // PackageSourceMap does not expose source_files() in v0.227; we collect source files
    // from the directory listing instead.
    fn parse_directory(&self, dir: &Path) -> Result<Vec<ParsedPackage>, ValidationResult> {
        let mut resolve = wit_parser::Resolve::default();

        let (pkg_id, _source_map) = resolve
            .push_dir(dir)
            .map_err(|e| self.convert_parse_error(&e, dir))?;

        // Collect .wit files from the directory
        let source_files: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|ext| ext == "wit"))
                    .collect()
            })
            .unwrap_or_default();

        Ok(self.convert_resolve(&resolve, &[pkg_id], source_files))
    }

    // LLI DEVIATION: push_file returns PackageId not Vec<PackageId>
    fn parse_file(&self, file: &Path) -> Result<Vec<ParsedPackage>, ValidationResult> {
        let mut resolve = wit_parser::Resolve::default();

        let pkg_id = resolve
            .push_file(file)
            .map_err(|e| self.convert_parse_error(&e, file))?;

        Ok(self.convert_resolve(&resolve, &[pkg_id], vec![file.to_path_buf()]))
    }

    // LLI DEVIATION: push_str takes impl AsRef<Path> as first arg
    fn parse_str(
        &self,
        name: &str,
        contents: &str,
    ) -> Result<Vec<ParsedPackage>, ValidationResult> {
        let mut resolve = wit_parser::Resolve::default();

        let path = Path::new(name);
        let pkg_id = resolve
            .push_str(path, contents)
            .map_err(|e| self.convert_parse_error(&e, path))?;

        Ok(self.convert_resolve(&resolve, &[pkg_id], vec![PathBuf::from(name)]))
    }
}

/// A mock parser for testing validation logic without requiring `wit-parser`.
#[cfg(test)]
pub(crate) struct MockWitParser {
    pub packages: Vec<ParsedPackage>,
    pub should_fail: bool,
    pub error_message: String,
}

#[cfg(test)]
impl MockWitParser {
    pub fn with_packages(packages: Vec<ParsedPackage>) -> Self {
        Self {
            packages,
            should_fail: false,
            error_message: String::new(),
        }
    }

    pub fn failing(msg: &str) -> Self {
        Self {
            packages: Vec::new(),
            should_fail: true,
            error_message: msg.to_string(),
        }
    }
}

#[cfg(test)]
impl WitParser for MockWitParser {
    fn parse_directory(&self, _dir: &Path) -> Result<Vec<ParsedPackage>, ValidationResult> {
        if self.should_fail {
            let mut result = ValidationResult::new();
            result.push(
                DiagnosticBuilder::error(ErrorCode::WitSyntaxError, &self.error_message).build(),
            );
            Err(result)
        } else {
            Ok(self.packages.clone())
        }
    }

    fn parse_file(&self, _file: &Path) -> Result<Vec<ParsedPackage>, ValidationResult> {
        self.parse_directory(Path::new(""))
    }

    fn parse_str(
        &self,
        _name: &str,
        _contents: &str,
    ) -> Result<Vec<ParsedPackage>, ValidationResult> {
        self.parse_directory(Path::new(""))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_parser_success() {
        let parser = MockWitParser::with_packages(vec![ParsedPackage {
            name: "torvyn:streaming".into(),
            version: Some(semver::Version::new(0, 1, 0)),
            interfaces: HashMap::new(),
            worlds: HashMap::new(),
            source_files: vec![],
        }]);

        let result = parser.parse_directory(Path::new("wit/"));
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 1);
    }

    #[test]
    fn test_mock_parser_failure() {
        let parser = MockWitParser::failing("syntax error on line 42");

        let result = parser.parse_directory(Path::new("wit/"));
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert!(!diags.is_ok());
    }

    #[test]
    fn test_parsed_type_equality() {
        assert_eq!(
            ParsedType::Primitive("u64".into()),
            ParsedType::Primitive("u64".into())
        );
        assert_ne!(
            ParsedType::Primitive("u32".into()),
            ParsedType::Primitive("u64".into())
        );
    }
}
