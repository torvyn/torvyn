//! Benchmarking integration.

pub mod report;

pub use report::{
    format_report, generate_report, BenchmarkReport, ComponentBreakdownEntry, DataMovementReport,
    LatencyReport, QueuePressureReport, ResourceUsageReport, ThroughputReport,
};
