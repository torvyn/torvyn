//! End-to-end test for the path a new user actually takes.
//!
//! `torvyn init` prints three next steps. This runs all of them, on a project
//! it scaffolds from scratch, and asserts the pipeline produces the output the
//! template promises.
//!
//! It exists because every one of those steps used to fail. `torvyn build` was
//! advertised in seventeen places and did not exist; a flow node naming a
//! component never resolved to an artifact, so `run` reported "unsupported
//! component reference scheme" naming the component as though it were a
//! malformed URI; the scaffolded WIT had drifted from the contract crate until
//! its `data-source` world no longer exported `lifecycle`, which made the host
//! refuse every generated source. `first_ten_minutes_tests.rs` covers
//! init → check → doctor → pack and stops immediately before the steps that
//! were broken.
//!
//! It now continues past `run` into the packaging path — `pack`, `inspect`,
//! `publish` — which had the same defect in a different place: `pack` wrote
//! 57 bytes of JSON with a `.tar` extension and reported success, `inspect`
//! reported every component as having no exports and no imports, and `publish`
//! printed a `sha256:` digest computed by hashing the artifact's *path*.
//!
//! Requires the component toolchain (`cargo component`, `wasm32-wasip2`),
//! which is why it is behind the `scaffold-e2e` feature rather than in the
//! default test set.

#![cfg(feature = "scaffold-e2e")]

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Building three Wasm components from a cold cargo cache is slow.
const BUILD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(900);

#[test]
fn scaffolded_pipeline_builds_and_runs() {
    let workspace = TempDir::new().expect("temp workspace");

    // 1. Scaffold, exactly as the README's quick start does.
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "my-pipeline", "--template", "full-pipeline"])
        .current_dir(workspace.path())
        .assert()
        .success();

    let project = workspace.path().join("my-pipeline");

    // 2. `torvyn check` must actually read the project's contracts. It used to
    //    look only at `<project>/wit`, miss the per-component directories a
    //    multi-component project uses, and report "0 file(s)" as a pass.
    let check = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["check"])
        .current_dir(&project)
        .assert()
        .success();
    let check_output = String::from_utf8_lossy(&check.get_output().stderr).into_owned();
    let parsed = wit_files_reported(&check_output)
        .unwrap_or_else(|| panic!("check did not report a WIT file count:\n{check_output}"));
    assert!(
        parsed > 0,
        "check reported {parsed} WIT files for a project that ships contracts for three \
         components:\n{check_output}"
    );

    // 3. `torvyn build` — the command `init` tells the user to run.
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["build"])
        .current_dir(&project)
        .timeout(BUILD_TIMEOUT)
        .assert()
        .success();

    // Artifacts land where `run` looks for them. This is the contract between
    // the two commands; if it drifts, the pipeline stops resolving.
    for component in ["source", "transform", "sink"] {
        let artifact = project
            .join(".torvyn/build")
            .join(format!("{component}.wasm"));
        assert!(
            artifact.is_file(),
            "torvyn build did not produce {}",
            artifact.display()
        );
    }

    // 4. `torvyn run` — resolves each node's component name to its artifact,
    //    instantiates all three, and drives the pipeline to completion.
    let run = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["run"])
        .current_dir(&project)
        .timeout(BUILD_TIMEOUT)
        .assert()
        .success();

    let stdout = String::from_utf8_lossy(&run.get_output().stdout).into_owned();
    let stderr = String::from_utf8_lossy(&run.get_output().stderr).into_owned();

    // The template's transform uppercases what the source emits, and the sink
    // prints it — which also proves the sink's `stdio:stdout` capability grant
    // reached the sandbox. A component that prints without the grant produces
    // no output at all.
    assert!(
        stdout.contains("HELLO, TORVYN!"),
        "the pipeline produced no transformed output.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    // And the run must be clean: a pipeline that errors on every element still
    // "succeeds" as a process.
    assert!(
        stderr.contains("Errors:  0") || stderr.contains("Errors: 0"),
        "the run reported errors.\nstderr:\n{stderr}"
    );

    // 5. `torvyn pack` — one artifact per declared component, each a real
    //    gzipped tar rather than a JSON stub with a `.tar` extension.
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["pack"])
        .current_dir(&project)
        .timeout(BUILD_TIMEOUT)
        .assert()
        .success();

    let artifacts_dir = project.join(".torvyn/artifacts");
    for component in ["source", "transform", "sink"] {
        let artifact = artifacts_dir.join(format!("{component}-0.1.0.torvyn"));
        assert!(
            artifact.is_file(),
            "torvyn pack did not produce {}",
            artifact.display()
        );

        // A gzip member starts with 0x1f 0x8b. The stub that used to be written
        // here began with `{`, and `tar -tf` called it an unrecognized format.
        let head = std::fs::read(&artifact).expect("read artifact");
        assert_eq!(
            &head[..2],
            &[0x1f, 0x8b],
            "{} is not gzip-compressed",
            artifact.display()
        );

        // The Wasm binary alone is ~80 KiB, so anything near-empty means the
        // artifact does not carry what it claims to.
        assert!(
            head.len() > 4096,
            "{} is {} bytes — too small to contain a component",
            artifact.display(),
            head.len()
        );
    }

    // 6. `torvyn inspect` — the artifact must round-trip, and the interfaces
    //    must come from the binary. Every component reported `exports: []`
    //    and `imports: []` before, which for a contract-first runtime is the
    //    one thing inspection exists to show.
    let inspect = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["inspect", ".torvyn/artifacts/source-0.1.0.torvyn"])
        .current_dir(&project)
        .assert()
        .success();
    let inspected = String::from_utf8_lossy(&inspect.get_output().stderr).into_owned();

    // A source exports the `source` interface and the `lifecycle` the host
    // requires of it, and imports the contract's shared `types`.
    for expected in [
        "torvyn:streaming/source",
        "torvyn:streaming/lifecycle",
        "torvyn:streaming/types",
    ] {
        assert!(
            inspected.contains(expected),
            "inspect did not report {expected}:\n{inspected}"
        );
    }
    assert!(
        inspected.contains("Exports:") && inspected.contains("Imports:"),
        "inspect reported no interfaces at all:\n{inspected}"
    );

    // 7. `torvyn publish` to a local registry, then verify the digest it
    //    printed is the artifact's real SHA-256 rather than a hash of its path.
    let publish = Command::cargo_bin("torvyn")
        .unwrap()
        .args([
            "publish",
            "--artifact",
            ".torvyn/artifacts/source-0.1.0.torvyn",
        ])
        .current_dir(&project)
        .assert()
        .success();
    let published = String::from_utf8_lossy(&publish.get_output().stderr).into_owned();

    let digest = digest_reported(&published)
        .unwrap_or_else(|| panic!("publish reported no digest:\n{published}"));
    let expected = sha256_hex(&std::fs::read(artifacts_dir.join("source-0.1.0.torvyn")).unwrap());
    assert_eq!(
        digest, expected,
        "publish reported a digest that is not the artifact's SHA-256:\n{published}"
    );

    // The copy in the registry must be byte-identical to what was published.
    let registered = project.join(".torvyn/registry/source-0.1.0.torvyn");
    assert!(
        registered.is_file(),
        "publish did not copy the artifact into the local registry"
    );
    assert_eq!(
        sha256_hex(&std::fs::read(&registered).unwrap()),
        expected,
        "the registry copy does not match the artifact that was published"
    );
}

/// Every template that scaffolds a flow must survive its own printed next
/// steps: `check`, then `build`, then `run`.
///
/// `full-pipeline` is covered above in more depth. This covers the templates
/// that scaffold the component the user is building at the project root, with
/// example components filling the ends of the pipeline it lacks — the shape
/// that made `torvyn run` fail with "No flow defined in manifest" for seven of
/// the eight templates that once shipped.
///
/// Nothing here was reachable by any test before, and all three templates were
/// broken in a different way each: the `source` and `sink` templates imported
/// `LifecycleGuest` and never implemented it, and the `transform` template's
/// pass-through body returned the borrowed input where the contract requires
/// an owned `OutputElement`. All three failed to compile.
#[test]
fn every_scaffolded_template_builds_and_runs() {
    for template in ["source", "sink", "transform"] {
        let workspace = TempDir::new().expect("temp workspace");
        let name = format!("p-{template}");

        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["init", &name, "--template", template])
            .current_dir(workspace.path())
            .assert()
            .success();

        let project = workspace.path().join(&name);

        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["check"])
            .current_dir(&project)
            .assert()
            .success();

        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["build"])
            .current_dir(&project)
            .timeout(BUILD_TIMEOUT)
            .assert()
            .success();

        // Every component the manifest declares must land where `run` looks.
        let manifest = std::fs::read_to_string(project.join("Torvyn.toml")).expect("read manifest");
        let declared = declared_components(&manifest);
        assert!(
            declared.len() >= 2,
            "{template} scaffolds {declared:?}; a pipeline needs a source and a sink"
        );
        for component in &declared {
            let artifact = project
                .join(".torvyn/build")
                .join(format!("{component}.wasm"));
            assert!(
                artifact.is_file(),
                "{template}: torvyn build did not produce {}",
                artifact.display()
            );
        }

        let run = Command::cargo_bin("torvyn")
            .unwrap()
            .args(["run"])
            .current_dir(&project)
            .timeout(BUILD_TIMEOUT)
            .assert()
            .success();

        let stdout = String::from_utf8_lossy(&run.get_output().stdout).into_owned();
        let stderr = String::from_utf8_lossy(&run.get_output().stderr).into_owned();

        // The example source emits numbered greetings and the sink prints
        // them, which also proves the sink's `stdio:stdout` grant reached the
        // sandbox — a component that prints without it produces nothing.
        assert!(
            stdout.contains("Hello, Torvyn!"),
            "{template}: the pipeline produced no output.\nstdout:\n{stdout}\nstderr:\n{stderr}"
        );
        assert!(
            stderr.contains("Errors:  0") || stderr.contains("Errors: 0"),
            "{template}: the run reported errors.\nstderr:\n{stderr}"
        );
    }
}

/// The `[[component]]` names a manifest declares, in order.
fn declared_components(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_component = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_component = line == "[[component]]";
            continue;
        }
        if in_component {
            if let Some(value) = line.strip_prefix("name") {
                if let Some(name) = value.split('=').nth(1) {
                    names.push(name.trim().trim_matches('"').to_owned());
                    in_component = false;
                }
            }
        }
    }
    names
}

/// A pipeline whose component fails must say so, name the component, and exit
/// non-zero.
///
/// This is the case the runtime handled worst. A component returning an error
/// aborted the flow, the sink printed nothing, and `torvyn run` exited 0 with a
/// summary whose only hint was `Errors: 1` between throughput and peak memory.
/// The diagnosis existed the whole time — the driver logs it, and the reactor
/// records the component and the error in the flow's cancellation reason — but
/// the CLI installs no log subscriber, the reactor deleted the reason when it
/// reaped the flow, and `run` never asked.
#[test]
fn a_failing_pipeline_reports_which_component_failed() {
    let workspace = TempDir::new().expect("temp workspace");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "failing", "--template", "transform"])
        .current_dir(workspace.path())
        .assert()
        .success();

    let project = workspace.path().join("failing");

    // Make the transform reject every element with a *non-fatal* error, which
    // is the variant the documentation is most explicit about.
    let lib_rs = project.join("src/lib.rs");
    let source = std::fs::read_to_string(&lib_rs).expect("read lib.rs");
    let marker = "        let data = input.payload.read_all();";
    assert!(
        source.contains(marker),
        "the transform template no longer reads its payload the way this test patches:\n{source}"
    );
    std::fs::write(
        &lib_rs,
        source.replacen(
            marker,
            "        return Err(ProcessError::InvalidInput(format!(\n\
             \x20           \"payload rejected by test at sequence {}\",\n\
             \x20           input.meta.sequence\n\
             \x20       )));\n\
             \x20       #[allow(unreachable_code)]\n\
             \x20       let data = input.payload.read_all();",
            1,
        ),
    )
    .expect("write lib.rs");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["build"])
        .current_dir(&project)
        .timeout(BUILD_TIMEOUT)
        .assert()
        .success();

    let run = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["run"])
        .current_dir(&project)
        .timeout(BUILD_TIMEOUT)
        .assert()
        // Exit code first: a pipeline that produced nothing must not report
        // success, whatever it printed.
        .failure();

    let stdout = String::from_utf8_lossy(&run.get_output().stdout).into_owned();
    let stderr = String::from_utf8_lossy(&run.get_output().stderr).into_owned();

    assert!(
        !stdout.contains("Hello, Torvyn!"),
        "the transform rejected every element, so the sink should have printed \
         nothing:\n{stdout}"
    );

    // The summary still renders — how far the flow got is the first thing
    // worth knowing — and it now says the flow failed.
    assert!(
        stderr.contains("Flow state") && stderr.contains("Failed"),
        "the summary does not report the terminal state:\n{stderr}"
    );

    // And the failure names the flow, the node the manifest declares, the
    // error's category, and the message the component returned — rather than
    // the bare count that used to be the only signal. The flow is `main`; the
    // project is `failing`.
    for expected in [
        "flow \"main\" failed",
        "component \"transform\"",
        "invalid input",
        "payload rejected by test",
    ] {
        assert!(
            stderr.contains(expected),
            "the failure does not mention {expected:?}:\n{stderr}"
        );
    }
}

/// The machine-readable output must carry the same verdict, as one document.
///
/// A JSON consumer has no error line to read, so `success` and the `failure`
/// object are how the failure reaches it — and two JSON values on stdout would
/// break the parse.
#[test]
fn a_failing_pipeline_reports_failure_in_json() {
    let workspace = TempDir::new().expect("temp workspace");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "failing-json", "--template", "transform"])
        .current_dir(workspace.path())
        .assert()
        .success();

    let project = workspace.path().join("failing-json");
    let lib_rs = project.join("src/lib.rs");
    let source = std::fs::read_to_string(&lib_rs).expect("read lib.rs");
    std::fs::write(
        &lib_rs,
        source.replacen(
            "        let data = input.payload.read_all();",
            "        return Err(ProcessError::Fatal(format!(\n\
             \x20           \"unrecoverable at sequence {}\",\n\
             \x20           input.meta.sequence\n\
             \x20       )));\n\
             \x20       #[allow(unreachable_code)]\n\
             \x20       let data = input.payload.read_all();",
            1,
        ),
    )
    .expect("write lib.rs");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["build"])
        .current_dir(&project)
        .timeout(BUILD_TIMEOUT)
        .assert()
        .success();

    let output = Command::cargo_bin("torvyn")
        .unwrap()
        .args(["--format", "json", "run"])
        .current_dir(&project)
        .timeout(BUILD_TIMEOUT)
        .output()
        .expect("run");

    assert!(!output.status.success(), "a failed flow must exit non-zero");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stdout is not a single JSON document ({e}):\n{stdout}"));

    assert_eq!(parsed["success"], serde_json::Value::Bool(false));
    assert_eq!(parsed["data"]["flow_state"], "Failed");
    assert_eq!(parsed["data"]["failed"], serde_json::Value::Bool(true));

    let detail = parsed["failure"]["detail"]
        .as_str()
        .expect("the failure must carry a detail");
    assert!(detail.contains("transform"), "{detail}");
    assert!(detail.contains("unrecoverable"), "{detail}");

    // The reason is carried structurally too, so a consumer need not parse
    // the sentence.
    assert!(parsed["data"]["flow_stopped_because"].is_string());
}

/// `torvyn link` is a static check: it needs the manifest, not compiled Wasm.
/// It used to fail on every project with "invalid type: map, expected a string
/// in `edges.from`", because it parsed the manifest with a private schema that
/// had drifted from `torvyn_config`'s.
#[test]
fn link_validates_the_scaffolded_topology() {
    let workspace = TempDir::new().expect("temp workspace");
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "my-pipeline", "--template", "full-pipeline"])
        .current_dir(workspace.path())
        .assert()
        .success();
    let project = workspace.path().join("my-pipeline");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["link"])
        .current_dir(&project)
        .assert()
        .success()
        .stderr(predicate::str::contains("3 components").and(predicate::str::contains("2 edges")));

    // A topology whose sink is unreachable must be rejected. That is the whole
    // point of the command, and a parser that rejected every manifest could
    // never get far enough to apply it. Dropping the last edge block leaves
    // `sink` with nothing feeding it.
    let manifest_path = project.join("Torvyn.toml");
    let manifest = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let last_edge = manifest
        .rfind("[[flow.main.edges]]")
        .expect("the template declares edges");
    std::fs::write(&manifest_path, &manifest[..last_edge]).expect("write manifest");

    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["link"])
        .current_dir(&project)
        .assert()
        .failure()
        .stderr(predicate::str::contains("sink"));
}

/// Extract the digest `publish` reported, without its `sha256:` prefix.
fn digest_reported(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.split("sha256:").nth(1))
        .map(|rest| rest.trim().to_owned())
}

/// SHA-256 of a byte slice, as lowercase hex.
///
/// Uses the packaging crate's digest so the test needs no second SHA-256
/// implementation; `ContentDigest` is pinned against a known vector in its own
/// unit tests, and the assertion below pins it again through this path.
fn sha256_hex(bytes: &[u8]) -> String {
    torvyn_packaging::ContentDigest::of_bytes(bytes).hex
}

/// `torvyn run` must refuse an option it cannot honour rather than accepting
/// it and behaving as though it were never typed.
#[test]
fn run_rejects_options_it_cannot_honour() {
    let workspace = TempDir::new().expect("temp workspace");
    Command::cargo_bin("torvyn")
        .unwrap()
        .args(["init", "my-pipeline", "--template", "full-pipeline"])
        .current_dir(workspace.path())
        .assert()
        .success();
    let project = workspace.path().join("my-pipeline");

    for option in ["--limit", "--input", "--output"] {
        Command::cargo_bin("torvyn")
            .unwrap()
            .args(["run", option, "10"])
            .current_dir(&project)
            .assert()
            .failure()
            .stderr(predicate::str::contains("is not implemented"));
    }
}

/// Extract the WIT file count from `check`'s human output.
fn wit_files_reported(output: &str) -> Option<usize> {
    let line = output
        .lines()
        .find(|line| line.contains("WIT contracts parsed"))?;
    let start = line.find('(')? + 1;
    let rest = &line[start..];
    let end = rest.find(' ')?;
    rest[..end].parse().ok()
}

#[cfg(test)]
mod unit {
    use super::{declared_components, digest_reported, sha256_hex, wit_files_reported};

    #[test]
    fn reads_every_declared_component_name() {
        let manifest = r#"
[torvyn]
name = "p"

[[component]]
name = "p"
path = "."

[[component]]
name = "sink"
path = "components/sink"

[flow.main.nodes.source]
component = "p"
"#;
        assert_eq!(declared_components(manifest), vec!["p", "sink"]);
        assert!(declared_components("[torvyn]\nname = \"p\"\n").is_empty());
    }

    #[test]
    fn parses_the_reported_digest() {
        assert_eq!(
            digest_reported("  Digest:  sha256:abc123\n  Size:  35.2 KiB"),
            Some("abc123".to_owned())
        );
        assert_eq!(digest_reported("[ok] Published: local:/x.torvyn"), None);
    }

    #[test]
    fn sha256_matches_the_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn parses_the_reported_file_count() {
        assert_eq!(
            wit_files_reported("[ok] WIT contracts parsed (21 file(s), 0 errors)"),
            Some(21)
        );
        assert_eq!(
            wit_files_reported("[ok] WIT contracts parsed (0 file(s), 0 errors)"),
            Some(0)
        );
        assert_eq!(wit_files_reported("[ok] Torvyn.toml is valid"), None);
    }
}
