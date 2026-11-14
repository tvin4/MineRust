//! Entry-point smoke (Шаг 9, stage G regression).
//!
//! The binary must survive start-up without a tracing double-init panic.
//! `main` installs the non-blocking subscriber and `try_main` used to call
//! `.init()` again, which panicked `SetGlobalDefaultError` before argument
//! parsing - so even `--help` never returned. Keep this test as the guard.

use std::process::Command;

#[test]
fn the_entry_point_does_not_panic_on_a_double_tracing_init() {
    let output = Command::new(env!("CARGO_BIN_EXE_server"))
        .arg("--help")
        .output()
        .expect("the server binary must run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("Unable to install global subscriber"),
        "the entry point panics on a double tracing init: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "the entry point panicked at start-up: {stderr}"
    );
    assert!(
        output.status.success(),
        "--help must exit 0, stderr: {stderr}"
    );
}
