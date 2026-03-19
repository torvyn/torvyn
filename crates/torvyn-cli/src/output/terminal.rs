//! Human-readable terminal output helpers.
//!
//! Provides styled output primitives used by command handlers to render
//! results in a visually clear, consistent format.

use crate::output::OutputContext;

/// Print a success checkmark with a message.
pub fn print_success(ctx: &OutputContext, message: &str) {
    if ctx.color_enabled {
        eprintln!("{} {message}", console::style("\u{2713}").green().bold());
    } else {
        eprintln!("[ok] {message}");
    }
}

/// Print a failure cross with a message.
pub fn print_failure(ctx: &OutputContext, message: &str) {
    if ctx.color_enabled {
        eprintln!("{} {message}", console::style("\u{2717}").red().bold());
    } else {
        eprintln!("[FAIL] {message}");
    }
}

/// Print a header line (e.g., "── Throughput ───────").
#[allow(dead_code)]
pub fn print_header(ctx: &OutputContext, title: &str) {
    let width = ctx.term_width as usize;
    let pad_len = width.saturating_sub(title.len() + 6);
    let line = "\u{2500}".repeat(pad_len.min(60));
    if ctx.color_enabled {
        eprintln!(
            "\n  {} {} {}",
            console::style("\u{2500}\u{2500}").dim(),
            console::style(title).bold(),
            console::style(line).dim()
        );
    } else {
        eprintln!("\n  -- {title} {}", "-".repeat(pad_len.min(60)));
    }
}

/// Print a key-value pair indented by 2 spaces.
pub fn print_kv(ctx: &OutputContext, key: &str, value: &str) {
    if ctx.color_enabled {
        eprintln!("  {}  {value}", console::style(format!("{key}:")).dim());
    } else {
        eprintln!("  {key}:  {value}");
    }
}

/// Print a directory tree.
///
/// # Parameters
/// - `entries`: List of (indent_level, name, is_last_sibling) tuples.
pub fn print_tree(ctx: &OutputContext, entries: &[(usize, &str, bool)]) {
    for (indent, name, is_last) in entries {
        let prefix: String = if *indent == 0 {
            String::new()
        } else {
            let mut p = String::new();
            for _ in 0..(*indent - 1) {
                p.push_str("\u{2502}   ");
            }
            if *is_last {
                p.push_str("\u{2514}\u{2500}\u{2500} ");
            } else {
                p.push_str("\u{251c}\u{2500}\u{2500} ");
            }
            p
        };
        if ctx.color_enabled {
            eprintln!("  {}{}", console::style(&prefix).dim(), name);
        } else {
            eprintln!("  {prefix}{name}");
        }
    }
}

/// Format bytes into human-readable string (e.g., "4.2 MiB").
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
