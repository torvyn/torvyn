//! Integration tests for CLI argument parsing.

use clap::Parser;
use torvyn_cli::cli::*;

#[test]
fn test_parse_init_defaults() {
    let cli = Cli::parse_from(["torvyn", "init", "my-project"]);
    match &cli.command {
        Command::Init(args) => {
            assert_eq!(args.project_name.as_deref(), Some("my-project"));
            assert_eq!(args.template, TemplateKind::Transform);
            assert_eq!(args.language, Language::Rust);
            assert!(!args.no_git);
            assert!(!args.force);
            assert_eq!(args.contract_version, "0.1.0");
        }
        _ => panic!("Expected Init command"),
    }
}

#[test]
fn test_parse_init_all_flags() {
    let cli = Cli::parse_from([
        "torvyn",
        "init",
        "my-proj",
        "--template",
        "full-pipeline",
        "--language",
        "rust",
        "--no-git",
        "--no-example",
        "--contract-version",
        "0.2.0",
        "--force",
    ]);
    match &cli.command {
        Command::Init(args) => {
            assert_eq!(args.template, TemplateKind::FullPipeline);
            assert!(args.no_git);
            assert!(args.no_example);
            assert_eq!(args.contract_version, "0.2.0");
            assert!(args.force);
        }
        _ => panic!("Expected Init command"),
    }
}

#[test]
fn test_parse_check_defaults() {
    let cli = Cli::parse_from(["torvyn", "check"]);
    match &cli.command {
        Command::Check(args) => {
            assert_eq!(args.manifest.to_str().unwrap(), "./Torvyn.toml");
            assert!(!args.strict);
        }
        _ => panic!("Expected Check command"),
    }
}

#[test]
fn test_parse_check_strict() {
    let cli = Cli::parse_from(["torvyn", "check", "--strict"]);
    match &cli.command {
        Command::Check(args) => assert!(args.strict),
        _ => panic!("Expected Check command"),
    }
}

#[test]
fn test_parse_run_with_limit_and_timeout() {
    let cli = Cli::parse_from(["torvyn", "run", "--limit", "100", "--timeout", "30s"]);
    match &cli.command {
        Command::Run(args) => {
            assert_eq!(args.limit, Some(100));
            assert_eq!(args.timeout.as_deref(), Some("30s"));
        }
        _ => panic!("Expected Run command"),
    }
}

#[test]
fn test_parse_bench_defaults() {
    let cli = Cli::parse_from(["torvyn", "bench"]);
    match &cli.command {
        Command::Bench(args) => {
            assert_eq!(args.duration, "10s");
            assert_eq!(args.warmup, "2s");
            assert_eq!(args.report_format, ReportFormat::Pretty);
        }
        _ => panic!("Expected Bench command"),
    }
}

#[test]
fn test_parse_bench_with_compare() {
    let cli = Cli::parse_from([
        "torvyn",
        "bench",
        "--duration",
        "30s",
        "--warmup",
        "5s",
        "--compare",
        "baseline.json",
        "--baseline",
        "v1",
        "--report-format",
        "json",
    ]);
    match &cli.command {
        Command::Bench(args) => {
            assert_eq!(args.duration, "30s");
            assert_eq!(args.warmup, "5s");
            assert!(args.compare.is_some());
            assert_eq!(args.baseline.as_deref(), Some("v1"));
            assert_eq!(args.report_format, ReportFormat::Json);
        }
        _ => panic!("Expected Bench command"),
    }
}

#[test]
fn test_parse_inspect_with_section() {
    let cli = Cli::parse_from([
        "torvyn",
        "inspect",
        "my-component.wasm",
        "--show",
        "interfaces",
    ]);
    match &cli.command {
        Command::Inspect(args) => {
            assert_eq!(args.target, "my-component.wasm");
            assert_eq!(args.show, InspectSection::Interfaces);
        }
        _ => panic!("Expected Inspect command"),
    }
}

#[test]
fn test_parse_doctor_fix() {
    let cli = Cli::parse_from(["torvyn", "doctor", "--fix"]);
    match &cli.command {
        Command::Doctor(args) => assert!(args.fix),
        _ => panic!("Expected Doctor command"),
    }
}

#[test]
fn test_parse_global_json_format() {
    let cli = Cli::parse_from(["torvyn", "--format", "json", "check"]);
    assert_eq!(cli.global.format, OutputFormat::Json);
}

#[test]
fn test_parse_global_verbose_quiet_conflict() {
    let result = Cli::try_parse_from(["torvyn", "--verbose", "--quiet", "check"]);
    assert!(result.is_err(), "verbose and quiet should conflict");
}

#[test]
fn test_parse_completions() {
    let cli = Cli::parse_from(["torvyn", "completions", "bash"]);
    match &cli.command {
        Command::Completions(args) => assert!(matches!(args.shell, ShellKind::Bash)),
        _ => panic!("Expected Completions command"),
    }
}

#[test]
fn test_parse_trace_with_all_flags() {
    let cli = Cli::parse_from([
        "torvyn",
        "trace",
        "--limit",
        "5",
        "--trace-format",
        "json",
        "--show-buffers",
        "--show-backpressure",
    ]);
    match &cli.command {
        Command::Trace(args) => {
            assert_eq!(args.limit, Some(5));
            assert_eq!(args.trace_format, TraceFormat::Json);
            assert!(args.show_buffers);
            assert!(args.show_backpressure);
        }
        _ => panic!("Expected Trace command"),
    }
}

#[test]
fn test_parse_pack_with_sign() {
    let cli = Cli::parse_from([
        "torvyn",
        "pack",
        "--sign",
        "--tag",
        "v0.1.0",
        "--include-source",
    ]);
    match &cli.command {
        Command::Pack(args) => {
            assert!(args.sign);
            assert_eq!(args.tag.as_deref(), Some("v0.1.0"));
            assert!(args.include_source);
        }
        _ => panic!("Expected Pack command"),
    }
}

#[test]
fn test_parse_publish_dry_run() {
    let cli = Cli::parse_from([
        "torvyn",
        "publish",
        "--dry-run",
        "--registry",
        "https://r.example.com",
    ]);
    match &cli.command {
        Command::Publish(args) => {
            assert!(args.dry_run);
            assert_eq!(args.registry.as_deref(), Some("https://r.example.com"));
        }
        _ => panic!("Expected Publish command"),
    }
}

#[test]
fn test_parse_link_with_flow() {
    let cli = Cli::parse_from(["torvyn", "link", "--flow", "main"]);
    match &cli.command {
        Command::Link(args) => {
            assert_eq!(args.flow.as_deref(), Some("main"));
        }
        _ => panic!("Expected Link command"),
    }
}

#[test]
fn test_all_template_variants_parseable() {
    for template in [
        "source",
        "sink",
        "transform",
        "filter",
        "router",
        "aggregator",
        "full-pipeline",
        "empty",
    ] {
        let cli = Cli::parse_from(["torvyn", "init", "proj", "--template", template]);
        assert!(matches!(cli.command, Command::Init(_)));
    }
}
