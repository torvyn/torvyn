//! Every flag the CLI declares must either do something or say it does not.
//!
//! Thirteen of the CLI's flags were parsed and never read — accepted in
//! silence, with no effect and no warning. The worst was `torvyn bench
//! --compare`: the observability guide presented it as a CI regression gate
//! ("fail the pipeline if latency regressions exceed a threshold") and the CLI
//! reference gave exit code 3 for a detected regression, while a baseline
//! claiming a billion elements per second against a real twenty-eight thousand
//! still exited zero. A gate that cannot fail is worse than no gate.
//!
//! `run` and `trace` already refused their unimplemented flags; nine others
//! across five commands never got the same treatment. The classification below
//! is the single place that records which is which.
//!
//! The list is checked against clap rather than written beside it: a flag the
//! CLI declares and this table does not classify fails
//! [`every_declared_flag_is_classified`]. A new flag is therefore unimplemented
//! until someone says otherwise, which is the default that was missing.

use assert_cmd::Command;
use clap::CommandFactory;
use predicates::prelude::*;
use std::collections::BTreeSet;
use tempfile::TempDir;
use torvyn_cli::cli::Cli;

/// A flag the command honours.
struct Implemented(&'static str, &'static [&'static str]);

/// A flag the command parses and refuses, with a value that triggers the
/// refusal. A flag with a default is refused only for a non-default value,
/// because asking for what the command already does is honest.
struct Refused(
    &'static str,
    &'static [(&'static str, Option<&'static str>)],
);

/// What each command does with each flag it declares.
///
/// Global flags (`--verbose`, `--format`, …) are excluded: they are declared
/// once on the root command, not per subcommand.
const IMPLEMENTED: &[Implemented] = &[
    Implemented(
        "init",
        &[
            "project_name",
            "template",
            "no_git",
            "contract_version",
            "force",
            "language",
        ],
    ),
    Implemented("check", &["manifest", "strict"]),
    Implemented("build", &["manifest", "component", "debug"]),
    Implemented("link", &["manifest", "flow"]),
    Implemented("run", &["manifest", "flow", "timeout", "log_level"]),
    Implemented(
        "trace",
        &[
            "manifest",
            "flow",
            "trace_format",
            "output_trace",
            "show_backpressure",
            "limit",
        ],
    ),
    Implemented(
        "bench",
        &["manifest", "flow", "duration", "warmup", "report_format"],
    ),
    Implemented(
        "pack",
        &[
            "manifest",
            "component",
            "output",
            "tag",
            "include_source",
            "sign",
        ],
    ),
    Implemented("publish", &["artifact", "registry", "dry_run"]),
    Implemented("inspect", &["target", "show"]),
    Implemented("doctor", &["fix"]),
    // `completions` writes a shell completion script and touches nothing else.
    Implemented("completions", &["shell"]),
];

/// Flags that must be refused, and the value that must trigger it.
///
/// `None` is a boolean flag. `Some(v)` is a value that differs from the
/// default, for the flags whose default is what the command already does.
const REFUSED: &[Refused] = &[
    Refused(
        "init",
        &[("--language", Some("go")), ("--no-example", None)],
    ),
    Refused(
        "link",
        &[("--components", Some("./components")), ("--detail", None)],
    ),
    Refused(
        "run",
        &[
            ("--limit", Some("10")),
            ("--input", Some("x")),
            ("--output", Some("x")),
            ("--config", Some("k=v")),
            ("--log-level", Some("debug")),
        ],
    ),
    Refused("trace", &[("--show-buffers", None), ("--input", Some("x"))]),
    Refused(
        "bench",
        &[
            ("--compare", Some("baseline.json")),
            ("--baseline", Some("named")),
            ("--report", Some("out.json")),
            ("--report-format", Some("json")),
            ("--input", Some("x")),
        ],
    ),
    Refused("publish", &[("--tag", Some("v9")), ("--force", None)]),
    Refused("inspect", &[("--show", Some("capabilities"))]),
];

/// Flags a subcommand declares, as clap knows them, by field id.
fn declared_flags(subcommand: &str) -> BTreeSet<String> {
    Cli::command()
        .get_subcommands()
        .find(|c| c.get_name() == subcommand)
        .unwrap_or_else(|| panic!("no such subcommand: {subcommand}"))
        .get_arguments()
        .filter(|a| !a.is_global_set())
        .map(|a| a.get_id().to_string())
        .filter(|id| id != "help" && id != "version")
        .collect()
}

/// Every flag clap declares must appear in exactly one of the two tables.
///
/// This is what makes a newly added flag unimplemented by default: wiring it
/// up means listing it, and forgetting fails here rather than shipping a flag
/// that silently does nothing.
#[test]
fn every_declared_flag_is_classified() {
    for Implemented(command, implemented) in IMPLEMENTED {
        let declared = declared_flags(command);

        let refused: BTreeSet<String> = REFUSED
            .iter()
            .find(|Refused(c, _)| c == command)
            .map(|Refused(_, flags)| {
                flags
                    .iter()
                    .map(|(flag, _)| flag.trim_start_matches('-').replace('-', "_"))
                    .collect()
            })
            .unwrap_or_default();

        let classified: BTreeSet<String> = implemented
            .iter()
            .map(|s| (*s).to_owned())
            .chain(refused.iter().cloned())
            .collect();

        let unclassified: Vec<&String> = declared.difference(&classified).collect();
        assert!(
            unclassified.is_empty(),
            "`torvyn {command}` declares {unclassified:?}, which this table does not classify. \
             A flag is unimplemented until it is listed: add it to IMPLEMENTED once it is \
             wired up, or to REFUSED with the value that triggers the refusal."
        );

        let phantom: Vec<&String> = classified.difference(&declared).collect();
        assert!(
            phantom.is_empty(),
            "`torvyn {command}` does not declare {phantom:?}, so this table has gone stale"
        );
    }
}

/// Every command clap declares must appear in the table.
#[test]
fn every_subcommand_is_covered() {
    let declared: BTreeSet<String> = Cli::command()
        .get_subcommands()
        .map(|c| c.get_name().to_owned())
        .collect();
    let covered: BTreeSet<String> = IMPLEMENTED
        .iter()
        .map(|Implemented(c, _)| (*c).to_owned())
        .collect();

    let uncovered: Vec<&String> = declared.difference(&covered).collect();
    assert!(
        uncovered.is_empty(),
        "these subcommands are not covered by the flag table: {uncovered:?}"
    );
}

/// A scaffolded, packed project, so every command below has something to act
/// on and a refusal is the only reason any of them can fail.
fn workspace() -> (TempDir, std::path::PathBuf) {
    const EMPTY_COMPONENT: &[u8] = &[0x00, 0x61, 0x73, 0x6d, 0x0d, 0x00, 0x01, 0x00];

    let dir = TempDir::new().expect("temp dir");
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "probe", "--template", "transform"])
        .current_dir(dir.path())
        .assert()
        .success();

    let project = dir.path().join("probe");
    let build = project.join(".torvyn/build");
    std::fs::create_dir_all(&build).unwrap();
    let manifest = std::fs::read_to_string(project.join("Torvyn.toml")).unwrap();
    for line in manifest.lines() {
        if let Some(rest) = line.trim().strip_prefix("name = \"") {
            if let Some(name) = rest.strip_suffix('"') {
                std::fs::write(build.join(format!("{name}.wasm")), EMPTY_COMPONENT).ok();
            }
        }
    }
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project)
        .assert()
        .success();

    (dir, project)
}

/// Every flag classified as refused must actually be refused, and say so.
///
/// Passing a flag and getting nothing is the failure this whole table exists
/// to prevent, so the assertion is on the message the user sees.
#[test]
fn every_refused_flag_says_it_is_not_implemented() {
    for Refused(command, flags) in REFUSED {
        for (flag, value) in *flags {
            let (_dir, project) = workspace();

            let mut invocation: Vec<String> = vec![(*command).to_owned()];
            // `init` needs a name, `inspect` a target; neither has a default.
            match *command {
                "init" => invocation.push("probe-2".to_owned()),
                "inspect" => {
                    invocation.push(".torvyn/artifacts/probe-0.1.0.torvyn".to_owned());
                }
                _ => {}
            }
            invocation.push((*flag).to_owned());
            if let Some(value) = value {
                invocation.push((*value).to_owned());
            }
            // Keep the bench runs short; the refusal happens before the flow
            // starts, but a missed refusal must not hang the suite.
            if *command == "bench" {
                invocation.extend(["--duration", "1s", "--warmup", "0s"].map(str::to_owned));
            }

            Command::cargo_bin("torvyn")
                .unwrap()
                .args(&invocation)
                .current_dir(&project)
                .assert()
                .failure()
                .stderr(
                    predicate::str::contains("is not implemented")
                        .and(predicate::str::contains(flag.trim_start_matches('-'))),
                );
        }
    }
}

/// A flag whose default is what the command already does must pass when the
/// default is given explicitly. Refusing it would break every caller that
/// spells out a value it was always going to get.
#[test]
fn explicitly_passing_a_default_is_not_refused() {
    let cases: &[(&str, &[&str])] = &[
        ("init", &["init", "probe-default", "--language", "rust"]),
        ("run", &["run", "--log-level", "info"]),
        (
            "bench",
            &[
                "bench",
                "--report-format",
                "pretty",
                "--duration",
                "1s",
                "--warmup",
                "0s",
            ],
        ),
    ];

    for (label, invocation) in cases {
        let (_dir, project) = workspace();
        let output = Command::cargo_bin("torvyn")
            .unwrap()
            .args(*invocation)
            .current_dir(&project)
            .output()
            .expect("run the command");

        // The probe project's components are eight-byte stubs, so a command
        // that starts a pipeline fails for that reason. What must not happen
        // is a refusal: the value given is the one the command already uses.
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("is not implemented"),
            "`torvyn {label}` refused a value that is its own default:\n{stderr}"
        );
    }
}
