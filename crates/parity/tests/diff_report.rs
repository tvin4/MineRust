//! Renders the owner-facing discrepancy reports from the saved traces.
//!
//! Run after the slices have recorded their traces:
//!
//! ```text
//! cargo test -p parity --test diff_report -- --ignored --nocapture
//! ```
//!
//! Every report lands in `target/parity-report/<slice>-diff.md` and is
//! echoed to stdout. Nothing is "fixed" from here: the report is the table
//! the project owner approves (or rejects) before any server change.

use std::path::{Path, PathBuf};

use parity::{diff_traces, render_report, Trace};

/// Where traces land: `<workspace>/target/parity/`.
fn trace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
}

/// Where reports land: `<workspace>/target/parity-report/`.
fn report_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity-report")
}

/// Diffs one pair of traces and writes + prints the Markdown report.
fn report_pair(slice: &str, vanilla_file: &str, ours_file: &str) {
    let vanilla_path = trace_dir().join(vanilla_file);
    let ours_path = trace_dir().join(ours_file);
    if !vanilla_path.exists() || !ours_path.exists() {
        eprintln!(
            "SKIP `{slice}`: run the slice test first ({vanilla_file} / {ours_file} missing)"
        );
        return;
    }
    let vanilla =
        Trace::load(&vanilla_path).unwrap_or_else(|error| panic!("load {vanilla_path:?}: {error}"));
    let ours =
        Trace::load(&ours_path).unwrap_or_else(|error| panic!("load {ours_path:?}: {error}"));
    let diff = diff_traces(&vanilla, &ours);
    let report = render_report(slice, &diff);

    let out = report_dir().join(format!("{slice}-diff.md"));
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("report dir");
    }
    std::fs::write(&out, &report).expect("report written");
    println!("{report}");
    println!("report written to {}", out.display());
}

#[test]
#[ignore = "requires traces recorded by the status and join slice tests"]
fn renders_status_and_join_reports() {
    report_pair("status", "status-vanilla.json", "status-ours.json");
    report_pair("join", "join-vanilla.json", "join-ours.json");
}
