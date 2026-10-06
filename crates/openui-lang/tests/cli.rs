//! The `openui-lang` command-line tool.

use std::path::{Path, PathBuf};
use std::process::Command;

fn conformance() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance")
}

#[test]
fn parse_prints_the_reference_parse_result() {
    let dir = conformance();
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("parse")
        .arg(dir.join("schemas/text.json"))
        .arg(dir.join("cases/hello-text.oui"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let expected = std::fs::read_to_string(dir.join("golden/hello-text.oneshot.json")).unwrap();
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{expected}\n")
    );
}

#[test]
fn unknown_command_prints_usage_and_fails() {
    let out = Command::new(env!("CARGO_BIN_EXE_openui-lang"))
        .arg("frobnicate")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage: openui-lang parse"));
}
