//! Table rendering for structured data output.
//!
//! Uses the `tabled` crate for terminal table rendering.

use crate::output::OutputContext;
use tabled::{Table, Tabled};

/// Render a table from a list of items to stderr.
///
/// COLD PATH — called by bench, inspect, doctor.
#[allow(dead_code)]
pub fn render_table<T: Tabled>(ctx: &OutputContext, items: &[T]) {
    if items.is_empty() {
        return;
    }
    let table = Table::new(items);
    let table_str = table.to_string();

    for line in table_str.lines() {
        eprintln!("  {line}");
    }
    let _ = ctx;
}

/// Display an `Option<String>` for tabled rendering.
fn display_option(o: &Option<String>) -> String {
    o.clone().unwrap_or_default()
}

/// A single row in the doctor check output.
#[derive(Debug, Tabled, serde::Serialize)]
#[allow(dead_code)]
pub struct DoctorCheckRow {
    /// The check category or tool name.
    #[tabled(rename = "Check")]
    pub check: String,
    /// Pass/fail status.
    #[tabled(rename = "Status")]
    pub status: String,
    /// Detailed result or version info.
    #[tabled(rename = "Detail")]
    pub detail: String,
    /// Fix suggestion if check failed.
    #[tabled(rename = "Fix", display_with = "display_option")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

/// A row in the inspect interfaces table.
#[derive(Debug, Tabled, serde::Serialize)]
#[allow(dead_code)]
pub struct InterfaceRow {
    /// Direction (export/import).
    #[tabled(rename = "Direction")]
    pub direction: String,
    /// Interface name.
    #[tabled(rename = "Interface")]
    pub interface: String,
}
