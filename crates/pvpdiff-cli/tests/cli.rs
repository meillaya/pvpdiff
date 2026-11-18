//! CLI smoke tests for the P0 binary skeleton.

use assert_cmd::Command;
use predicates::str::contains;

#[test]
fn version_flag_prints_name_and_version() {
    Command::cargo_bin("pvpdiff")
        .expect("binary is built for the test")
        .arg("--version")
        .assert()
        .success()
        .stdout(contains("pvpdiff"))
        .stdout(contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_mentions_the_tool() {
    Command::cargo_bin("pvpdiff")
        .expect("binary is built for the test")
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("Package Versioning Policy"));
}
