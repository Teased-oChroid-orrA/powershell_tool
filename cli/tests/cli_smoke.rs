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

/// `--help`'s own example for `--extensions` is dotless ("txt,log,pdf"),
/// which must actually work, not silently match zero files - a real gap
/// found investigating a user-reported PDF extraction issue: `--extensions
/// pdf` (following the documented example literally) matched nothing,
/// since `extension_catalog`/`filter_by_extension` always store/compare
/// extensions with a leading dot.
#[test]
fn extensions_flag_works_with_or_without_a_leading_dot() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "torque spec\n").unwrap();
    let out_dir = dir.path().join("out");

    let output = Command::new(env!("CARGO_BIN_EXE_search-cli"))
        .arg(dir.path())
        .args(["-f", "torque"])
        .args(["-o", out_dir.to_str().unwrap()])
        .args(["--extensions", "txt"]) // no leading dot, matching --help's own example
        .output()
        .expect("failed to run search-cli");

    assert!(output.status.success(), "stderr: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("1 file(s) with hits"), "stdout: {stdout}");
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

/// Regression test for a field-reported "the app doesn't launch at all"
/// bug specific to double-clicking `search-cli.exe` on Windows. A bare
/// double-click passes zero arguments (argv = [program name] only); clap's
/// `required_unless_present_any` on `search_path`/`filters` doesn't know
/// "no args" means "go interactive" - it printed a missing-required-
/// argument error and exited with clap's usage-error code (2) before
/// `main` ever checked `cli.interactive`, even though
/// docs/deployment-rust.md documents bare invocation as the
/// interactive-menu entry point. On Windows this reads as the console
/// window flashing open and closing with nothing readable in it.
///
/// This test can't drive the interactive prompts themselves (dialoguer
/// needs a real terminal, not a piped test harness), but it proves the
/// process no longer takes clap's hard-exit(2) usage-error path: it must
/// print the interactive banner and fail only once dialoguer itself
/// detects a non-terminal stdin/stdout, not before.
#[test]
fn bare_invocation_with_no_args_attempts_interactive_mode_not_a_clap_usage_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_search-cli"))
        .output()
        .expect("failed to run search-cli");

    assert_ne!(output.status.code(), Some(2), "must not take clap's required-argument usage-error exit path");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("interactive mode"), "stdout: {stdout}");
}
