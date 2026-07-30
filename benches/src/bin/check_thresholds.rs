//! Benchmark regression gate.
//!
//! Reads criterion's own JSON output from a completed `cargo bench` run and
//! compares each benchmark's median against a ceiling committed in
//! `benches/thresholds.json`. Exits non-zero if any benchmark is missing or
//! over its ceiling, which is what turns "we publish benchmarks" into "a
//! regression fails the build".
//!
//! ```text
//! cargo bench -p torvyn-benchmarks --features real-wasm
//! cargo run  -p torvyn-benchmarks --release --bin check-thresholds
//! ```
//!
//! # What the ceilings are, and are not
//!
//! They are **order-of-magnitude regression detectors**, not SLOs. They are
//! set several times above the numbers a developer machine produces, because
//! this has to pass on shared CI runners whose per-core throughput varies by
//! a large factor between runs and generations. A benchmark that drifts 30%
//! slower will not trip them; one that regresses 5× will. Treat a failure as
//! "something structural changed", and the published numbers in
//! `benches/README.md` as the real baseline to compare against.
//!
//! Tightening a ceiling is welcome when a platform's variance is understood.
//! Raising one to make a red build green is not: raise it only alongside a
//! deliberate, explained change in what the runtime does.
//!
//! # Arguments
//!
//! ```text
//! check-thresholds [--criterion-dir <path>] [--thresholds <path>]
//! ```
//!
//! Defaults: `target/criterion` and `benches/thresholds.json`, both resolved
//! relative to the current working directory (the workspace root under
//! `cargo run`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

/// A benchmark result recovered from criterion's output.
struct Measured {
    /// Median estimate, in nanoseconds.
    median_ns: f64,
}

fn main() -> ExitCode {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("check-thresholds: {msg}");
            eprintln!("usage: check-thresholds [--criterion-dir <path>] [--thresholds <path>]");
            return ExitCode::FAILURE;
        }
    };

    let thresholds = match load_thresholds(&args.thresholds) {
        Ok(t) => t,
        Err(msg) => {
            eprintln!("check-thresholds: {msg}");
            return ExitCode::FAILURE;
        }
    };

    if !args.criterion_dir.is_dir() {
        eprintln!(
            "check-thresholds: no criterion output at {}. Run `cargo bench -p torvyn-benchmarks \
             --features real-wasm` first.",
            args.criterion_dir.display()
        );
        return ExitCode::FAILURE;
    }

    let mut measured = BTreeMap::new();
    collect(&args.criterion_dir, &mut measured);

    if measured.is_empty() {
        eprintln!(
            "check-thresholds: found no benchmark results under {}.",
            args.criterion_dir.display()
        );
        return ExitCode::FAILURE;
    }

    let mut failures = Vec::new();
    let mut checked = 0usize;

    for (id, ceiling_ns) in &thresholds {
        match measured.get(id) {
            None => failures.push(format!(
                "MISSING  {id}\n           no result found. If this run omitted \
                 `--features real-wasm`, the real-Wasm benchmarks did not execute."
            )),
            Some(m) => {
                checked += 1;
                if m.median_ns > *ceiling_ns {
                    failures.push(format!(
                        "SLOW     {id}\n           median {} exceeds ceiling {} ({:.2}x)",
                        format_ns(m.median_ns),
                        format_ns(*ceiling_ns),
                        m.median_ns / ceiling_ns,
                    ));
                } else {
                    println!(
                        "ok       {id}: {} (ceiling {}, {:.0}% of budget)",
                        format_ns(m.median_ns),
                        format_ns(*ceiling_ns),
                        100.0 * m.median_ns / ceiling_ns,
                    );
                }
            }
        }
    }

    // Results with no threshold are reported, not failed: adding a benchmark
    // should not require touching this file in the same commit.
    for id in measured.keys() {
        if !thresholds.contains_key(id) {
            println!("note     {id}: no threshold configured");
        }
    }

    if failures.is_empty() {
        println!("\ncheck-thresholds: {checked} benchmark(s) within budget.");
        ExitCode::SUCCESS
    } else {
        eprintln!("\ncheck-thresholds: {} failure(s):", failures.len());
        for f in &failures {
            eprintln!("  {f}");
        }
        eprintln!(
            "\nSee the header of benches/src/bin/check_thresholds.rs before changing \
             any ceiling."
        );
        ExitCode::FAILURE
    }
}

/// Command-line arguments.
struct Args {
    criterion_dir: PathBuf,
    thresholds: PathBuf,
}

impl Args {
    fn parse<I: Iterator<Item = String>>(mut it: I) -> Result<Self, String> {
        let mut criterion_dir = PathBuf::from("target/criterion");
        let mut thresholds = PathBuf::from("benches/thresholds.json");
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--criterion-dir" => {
                    criterion_dir = it
                        .next()
                        .map(PathBuf::from)
                        .ok_or("--criterion-dir needs a value")?;
                }
                "--thresholds" => {
                    thresholds = it
                        .next()
                        .map(PathBuf::from)
                        .ok_or("--thresholds needs a value")?;
                }
                other => return Err(format!("unrecognised argument '{other}'")),
            }
        }
        Ok(Self {
            criterion_dir,
            thresholds,
        })
    }
}

/// Load `{"benchmarks": {"<full id>": {"max_median_ns": <number>}}}`.
fn load_thresholds(path: &Path) -> Result<BTreeMap<String, f64>, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read thresholds file {}: {e}", path.display()))?;
    let doc: Value =
        serde_json::from_str(&raw).map_err(|e| format!("cannot parse {}: {e}", path.display()))?;

    let entries = doc
        .get("benchmarks")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{}: expected a 'benchmarks' object", path.display()))?;

    let mut out = BTreeMap::new();
    for (id, entry) in entries {
        let ceiling = entry
            .get("max_median_ns")
            .and_then(Value::as_f64)
            .ok_or_else(|| format!("{}: '{id}' has no numeric 'max_median_ns'", path.display()))?;
        out.insert(id.clone(), ceiling);
    }
    Ok(out)
}

/// Walk `dir` collecting every `new/benchmark.json` + `new/estimates.json`
/// pair criterion wrote.
///
/// Walking for the pair rather than reconstructing paths from benchmark ids
/// keeps this working regardless of how criterion sanitises directory names.
fn collect(dir: &Path, out: &mut BTreeMap<String, Measured>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.file_name().is_some_and(|n| n == "benchmark.json") {
            // Only `new/` holds the current run; `base/` and `change/` are
            // criterion's comparison bookkeeping from previous runs.
            if path
                .parent()
                .and_then(Path::file_name)
                .is_none_or(|n| n != "new")
            {
                continue;
            }
            if let Some((id, measured)) = read_result(&path) {
                out.insert(id, measured);
            }
        }
    }
}

/// Read one benchmark's id and median from criterion's JSON pair.
fn read_result(benchmark_json: &Path) -> Option<(String, Measured)> {
    let raw = std::fs::read_to_string(benchmark_json).ok()?;
    let doc: Value = serde_json::from_str(&raw).ok()?;
    let id = doc.get("full_id").and_then(Value::as_str)?.to_owned();

    let estimates_path = benchmark_json.with_file_name("estimates.json");
    let raw = std::fs::read_to_string(estimates_path).ok()?;
    let doc: Value = serde_json::from_str(&raw).ok()?;
    let median_ns = doc
        .get("median")
        .and_then(|m| m.get("point_estimate"))
        .and_then(Value::as_f64)?;

    Some((id, Measured { median_ns }))
}

/// Render nanoseconds in the largest unit that keeps the number readable.
fn format_ns(ns: f64) -> String {
    if ns >= 1_000_000_000.0 {
        format!("{:.3} s", ns / 1_000_000_000.0)
    } else if ns >= 1_000_000.0 {
        format!("{:.3} ms", ns / 1_000_000.0)
    } else if ns >= 1_000.0 {
        format!("{:.3} µs", ns / 1_000.0)
    } else {
        format!("{ns:.1} ns")
    }
}
