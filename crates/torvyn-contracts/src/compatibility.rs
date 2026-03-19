//! Version compatibility engine.
//!
//! Implements the semver-based compatibility checking algorithm from
//! Doc 01 Section 6. Given two versions of a WIT package, determines
//! whether they are compatible and what changed.
//!
//! The algorithm is deterministic: same inputs always produce the
//! same compatibility verdict.

use std::collections::HashMap;

use crate::errors::{DiagnosticBuilder, ErrorCode, ValidationResult};
use crate::parser::{
    ParsedCase, ParsedField, ParsedInterface, ParsedPackage, ParsedType, ParsedTypeDefKind,
};

// COLD PATH — compatibility checking runs during `torvyn link`.

/// The result of comparing two versions of a WIT package.
///
/// Invariants:
/// - `changes` is non-empty if `verdict != Compatible`.
/// - If `verdict == Breaking`, at least one change has `severity == Breaking`.
#[derive(Debug, Clone)]
pub struct CompatibilityReport {
    /// Overall compatibility verdict.
    pub verdict: CompatibilityVerdict,
    /// Individual changes detected between versions.
    pub changes: Vec<ChangeEntry>,
    /// The older version being compared.
    pub old_version: semver::Version,
    /// The newer version being compared.
    pub new_version: semver::Version,
}

/// Overall compatibility verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityVerdict {
    /// Versions are fully compatible. No changes or only patch-level changes.
    Compatible,
    /// Versions are compatible with additions. Minor version bump expected.
    CompatibleWithAdditions,
    /// Versions are incompatible. Major version bump required.
    Breaking,
}

/// A single change between two versions.
#[derive(Debug, Clone)]
pub struct ChangeEntry {
    /// What changed.
    pub description: String,
    /// Severity of the change.
    pub severity: ChangeSeverity,
    /// Path to the changed element (e.g., "types.buffer.read").
    pub path: String,
}

/// Severity of a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeSeverity {
    /// No behavioral change (documentation, clarification).
    Patch,
    /// Additive change (new function, new interface, new variant case).
    Minor,
    /// Breaking change (removed function, changed type, changed ownership).
    Breaking,
}

/// Compare two versions of a WIT package for compatibility.
///
/// # Preconditions
/// - `old` and `new` represent the same package (same name).
/// - Both packages are fully resolved (no dangling references).
///
/// # Postconditions
/// - Returns a `CompatibilityReport` with all detected changes.
/// - The report's verdict is deterministic for the same inputs.
///
/// # COLD PATH
pub fn check_compatibility(old: &ParsedPackage, new: &ParsedPackage) -> CompatibilityReport {
    let old_version = old
        .version
        .clone()
        .unwrap_or_else(|| semver::Version::new(0, 0, 0));
    let new_version = new
        .version
        .clone()
        .unwrap_or_else(|| semver::Version::new(0, 0, 0));

    let mut changes = Vec::new();

    // Compare interfaces
    compare_interfaces(&old.interfaces, &new.interfaces, &mut changes);

    // Compare worlds
    compare_worlds(&old.worlds, &new.worlds, &mut changes);

    // Determine overall verdict
    let verdict = if changes
        .iter()
        .any(|c| c.severity == ChangeSeverity::Breaking)
    {
        CompatibilityVerdict::Breaking
    } else if changes.iter().any(|c| c.severity == ChangeSeverity::Minor) {
        CompatibilityVerdict::CompatibleWithAdditions
    } else {
        CompatibilityVerdict::Compatible
    };

    CompatibilityReport {
        verdict,
        changes,
        old_version,
        new_version,
    }
}

/// Convert a compatibility report to validation diagnostics.
///
/// # COLD PATH
pub fn report_to_diagnostics(
    report: &CompatibilityReport,
    old_source: &str,
    new_source: &str,
) -> ValidationResult {
    let mut result = ValidationResult::new();

    for change in &report.changes {
        if change.severity == ChangeSeverity::Breaking {
            result.push(
                DiagnosticBuilder::error(error_code_for_change(change), &change.description)
                    .note(format!(
                        "between {} ({}) and {} ({})",
                        old_source, report.old_version, new_source, report.new_version
                    ))
                    .note(format!("path: {}", change.path))
                    .help(format!(
                        "this is a breaking change requiring a major version bump (current: {})",
                        report.old_version
                    ))
                    .build(),
            );
        }
    }

    result
}

/// Map a change entry to an appropriate error code.
fn error_code_for_change(change: &ChangeEntry) -> ErrorCode {
    let desc = change.description.to_lowercase();
    if desc.contains("removed") && desc.contains("function") {
        ErrorCode::MissingFunction
    } else if desc.contains("removed") && desc.contains("interface") {
        ErrorCode::MissingInterface
    } else if desc.contains("type") && desc.contains("changed") {
        ErrorCode::TypeSignatureChanged
    } else if desc.contains("ownership") {
        ErrorCode::OwnershipSemanticChanged
    } else if desc.contains("field") && desc.contains("removed") {
        ErrorCode::FieldRemoved
    } else if desc.contains("case") && desc.contains("removed") {
        ErrorCode::VariantCaseRemoved
    } else {
        ErrorCode::IncompatibleMajorVersion
    }
}

/// Compare interface maps between old and new packages.
fn compare_interfaces(
    old: &HashMap<String, ParsedInterface>,
    new: &HashMap<String, ParsedInterface>,
    changes: &mut Vec<ChangeEntry>,
) {
    // Check for removed interfaces (breaking)
    for name in old.keys() {
        if !new.contains_key(name) {
            changes.push(ChangeEntry {
                description: format!("interface '{}' was removed", name),
                severity: ChangeSeverity::Breaking,
                path: name.clone(),
            });
        }
    }

    // Check for added interfaces (minor)
    for name in new.keys() {
        if !old.contains_key(name) {
            changes.push(ChangeEntry {
                description: format!("interface '{}' was added", name),
                severity: ChangeSeverity::Minor,
                path: name.clone(),
            });
        }
    }

    // Check for changes within shared interfaces
    for (name, old_iface) in old {
        if let Some(new_iface) = new.get(name) {
            compare_interface_contents(old_iface, new_iface, name, changes);
        }
    }
}

/// Compare the contents of two interfaces.
fn compare_interface_contents(
    old: &ParsedInterface,
    new: &ParsedInterface,
    path_prefix: &str,
    changes: &mut Vec<ChangeEntry>,
) {
    // Check removed functions (breaking)
    for func_name in old.functions.keys() {
        if !new.functions.contains_key(func_name) {
            changes.push(ChangeEntry {
                description: format!("function '{}' was removed from interface", func_name),
                severity: ChangeSeverity::Breaking,
                path: format!("{}.{}", path_prefix, func_name),
            });
        }
    }

    // Check added functions (minor)
    for func_name in new.functions.keys() {
        if !old.functions.contains_key(func_name) {
            changes.push(ChangeEntry {
                description: format!("function '{}' was added to interface", func_name),
                severity: ChangeSeverity::Minor,
                path: format!("{}.{}", path_prefix, func_name),
            });
        }
    }

    // Check function signature changes (breaking)
    for (func_name, old_func) in &old.functions {
        if let Some(new_func) = new.functions.get(func_name) {
            if old_func.params.len() != new_func.params.len() {
                changes.push(ChangeEntry {
                    description: format!(
                        "function '{}' parameter count changed from {} to {}",
                        func_name,
                        old_func.params.len(),
                        new_func.params.len()
                    ),
                    severity: ChangeSeverity::Breaking,
                    path: format!("{}.{}", path_prefix, func_name),
                });
            } else {
                for (old_p, new_p) in old_func.params.iter().zip(new_func.params.iter()) {
                    if old_p.typ != new_p.typ {
                        changes.push(ChangeEntry {
                            description: format!(
                                "function '{}' parameter '{}' type changed",
                                func_name, old_p.name
                            ),
                            severity: ChangeSeverity::Breaking,
                            path: format!("{}.{}.{}", path_prefix, func_name, old_p.name),
                        });
                    }
                }
            }

            if old_func.result != new_func.result {
                changes.push(ChangeEntry {
                    description: format!("function '{}' return type changed", func_name),
                    severity: ChangeSeverity::Breaking,
                    path: format!("{}.{}", path_prefix, func_name),
                });
            }
        }
    }

    // Check removed resources (breaking)
    for res_name in &old.resources {
        if !new.resources.contains(res_name) {
            changes.push(ChangeEntry {
                description: format!("resource '{}' was removed", res_name),
                severity: ChangeSeverity::Breaking,
                path: format!("{}.{}", path_prefix, res_name),
            });
        }
    }

    // Check type definition changes
    for (type_name, old_td) in &old.types {
        if let Some(new_td) = new.types.get(type_name) {
            compare_type_defs(
                old_td,
                new_td,
                &format!("{}.{}", path_prefix, type_name),
                changes,
            );
        } else {
            changes.push(ChangeEntry {
                description: format!("type '{}' was removed", type_name),
                severity: ChangeSeverity::Breaking,
                path: format!("{}.{}", path_prefix, type_name),
            });
        }
    }

    // Check added types (minor)
    for type_name in new.types.keys() {
        if !old.types.contains_key(type_name) {
            changes.push(ChangeEntry {
                description: format!("type '{}' was added", type_name),
                severity: ChangeSeverity::Minor,
                path: format!("{}.{}", path_prefix, type_name),
            });
        }
    }
}

/// Compare two type definitions.
fn compare_type_defs(
    old: &crate::parser::ParsedTypeDef,
    new: &crate::parser::ParsedTypeDef,
    path: &str,
    changes: &mut Vec<ChangeEntry>,
) {
    match (&old.kind, &new.kind) {
        (ParsedTypeDefKind::Record(old_fields), ParsedTypeDefKind::Record(new_fields)) => {
            compare_record_fields(old_fields, new_fields, path, changes);
        }
        (ParsedTypeDefKind::Variant(old_cases), ParsedTypeDefKind::Variant(new_cases)) => {
            compare_variant_cases(old_cases, new_cases, path, changes);
        }
        (ParsedTypeDefKind::Enum(old_cases), ParsedTypeDefKind::Enum(new_cases)) => {
            for case in old_cases {
                if !new_cases.contains(case) {
                    changes.push(ChangeEntry {
                        description: format!("enum case '{}' was removed", case),
                        severity: ChangeSeverity::Breaking,
                        path: format!("{}.{}", path, case),
                    });
                }
            }
            for case in new_cases {
                if !old_cases.contains(case) {
                    changes.push(ChangeEntry {
                        description: format!("enum case '{}' was added", case),
                        severity: ChangeSeverity::Minor,
                        path: format!("{}.{}", path, case),
                    });
                }
            }
        }
        (old_kind, new_kind) => {
            let old_kind_str = type_def_kind_name(old_kind);
            let new_kind_str = type_def_kind_name(new_kind);
            if old_kind_str != new_kind_str {
                changes.push(ChangeEntry {
                    description: format!(
                        "type definition kind changed from {} to {}",
                        old_kind_str, new_kind_str
                    ),
                    severity: ChangeSeverity::Breaking,
                    path: path.to_string(),
                });
            }
        }
    }
}

/// Compare record fields between versions.
fn compare_record_fields(
    old: &[ParsedField],
    new: &[ParsedField],
    path: &str,
    changes: &mut Vec<ChangeEntry>,
) {
    let old_map: HashMap<&str, &ParsedType> =
        old.iter().map(|f| (f.name.as_str(), &f.typ)).collect();
    let new_map: HashMap<&str, &ParsedType> =
        new.iter().map(|f| (f.name.as_str(), &f.typ)).collect();

    for name in old_map.keys() {
        if !new_map.contains_key(name) {
            changes.push(ChangeEntry {
                description: format!("field '{}' was removed from record", name),
                severity: ChangeSeverity::Breaking,
                path: format!("{}.{}", path, name),
            });
        }
    }

    // Added field = breaking (WIT records are not extensible)
    for name in new_map.keys() {
        if !old_map.contains_key(name) {
            changes.push(ChangeEntry {
                description: format!(
                    "field '{}' was added to record (breaking: WIT records are not extensible)",
                    name
                ),
                severity: ChangeSeverity::Breaking,
                path: format!("{}.{}", path, name),
            });
        }
    }

    for (name, old_type) in &old_map {
        if let Some(new_type) = new_map.get(name) {
            if old_type != new_type {
                changes.push(ChangeEntry {
                    description: format!("field '{}' type changed in record", name),
                    severity: ChangeSeverity::Breaking,
                    path: format!("{}.{}", path, name),
                });
            }
        }
    }
}

/// Compare variant cases between versions.
fn compare_variant_cases(
    old: &[ParsedCase],
    new: &[ParsedCase],
    path: &str,
    changes: &mut Vec<ChangeEntry>,
) {
    let old_map: HashMap<&str, &Option<ParsedType>> =
        old.iter().map(|c| (c.name.as_str(), &c.typ)).collect();
    let new_map: HashMap<&str, &Option<ParsedType>> =
        new.iter().map(|c| (c.name.as_str(), &c.typ)).collect();

    for name in old_map.keys() {
        if !new_map.contains_key(name) {
            changes.push(ChangeEntry {
                description: format!("variant case '{}' was removed", name),
                severity: ChangeSeverity::Breaking,
                path: format!("{}.{}", path, name),
            });
        }
    }

    for name in new_map.keys() {
        if !old_map.contains_key(name) {
            changes.push(ChangeEntry {
                description: format!("variant case '{}' was added", name),
                severity: ChangeSeverity::Minor,
                path: format!("{}.{}", path, name),
            });
        }
    }

    for (name, old_type) in &old_map {
        if let Some(new_type) = new_map.get(name) {
            if old_type != new_type {
                changes.push(ChangeEntry {
                    description: format!("variant case '{}' payload type changed", name),
                    severity: ChangeSeverity::Breaking,
                    path: format!("{}.{}", path, name),
                });
            }
        }
    }
}

/// Compare worlds between packages.
fn compare_worlds(
    old: &HashMap<String, crate::parser::ParsedWorld>,
    new: &HashMap<String, crate::parser::ParsedWorld>,
    changes: &mut Vec<ChangeEntry>,
) {
    for name in old.keys() {
        if !new.contains_key(name) {
            changes.push(ChangeEntry {
                description: format!("world '{}' was removed", name),
                severity: ChangeSeverity::Breaking,
                path: format!("world:{}", name),
            });
        }
    }
    for name in new.keys() {
        if !old.contains_key(name) {
            changes.push(ChangeEntry {
                description: format!("world '{}' was added", name),
                severity: ChangeSeverity::Minor,
                path: format!("world:{}", name),
            });
        }
    }
}

/// Get a human-readable name for a type definition kind.
fn type_def_kind_name(kind: &ParsedTypeDefKind) -> &'static str {
    match kind {
        ParsedTypeDefKind::Record(_) => "record",
        ParsedTypeDefKind::Variant(_) => "variant",
        ParsedTypeDefKind::Enum(_) => "enum",
        ParsedTypeDefKind::Flags(_) => "flags",
        ParsedTypeDefKind::Resource => "resource",
        ParsedTypeDefKind::Alias(_) => "alias",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::*;

    fn make_package(version: &str, interfaces: HashMap<String, ParsedInterface>) -> ParsedPackage {
        ParsedPackage {
            name: "torvyn:streaming".into(),
            version: Some(semver::Version::parse(version).unwrap()),
            interfaces,
            worlds: HashMap::new(),
            source_files: vec![],
        }
    }

    type FunctionDef<'a> = (&'a str, Vec<(&'a str, ParsedType)>, Option<ParsedType>);

    fn make_interface(functions: Vec<FunctionDef<'_>>) -> ParsedInterface {
        let mut funcs = HashMap::new();
        for (name, params, result) in functions {
            funcs.insert(
                name.to_string(),
                ParsedFunction {
                    name: name.to_string(),
                    params: params
                        .into_iter()
                        .map(|(n, t)| ParsedParam {
                            name: n.to_string(),
                            typ: t,
                        })
                        .collect(),
                    result,
                },
            );
        }
        ParsedInterface {
            name: "test".into(),
            functions: funcs,
            types: HashMap::new(),
            resources: vec![],
        }
    }

    #[test]
    fn test_identical_packages_are_compatible() {
        let iface = make_interface(vec![(
            "process",
            vec![("input", ParsedType::Named("stream-element".into()))],
            Some(ParsedType::Named("process-result".into())),
        )]);

        let mut interfaces = HashMap::new();
        interfaces.insert("processor".into(), iface.clone());

        let old = make_package("0.1.0", interfaces.clone());
        let new = make_package("0.1.0", interfaces);

        let report = check_compatibility(&old, &new);
        assert_eq!(report.verdict, CompatibilityVerdict::Compatible);
        assert!(report.changes.is_empty());
    }

    #[test]
    fn test_added_function_is_minor() {
        let old_iface = make_interface(vec![(
            "process",
            vec![("input", ParsedType::Named("stream-element".into()))],
            Some(ParsedType::Named("process-result".into())),
        )]);

        let new_iface = make_interface(vec![
            (
                "process",
                vec![("input", ParsedType::Named("stream-element".into()))],
                Some(ParsedType::Named("process-result".into())),
            ),
            (
                "process-batch",
                vec![(
                    "inputs",
                    ParsedType::List(Box::new(ParsedType::Named("stream-element".into()))),
                )],
                Some(ParsedType::Named("process-result".into())),
            ),
        ]);

        let mut old_interfaces = HashMap::new();
        old_interfaces.insert("processor".into(), old_iface);
        let mut new_interfaces = HashMap::new();
        new_interfaces.insert("processor".into(), new_iface);

        let old = make_package("0.1.0", old_interfaces);
        let new = make_package("0.2.0", new_interfaces);

        let report = check_compatibility(&old, &new);
        assert_eq!(
            report.verdict,
            CompatibilityVerdict::CompatibleWithAdditions
        );
    }

    #[test]
    fn test_removed_function_is_breaking() {
        let old_iface = make_interface(vec![
            (
                "process",
                vec![("input", ParsedType::Named("stream-element".into()))],
                Some(ParsedType::Named("process-result".into())),
            ),
            ("reset", vec![], None),
        ]);

        let new_iface = make_interface(vec![(
            "process",
            vec![("input", ParsedType::Named("stream-element".into()))],
            Some(ParsedType::Named("process-result".into())),
        )]);

        let mut old_interfaces = HashMap::new();
        old_interfaces.insert("processor".into(), old_iface);
        let mut new_interfaces = HashMap::new();
        new_interfaces.insert("processor".into(), new_iface);

        let old = make_package("0.1.0", old_interfaces);
        let new = make_package("1.0.0", new_interfaces);

        let report = check_compatibility(&old, &new);
        assert_eq!(report.verdict, CompatibilityVerdict::Breaking);
        assert!(report
            .changes
            .iter()
            .any(|c| c.description.contains("removed")));
    }

    #[test]
    fn test_changed_param_type_is_breaking() {
        let old_iface = make_interface(vec![(
            "process",
            vec![("input", ParsedType::Named("stream-element".into()))],
            Some(ParsedType::Named("process-result".into())),
        )]);

        let new_iface = make_interface(vec![(
            "process",
            vec![("input", ParsedType::Primitive("string".into()))],
            Some(ParsedType::Named("process-result".into())),
        )]);

        let mut old_interfaces = HashMap::new();
        old_interfaces.insert("processor".into(), old_iface);
        let mut new_interfaces = HashMap::new();
        new_interfaces.insert("processor".into(), new_iface);

        let old = make_package("0.1.0", old_interfaces);
        let new = make_package("1.0.0", new_interfaces);

        let report = check_compatibility(&old, &new);
        assert_eq!(report.verdict, CompatibilityVerdict::Breaking);
    }

    #[test]
    fn test_removed_interface_is_breaking() {
        let iface = make_interface(vec![]);
        let mut old_interfaces = HashMap::new();
        old_interfaces.insert("processor".into(), iface);

        let old = make_package("0.1.0", old_interfaces);
        let new = make_package("1.0.0", HashMap::new());

        let report = check_compatibility(&old, &new);
        assert_eq!(report.verdict, CompatibilityVerdict::Breaking);
    }

    #[test]
    fn test_added_interface_is_minor() {
        let iface = make_interface(vec![]);

        let old = make_package("0.1.0", HashMap::new());
        let mut new_interfaces = HashMap::new();
        new_interfaces.insert("batch-processor".into(), iface);
        let new = make_package("0.2.0", new_interfaces);

        let report = check_compatibility(&old, &new);
        assert_eq!(
            report.verdict,
            CompatibilityVerdict::CompatibleWithAdditions
        );
    }

    #[test]
    fn test_added_record_field_is_breaking() {
        let old_type = ParsedTypeDef {
            name: "element-meta".into(),
            kind: ParsedTypeDefKind::Record(vec![ParsedField {
                name: "sequence".into(),
                typ: ParsedType::Primitive("u64".into()),
            }]),
        };

        let new_type = ParsedTypeDef {
            name: "element-meta".into(),
            kind: ParsedTypeDefKind::Record(vec![
                ParsedField {
                    name: "sequence".into(),
                    typ: ParsedType::Primitive("u64".into()),
                },
                ParsedField {
                    name: "priority".into(),
                    typ: ParsedType::Primitive("u32".into()),
                },
            ]),
        };

        let mut old_types = HashMap::new();
        old_types.insert("element-meta".into(), old_type);
        let mut new_types = HashMap::new();
        new_types.insert("element-meta".into(), new_type);

        let old_iface = ParsedInterface {
            name: "types".into(),
            functions: HashMap::new(),
            types: old_types,
            resources: vec![],
        };
        let new_iface = ParsedInterface {
            name: "types".into(),
            functions: HashMap::new(),
            types: new_types,
            resources: vec![],
        };

        let mut old_interfaces = HashMap::new();
        old_interfaces.insert("types".into(), old_iface);
        let mut new_interfaces = HashMap::new();
        new_interfaces.insert("types".into(), new_iface);

        let old = make_package("0.1.0", old_interfaces);
        let new = make_package("1.0.0", new_interfaces);

        let report = check_compatibility(&old, &new);
        assert_eq!(report.verdict, CompatibilityVerdict::Breaking);
        assert!(report
            .changes
            .iter()
            .any(|c| c.description.contains("added to record")));
    }

    #[test]
    fn test_added_variant_case_is_minor() {
        let old_type = ParsedTypeDef {
            name: "process-error".into(),
            kind: ParsedTypeDefKind::Variant(vec![ParsedCase {
                name: "internal".into(),
                typ: Some(ParsedType::Primitive("string".into())),
            }]),
        };

        let new_type = ParsedTypeDef {
            name: "process-error".into(),
            kind: ParsedTypeDefKind::Variant(vec![
                ParsedCase {
                    name: "internal".into(),
                    typ: Some(ParsedType::Primitive("string".into())),
                },
                ParsedCase {
                    name: "timeout".into(),
                    typ: None,
                },
            ]),
        };

        let mut old_types = HashMap::new();
        old_types.insert("process-error".into(), old_type);
        let mut new_types = HashMap::new();
        new_types.insert("process-error".into(), new_type);

        let old_iface = ParsedInterface {
            name: "types".into(),
            functions: HashMap::new(),
            types: old_types,
            resources: vec![],
        };
        let new_iface = ParsedInterface {
            name: "types".into(),
            functions: HashMap::new(),
            types: new_types,
            resources: vec![],
        };

        let mut old_interfaces = HashMap::new();
        old_interfaces.insert("types".into(), old_iface);
        let mut new_interfaces = HashMap::new();
        new_interfaces.insert("types".into(), new_iface);

        let old = make_package("0.1.0", old_interfaces);
        let new = make_package("0.2.0", new_interfaces);

        let report = check_compatibility(&old, &new);
        assert_eq!(
            report.verdict,
            CompatibilityVerdict::CompatibleWithAdditions
        );
    }

    #[test]
    fn test_removed_variant_case_is_breaking() {
        let old_type = ParsedTypeDef {
            name: "process-error".into(),
            kind: ParsedTypeDefKind::Variant(vec![
                ParsedCase {
                    name: "internal".into(),
                    typ: Some(ParsedType::Primitive("string".into())),
                },
                ParsedCase {
                    name: "fatal".into(),
                    typ: Some(ParsedType::Primitive("string".into())),
                },
            ]),
        };

        let new_type = ParsedTypeDef {
            name: "process-error".into(),
            kind: ParsedTypeDefKind::Variant(vec![ParsedCase {
                name: "internal".into(),
                typ: Some(ParsedType::Primitive("string".into())),
            }]),
        };

        let mut old_types = HashMap::new();
        old_types.insert("process-error".into(), old_type);
        let mut new_types = HashMap::new();
        new_types.insert("process-error".into(), new_type);

        let old_iface = ParsedInterface {
            name: "types".into(),
            functions: HashMap::new(),
            types: old_types,
            resources: vec![],
        };
        let new_iface = ParsedInterface {
            name: "types".into(),
            functions: HashMap::new(),
            types: new_types,
            resources: vec![],
        };

        let mut old_interfaces = HashMap::new();
        old_interfaces.insert("types".into(), old_iface);
        let mut new_interfaces = HashMap::new();
        new_interfaces.insert("types".into(), new_iface);

        let old = make_package("0.1.0", old_interfaces);
        let new = make_package("1.0.0", new_interfaces);

        let report = check_compatibility(&old, &new);
        assert_eq!(report.verdict, CompatibilityVerdict::Breaking);
    }

    #[test]
    fn test_report_to_diagnostics_produces_errors_for_breaking() {
        let report = CompatibilityReport {
            verdict: CompatibilityVerdict::Breaking,
            changes: vec![ChangeEntry {
                description: "function 'process' was removed from interface".into(),
                severity: ChangeSeverity::Breaking,
                path: "processor.process".into(),
            }],
            old_version: semver::Version::new(0, 1, 0),
            new_version: semver::Version::new(0, 2, 0),
        };

        let result = report_to_diagnostics(&report, "component-a", "component-b");
        assert!(!result.is_ok());
    }

    #[test]
    fn test_deterministic_result() {
        let old_iface = make_interface(vec![("a", vec![], None), ("b", vec![], None)]);
        let new_iface = make_interface(vec![("a", vec![], None), ("c", vec![], None)]);

        let mut old_interfaces = HashMap::new();
        old_interfaces.insert("test".into(), old_iface);
        let mut new_interfaces = HashMap::new();
        new_interfaces.insert("test".into(), new_iface);

        let old = make_package("0.1.0", old_interfaces.clone());
        let new = make_package("1.0.0", new_interfaces.clone());

        let report1 = check_compatibility(&old, &new);
        let report2 = check_compatibility(&old, &new);

        assert_eq!(report1.verdict, report2.verdict);
        assert_eq!(report1.changes.len(), report2.changes.len());
    }
}
