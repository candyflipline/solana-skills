//! End-to-end smoke for v2.21 Slice 1 — `qedgen probe --fuzz <budget> --root <path>`.
//!
//! The test exercises the CLI gate-lift, brownfield runtime detection,
//! handler enumeration, and harness emission. Crucible binary + cargo
//! build are deliberately skipped via `--fuzz 0` (budget-0 emit-and-
//! exit). Validates the headline PRD-v2.21 exit criterion (S1.1 + S1.3
//! + S1.4 — minus the live fuzz run which requires Crucible on PATH).
//!
//! Sanity:
//! - `--fuzz` without `--spec` or `--root` errors clearly.
//! - `--fuzz 0 --root <anchor-crate>` emits `.qed/fuzz/<prog>/`.
//! - The emitted harness contains the PROTOCOL banner and an empty
//!   `invariant_test()` body.
//! - The emitted JSON envelope has `mode: spec_less` and 0 findings.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("qedgen crate should live under <repo>/crates/qedgen")
        .to_path_buf()
}

fn qedgen_bin() -> PathBuf {
    repo_root()
        .join("target")
        .join(profile_dir())
        .join("qedgen")
}

fn profile_dir() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

fn ensure_qedgen_built() {
    if !qedgen_bin().exists() {
        let status = Command::new("cargo")
            .args(["build", "--bin", "qedgen"])
            .current_dir(repo_root())
            .status()
            .expect("spawn cargo build");
        assert!(status.success(), "cargo build qedgen failed");
    }
}

fn write_brownfield_anchor(dir: &Path, name: &str) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[dependencies]
anchor-lang = "0.30"
"#
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("lib.rs"),
        r#"use anchor_lang::prelude::*;
declare_id!("11111111111111111111111111111111");

#[program]
pub mod p {
    use super::*;
    pub fn run(ctx: Context<Empty>) -> Result<()> {
        let _ = ctx;
        Ok(())
    }
    pub fn another(ctx: Context<Empty>, n: u64) -> Result<()> {
        let _ = (ctx, n);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Empty<'info> {
    x: AccountInfo<'info>,
}
"#,
    )
    .unwrap();
}

#[test]
fn fuzz_without_spec_or_root_errors_clearly() {
    ensure_qedgen_built();
    let out = Command::new(qedgen_bin())
        .args(["probe", "--fuzz", "0"])
        .output()
        .expect("spawn qedgen");
    assert!(
        !out.status.success(),
        "expected non-zero exit when neither --spec nor --root is given"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("requires either --spec") && stderr.contains("--root"),
        "error should name both flags; got: {stderr}"
    );
}

#[test]
fn fuzz_zero_brownfield_emits_protocol_harness() {
    ensure_qedgen_built();
    let tmp = tempfile::tempdir().expect("tmpdir");
    write_brownfield_anchor(tmp.path(), "buggy_anchor");

    let out = Command::new(qedgen_bin())
        .args(["probe", "--fuzz", "0", "--root"])
        .arg(tmp.path())
        .output()
        .expect("spawn qedgen");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "qedgen probe failed.\nstdout: {stdout}\nstderr: {stderr}"
    );

    // 1. JSON envelope is spec_less mode with no findings.
    assert!(
        stdout.contains("\"mode\": \"spec_less\""),
        "expected spec_less envelope, got: {stdout}"
    );
    assert!(
        stdout.contains("\"findings\": []"),
        "expected empty findings, got: {stdout}"
    );

    // 2. Harness directory was emitted at .qed/fuzz/<prog>/.
    let harness = tmp.path().join(".qed/fuzz/buggy_anchor");
    assert!(
        harness.join("Cargo.toml").exists(),
        "Cargo.toml missing at {}",
        harness.display()
    );
    let main_rs =
        std::fs::read_to_string(harness.join("src").join("main.rs")).expect("read main.rs");
    assert!(
        main_rs.contains("Mode: PROTOCOL (no spec)"),
        "PROTOCOL banner missing from emitted harness"
    );
    assert!(
        main_rs.contains("fn invariant_test(_fixture:"),
        "protocol-mode emits empty invariant_test body with _fixture; got:\n{main_rs}"
    );
    // Action stubs for both discovered handlers.
    assert!(
        main_rs.contains("pub fn action_run"),
        "action_run stub missing"
    );
    assert!(
        main_rs.contains("pub fn action_another"),
        "action_another stub missing"
    );

    // 3. Budget-0 short-circuit message on stderr.
    assert!(
        stderr.contains("Budget = 0:"),
        "expected budget-0 message; got: {stderr}"
    );
}

/// Pins that the committed v2.21 fixture under
/// `examples/regressions/v2.21-crucible-crash-first/buggy_anchor/`
/// drives the brownfield path end-to-end. Catches regressions that
/// would otherwise only surface when a user manually copies the
/// fixture out of the repo.
#[test]
fn fixture_buggy_anchor_drives_brownfield_emit() {
    ensure_qedgen_built();
    // Copy the fixture out of the repo into a tempdir so emitted
    // `.qed/fuzz/` doesn't pollute the working tree.
    let src = repo_root().join("examples/regressions/v2.21-crucible-crash-first/buggy_anchor");
    assert!(
        src.exists(),
        "fixture missing at {} — did the v2.21 commit land?",
        src.display()
    );
    let tmp = tempfile::tempdir().expect("tmpdir");
    let dst = tmp.path().join("buggy_anchor");
    copy_dir_recursive(&src, &dst);

    let out = Command::new(qedgen_bin())
        .args(["probe", "--fuzz", "0", "--root"])
        .arg(&dst)
        .output()
        .expect("spawn qedgen");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "fixture brownfield emit failed.\nstdout: {stdout}\nstderr: {stderr}"
    );

    let main_rs = dst.join(".qed/fuzz/buggy_anchor/src/main.rs");
    let body = std::fs::read_to_string(&main_rs).expect("read emitted main.rs");
    assert!(body.contains("pub fn action_run"));
    assert!(body.contains("pub fn action_maybe"));
    assert!(body.contains("Mode: PROTOCOL"));
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_recursive(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

#[test]
fn brownfield_errors_when_no_anchor_handlers_found() {
    ensure_qedgen_built();
    let tmp = tempfile::tempdir().expect("tmpdir");
    std::fs::create_dir_all(tmp.path().join("src")).unwrap();
    std::fs::write(
        tmp.path().join("Cargo.toml"),
        "[package]\nname = \"empty\"\nversion = \"0.1.0\"\n\n[dependencies]\nanchor-lang = \"0.30\"\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("src").join("lib.rs"), "// no handlers\n").unwrap();
    let out = Command::new(qedgen_bin())
        .args(["probe", "--fuzz", "0", "--root"])
        .arg(tmp.path())
        .output()
        .expect("spawn qedgen");
    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("No `pub fn"),
        "expected handler-discovery error; got: {stderr}"
    );
}
