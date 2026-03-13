//! Benchmarking integration.

pub mod report;

pub use report::{
    BenchmarkReport, ComponentBreakdownEntry, DataMovementReport, LatencyReport,
    QueuePressureReport, ResourceUsageReport, ThroughputReport, format_report, generate_report,
};
