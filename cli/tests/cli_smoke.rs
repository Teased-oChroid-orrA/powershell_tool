//! Real end-to-end smoke test of the compiled CLI binary - spawns the
//! actual `search-cli` executable (not a call into its internals) against
//! real files on disk, matching this project's established "verify
//! against the real thing, not a synthetic shortcut" discipline.
//! `CARGO_BIN_EXE_search-cli` is a Cargo-provided env var pointing at the
//! just-built binary for this package - no extra test-only dependency
//! (`assert_cmd` etc.) needed for this.

use std::process::Command;

#[test]
fn finds_a_hit_and_writes_a_report() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "apple pie recipe with torque specs\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "unrelated content\n").unwrap();
    let out_dir = dir.path().join("out");

    let output = Command::new(env!("CARGO_BIN_EXE_search-cli"))
        .arg(dir.path())
        .args(["-f", "torque"])
        .args(["-o", out_dir.to_str().unwrap()])
        .output()
        .expect("failed to run search-cli");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 file(s) with hits"), "stdout: {stdout}");

    let reports: Vec<_> = std::fs::read_dir(&out_dir).unwrap().filter_map(|e| e.ok()).collect();
    assert_eq!(reports.len(), 1, "expected exactly one report file");
    let html = std::fs::read_to_string(reports[0].path()).unwrap();
    assert!(html.contains("<mark>torque</mark>"), "report should highlight the match");
}

#[test]
fn dry_run_reads_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "torque spec\n").unwrap();
    let out_dir = dir.path().join("out");

    let output = Command::new(env!("CARGO_BIN_EXE_search-cli"))
        .arg(dir.path())
        .args(["-f", "torque"])
        .args(["-o", out_dir.to_str().unwrap()])
        .arg("--dry-run")
        .output()
        .expect("failed to run search-cli");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Dry run:"));
    assert!(!out_dir.exists(), "dry run must not create the output folder");
}

#[test]
fn csv_and_json_flags_write_those_files_too() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "torque spec\n").unwrap();
    let out_dir = dir.path().join("out");

    let output = Command::new(env!("CARGO_BIN_EXE_search-cli"))
        .arg(dir.path())
        .args(["-f", "torque"])
        .args(["-o", out_dir.to_str().unwrap()])
        .args(["--csv", "--json"])
        .output()
        .expect("failed to run search-cli");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let extensions: Vec<String> = std::fs::read_dir(&out_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path().extension().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(extensions.contains(&"html".to_string()));
    assert!(extensions.contains(&"csv".to_string()));
    assert!(extensions.contains(&"json".to_string()));
}

#[test]
fn a_bad_regex_filter_is_a_clean_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "torque spec\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_search-cli"))
        .arg(dir.path())
        .args(["-f", "("])
        .arg("--regex")
        .output()
        .expect("failed to run search-cli");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Error"));
}
