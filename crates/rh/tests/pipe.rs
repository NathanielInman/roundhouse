//! The CLI writing into a pipe whose reader has gone (`rh status | head`)
//! ends quietly instead of panicking.

#![cfg(unix)]

use std::process::Command;

#[test]
fn a_closed_pipe_ends_the_cli_quietly() {
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let out = Command::new(env!("CARGO_BIN_EXE_rh"))
        .arg("--help")
        .stdout(writer)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "rh panicked: {stderr}");
    assert!(!stderr.contains("Broken pipe"), "rh complained: {stderr}");
}
