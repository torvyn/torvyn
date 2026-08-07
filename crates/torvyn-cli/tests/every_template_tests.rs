//! Every template `torvyn init` offers must scaffold a project whose printed
//! next steps can be completed.
//!
//! `init` finishes by telling the user to run `torvyn check`, `torvyn build`,
//! and `torvyn run`. Seven of the eight templates that once shipped failed at
//! step three: their manifests declared components and no flow, so `run`
//! reported "No flow defined in manifest" and told the user to hand-write a
//! section into a file generated seconds earlier. Three of those seven — the
//! `filter`, `router`, and `aggregator` templates — went further and
//! scaffolded components against `torvyn:streaming/{filter,router,aggregator}`,
//! interfaces that exist in no contract the engine binds; they compiled and
//! were then refused at instantiation. They have been withdrawn until the
//! roadmap's Phase 1 interfaces make them runnable.
//!
//! This file walks the whole of `TemplateKind::ALL` rather than a list written
//! by hand, because a hand-written list falling behind the enum is exactly how
//! that went unnoticed. It stops short of `build`, which needs the component
//! toolchain; `scaffold_end_to_end_tests.rs` carries it through to a running
//! pipeline behind the `scaffold-e2e` feature.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;
use torvyn_cli::cli::TemplateKind;

/// The template name as it is typed on the command line.
fn flag(kind: TemplateKind) -> &'static str {
    match kind {
        TemplateKind::Source => "source",
        TemplateKind::Sink => "sink",
        TemplateKind::Transform => "transform",
        TemplateKind::FullPipeline => "full-pipeline",
        TemplateKind::Empty => "empty",
    }
}

/// Scaffold `kind` into a fresh directory and return the project path.
fn scaffold(workspace: &TempDir, kind: TemplateKind) -> std::path::PathBuf {
    let name = format!("p-{}", flag(kind));
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", &name, "--template", flag(kind)])
        .current_dir(workspace.path())
        .assert()
        .success();
    workspace.path().join(name)
}

/// Every template must produce a manifest `torvyn check` accepts, and must
/// carry the WIT contracts its components are compiled against.
#[test]
fn every_template_passes_check() {
    for &kind in TemplateKind::ALL {
        let workspace = TempDir::new().expect("temp workspace");
        let project = scaffold(&workspace, kind);

        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["check"])
            .current_dir(&project)
            .assert()
            .success();
    }
}

/// `link` is a static check over the manifest, so it needs no compiled Wasm
/// and can assert the topology of every template that declares one.
///
/// It is also the cheapest proof that a scaffolded flow is coherent: every
/// node resolves to a declared component, every edge connects two nodes, the
/// graph is a DAG, and the roles along it are consistent.
#[test]
fn every_template_with_components_links() {
    for &kind in TemplateKind::ALL.iter().filter(|k| k.scaffolds_flow()) {
        let workspace = TempDir::new().expect("temp workspace");
        let project = scaffold(&workspace, kind);

        // Count what the manifest declares rather than predicting it from the
        // template: `link` reporting a different number means it resolved a
        // different set of components than the manifest names.
        let manifest = std::fs::read_to_string(project.join("Torvyn.toml")).expect("manifest");
        let declared = manifest.matches("[[component]]").count();
        assert!(
            declared >= 2,
            "{kind:?} scaffolds {declared} component(s); a pipeline needs a source and a sink"
        );

        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["link"])
            .current_dir(&project)
            .assert()
            .success()
            .stderr(
                predicate::str::contains("links successfully")
                    .and(predicate::str::contains(format!("{declared} components"))),
            );
    }
}

/// The steps `init` prints must be steps this project can take.
///
/// `empty` scaffolds no components on purpose, so it must not advertise
/// `build` and `run`; everything else must.
#[test]
fn printed_next_steps_match_what_was_scaffolded() {
    for &kind in TemplateKind::ALL {
        let workspace = TempDir::new().expect("temp workspace");
        let name = format!("p-{}", flag(kind));
        let assert = Command::cargo_bin("torvyn")
            .unwrap()
            .args(["init", &name, "--template", flag(kind)])
            .current_dir(workspace.path())
            .assert()
            .success();
        let printed = String::from_utf8_lossy(&assert.get_output().stderr).into_owned();

        assert!(
            printed.contains("torvyn check"),
            "{kind:?} did not print `torvyn check`:\n{printed}"
        );
        if kind.scaffolds_flow() {
            assert!(
                printed.contains("torvyn run"),
                "{kind:?} scaffolds a runnable flow but did not print `torvyn run`:\n{printed}"
            );
        } else {
            assert!(
                !printed.contains("torvyn run"),
                "{kind:?} scaffolds no flow, so `torvyn run` cannot be a next step:\n{printed}"
            );
        }
    }
}

/// The project that scaffolds no flow must say what is missing, name the
/// components it does have, and show the block to add — not merely repeat the
/// name of the section.
#[test]
fn a_project_without_a_flow_is_told_what_to_add() {
    let workspace = TempDir::new().expect("temp workspace");
    let project = scaffold(&workspace, TemplateKind::Empty);

    for command in ["run", "link", "trace", "bench"] {
        Command::cargo_bin("torvyn")
            .unwrap()
            .args([command])
            .current_dir(&project)
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("no flow")
                    .and(predicate::str::contains("[flow.main]"))
                    .and(predicate::str::contains("[[flow.main.edges]]")),
            );
    }
}

/// A project named after a companion component would declare that name twice.
/// `init` must refuse it, and say what to do instead.
#[test]
fn a_name_colliding_with_a_companion_is_refused() {
    let workspace = TempDir::new().expect("temp workspace");

    for (template, colliding) in [
        ("transform", "source"),
        ("transform", "sink"),
        ("source", "sink"),
        ("sink", "source"),
    ] {
        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["init", colliding, "--template", template])
            .current_dir(workspace.path())
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("collides")
                    .and(predicate::str::contains(format!("my-{colliding}"))),
            );
    }

    // The collision is specific: a source template ships only a sink, so
    // `torvyn init source --template source` is fine.
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "source", "--template", "source"])
        .current_dir(workspace.path())
        .assert()
        .success();
}

/// The templates withdrawn for scaffolding components the runtime cannot run
/// must no longer be accepted, and the error must list what is available.
#[test]
fn the_withdrawn_role_templates_are_no_longer_offered() {
    let workspace = TempDir::new().expect("temp workspace");

    for withdrawn in ["filter", "router", "aggregator"] {
        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["init", "p", "--template", withdrawn])
            .current_dir(workspace.path())
            .assert()
            .failure()
            .stderr(
                predicate::str::contains("invalid value")
                    .and(predicate::str::contains("transform"))
                    .and(predicate::str::contains("full-pipeline")),
            );
    }
}
