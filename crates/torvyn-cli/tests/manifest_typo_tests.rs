//! A mistyped manifest key must be reported, not dropped.
//!
//! No struct in `torvyn-config` rejected unknown keys, and `torvyn check`
//! never looked inside the `[flow.*]` section at all, so a one-character typo
//! was silently ignored while every command reported success:
//!
//! - `fuel_budgett = 1` left a component unbounded. The correctly spelled key
//!   stops it on its first element.
//! - `[secuirty.grants.sink]` left the sink unable to print. The pipeline ran
//!   to `Completed` with `Errors: 0` and produced no output at all, because
//!   the grant that reaches the sandbox had vanished.
//! - `max_memory = "banana"` passed `check` and failed `run` — with an error
//!   ending "run `torvyn check` to validate", pointing at the command that
//!   had just passed it.
//!
//! These are cheap to check and expensive to debug, which is what makes them
//! worth a test apiece.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use tempfile::TempDir;

/// A complete, valid manifest for a two-component pipeline.
const VALID_MANIFEST: &str = r#"
[torvyn]
name = "typo-probe"
version = "0.1.0"
contract_version = "0.1.0"

[[component]]
name = "source"
path = "components/source"
language = "rust"

[[component]]
name = "sink"
path = "components/sink"
language = "rust"

[flow.main.nodes.source]
component = "source"
interface = "torvyn:streaming/source"

[flow.main.nodes.sink]
component = "sink"
interface = "torvyn:streaming/sink"

[[flow.main.edges]]
from = { node = "source", port = "output" }
to = { node = "sink", port = "input" }

[security.grants.sink]
capabilities = ["stdio:stdout"]
"#;

/// Write a manifest into a fresh project directory.
fn project_with(dir: &Path, manifest: &str) -> std::path::PathBuf {
    let project = dir.join("proj");
    std::fs::create_dir_all(&project).expect("create project dir");
    std::fs::write(project.join("Torvyn.toml"), manifest).expect("write manifest");
    project
}

/// The unmodified manifest must pass, or every assertion below is meaningless.
#[test]
fn the_control_manifest_passes() {
    let dir = TempDir::new().unwrap();
    let project = project_with(dir.path(), VALID_MANIFEST);

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(&project)
        .assert()
        .success();
}

/// A mistyped key inside a flow node. This is where the resource limits live,
/// so a typo here is the difference between a bounded component and an
/// unbounded one.
#[test]
fn a_mistyped_node_key_is_reported_with_a_suggestion() {
    let dir = TempDir::new().unwrap();
    let project = project_with(
        dir.path(),
        &VALID_MANIFEST.replace(
            "interface = \"torvyn:streaming/sink\"",
            "interface = \"torvyn:streaming/sink\"\nfuel_budgett = 1",
        ),
    );

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(&project)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("fuel_budgett")
                .and(predicate::str::contains("Did you mean `fuel_budget`")),
        );
}

/// A mistyped top-level table. The manifest captures unknown tables for
/// forward compatibility and reads none of them, so this silently discarded
/// the sink's capability grant.
#[test]
fn a_mistyped_table_is_reported_with_a_suggestion() {
    let dir = TempDir::new().unwrap();
    let project = project_with(
        dir.path(),
        &VALID_MANIFEST.replace("[security.grants.sink]", "[secuirty.grants.sink]"),
    );

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(&project)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("secuirty")
                .and(predicate::str::contains("Did you mean `[security]`")),
        );
}

/// A table with no near match is more likely a field from a newer Torvyn than
/// a typo. The manifest is explicitly designed to tolerate those, so it must
/// be a warning — but a silent one is how an ignored table goes unnoticed.
#[test]
fn an_unrecognised_table_warns_rather_than_failing() {
    let dir = TempDir::new().unwrap();
    let project = project_with(
        dir.path(),
        &format!("{VALID_MANIFEST}\n[telemetry_exporter]\nendpoint = \"https://example\"\n"),
    );

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(&project)
        .assert()
        .success()
        .stderr(
            predicate::str::contains("telemetry_exporter")
                .and(predicate::str::contains("does not recognise")),
        );
}

/// `check` and `run` must agree. A value `run` rejects cannot pass `check`,
/// because `run`'s own error tells the reader to go and run `check`.
#[test]
fn check_rejects_what_run_rejects() {
    let dir = TempDir::new().unwrap();
    let project = project_with(
        dir.path(),
        &VALID_MANIFEST.replace(
            "interface = \"torvyn:streaming/sink\"",
            "interface = \"torvyn:streaming/sink\"\nmax_memory = \"banana\"",
        ),
    );

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(&project)
        .assert()
        .failure()
        .stderr(predicate::str::contains("max_memory"));
}

/// A project that declares components and no pipeline is valid — the `empty`
/// template produces one — and must not be failed by a validator that assumes
/// every manifest has a flow.
#[test]
fn a_project_without_a_flow_still_passes() {
    let dir = TempDir::new().unwrap();
    let manifest = VALID_MANIFEST
        .split("[flow.main.nodes.source]")
        .next()
        .expect("the control manifest has a flow section");
    let project = project_with(dir.path(), manifest);

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(&project)
        .assert()
        .success();
}

/// Every section must reject unknown keys, not only the ones that happened to
/// be tested. A section left open is a section where a typo still vanishes.
#[test]
fn every_section_rejects_an_unknown_key() {
    let cases = [
        ("[torvyn]", "\n[torvyn]\nname = \"p\"\nversion = \"0.1.0\"\ncontract_version = \"0.1.0\"\nverzion = \"9\"\n"),
        ("[[component]]", "\n[[component]]\nname = \"x\"\npath = \".\"\nlangauge = \"go\"\n"),
        ("[runtime]", "\n[runtime]\nmax_memmory = \"1KiB\"\n"),
        ("[build]", "\n[build]\noptimise = true\n"),
        ("[observability]", "\n[observability]\nlevel_of_detail = \"high\"\n"),
        ("[security.grants.x]", "\n[security.grants.x]\ncapabilites = [\"stdio:stdout\"]\n"),
    ];

    for (section, snippet) in cases {
        let dir = TempDir::new().unwrap();
        // The `[torvyn]` case supplies its own table, so drop the original.
        let base = if section == "[torvyn]" {
            VALID_MANIFEST
                .split("[torvyn]")
                .nth(1)
                .and_then(|rest| rest.split_once("\n\n"))
                .map(|(_, rest)| rest.to_owned())
                .expect("the control manifest starts with [torvyn]")
        } else {
            VALID_MANIFEST.to_owned()
        };
        let project = project_with(dir.path(), &format!("{base}{snippet}"));

        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["check"])
            .current_dir(&project)
            .assert()
            .failure()
            .stderr(predicate::str::contains("Unknown configuration"));
    }
}
