//! Phase 1d MIR snapshot equivalence — `docs/design/qedgen-mir-sketch.md`
//! §"Phase 1d — snapshot equivalence".
//!
//! For every pilot fixture (`examples/rust/{escrow, escrow-split,
//! lending, multisig, bundled-stdlib-demo, cross-program-vault}`),
//! regenerates the MIR-rendered `Spec.lean` and compares against a
//! checked-in snapshot at `tests/snapshots/<fixture>.Spec.lean`.
//!
//! When the snapshot diverges, the test prints the unified diff and
//! fails — silent drift between MIR + legacy renderers is detectable
//! immediately. Refreshing a snapshot is intentional and requires
//! re-running the fixture binary + committing the updated file
//! (`UPDATE_SNAPSHOTS=1 cargo test --test mir_snapshot` writes them
//! in place).
//!
//! Path coverage across fixtures (post-v2.33, when `pragma state_repr =
//! adt` became the explicit opt-in for the inductive multi-variant
//! State, replacing the incidental `WrongState`-error footgun):
//!   * Flat path (escrow, escrow-split, bundled-stdlib-demo, lending):
//!     the default `structure State` + `status` discriminant. These
//!     were legacy ADT byte-identity fixtures before the representation
//!     default flipped to flat.
//!   * ADT path (cross-program-vault): declares `pragma state_repr =
//!     adt` — its hand-written instruction logic destructures the
//!     inner-enum, so it is the bundled `inductive State` /
//!     `render_single_account_adt` showcase. The dispatch itself (same
//!     shape ⇒ flat vs ADT by pragma) is additionally unit-tested by
//!     `lean_gen_mir::tests::state_repr_pragma_dispatches_inductive_vs_flat`.
//!   * Indexed path (multisig): byte-identical post Phase 1e
//!     indexed-state lowering (Mathlib + IndexedState imports,
//!     `Map[N] T` capacity, `Function.update` collapse).
//!   * Multi-account path (lending): byte-identical post Phase 2
//!     multi-account renderer (per-account `<Name>State` structures,
//!     per-group `apply<Name>Op` dispatchers, per-property
//!     environment scoping, per-via-op liveness scoping).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("qedgen crate at <repo>/crates/qedgen")
        .to_path_buf()
}

fn qedgen_bin() -> PathBuf {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    repo_root().join("target").join(profile).join("qedgen")
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

fn snapshots_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

/// Run `qedgen codegen --spec <spec> --lean` in an isolated tempdir
/// and return the rendered `Spec.lean` string.
///
/// MIR (`lean_gen_mir`) is the sole Lean-codegen path — the legacy
/// `lean_gen` renderer and its `QEDGEN_LEGACY_LEAN` escape hatch were
/// deleted in v2.32.
///
/// `qedgen` is git-native by design ([[project-git-native]]); the
/// tempdir is initialized as an empty git repo so the codegen
/// proceeds without colliding with the workspace's git state.
fn render_mir_spec(spec_arg: &str) -> String {
    ensure_qedgen_built();
    let tmp = tempfile::tempdir().expect("create tempdir");
    let spec_path = repo_root().join(spec_arg);

    let git_init = Command::new("git")
        .arg("init")
        .arg("--quiet")
        .current_dir(tmp.path())
        .status()
        .expect("spawn git init");
    assert!(
        git_init.success(),
        "git init failed in {}",
        tmp.path().display()
    );

    let status = Command::new(qedgen_bin())
        .arg("codegen")
        .arg("--spec")
        .arg(&spec_path)
        .arg("--lean")
        .current_dir(tmp.path())
        .status()
        .expect("spawn qedgen codegen");
    assert!(status.success(), "qedgen codegen failed for {}", spec_arg);

    let out = tmp.path().join("formal_verification").join("Spec.lean");
    fs::read_to_string(&out).unwrap_or_else(|e| panic!("read {}: {e}", out.display()))
}

fn assert_or_update_snapshot(fixture: &str, spec_arg: &str) {
    let rendered = render_mir_spec(spec_arg);
    let snapshot_path = snapshots_dir().join(format!("{}.Spec.lean", fixture));
    let update = std::env::var("UPDATE_SNAPSHOTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if update {
        fs::create_dir_all(snapshots_dir()).expect("create snapshots dir");
        fs::write(&snapshot_path, &rendered)
            .unwrap_or_else(|e| panic!("write {}: {e}", snapshot_path.display()));
        eprintln!("UPDATE_SNAPSHOTS=1: wrote {}", snapshot_path.display());
        return;
    }

    let expected = fs::read_to_string(&snapshot_path).unwrap_or_else(|e| {
        panic!(
            "missing snapshot {}: {e}\n\
             Run with UPDATE_SNAPSHOTS=1 to seed it.",
            snapshot_path.display()
        )
    });

    if expected != rendered {
        // Print a compact unified diff so the failure shows the
        // offending region without the whole file.
        let diff = diff_unified(&expected, &rendered);
        panic!(
            "{fixture}: MIR snapshot drift detected.\n\
             Snapshot: {}\n\
             Re-run with UPDATE_SNAPSHOTS=1 to refresh (then inspect the diff before \
             committing).\n\
             {diff}",
            snapshot_path.display()
        );
    }
}

/// Produce a unified-diff string between two multiline texts. Avoids
/// pulling in an extra crate; the output isn't IDE-grade but suffices
/// for test failure messages.
fn diff_unified(expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let mut out = String::new();
    out.push_str("--- snapshot\n+++ rendered\n");
    let max = exp_lines.len().max(act_lines.len());
    let mut printed = 0usize;
    let max_lines = 80usize;
    for i in 0..max {
        let e = exp_lines.get(i).copied().unwrap_or("");
        let a = act_lines.get(i).copied().unwrap_or("");
        if e != a {
            if printed >= max_lines {
                out.push_str("... (diff truncated)\n");
                break;
            }
            out.push_str(&format!("@@ line {} @@\n", i + 1));
            if !e.is_empty() || i < exp_lines.len() {
                out.push_str(&format!("-{}\n", e));
            }
            if !a.is_empty() || i < act_lines.len() {
                out.push_str(&format!("+{}\n", a));
            }
            printed += 1;
        }
    }
    out
}

// ---- Per-fixture snapshot tests ----
//
// Each test is small + boilerplate-light so failures point at one
// fixture. Adding a new pilot fixture: drop a new test with the same
// shape + run `UPDATE_SNAPSHOTS=1 cargo test --test mir_snapshot
// <new_fixture_name>` once to seed.

#[test]
fn snapshot_escrow() {
    assert_or_update_snapshot("escrow", "examples/rust/escrow/escrow.qedspec");
}

#[test]
fn snapshot_lending() {
    assert_or_update_snapshot("lending", "examples/rust/lending/lending.qedspec");
}

#[test]
fn snapshot_multisig() {
    assert_or_update_snapshot("multisig", "examples/rust/multisig/multisig.qedspec");
}

// Record-bearing fixture (v2.32 records→MIR migration). Verified
// byte-identical to the legacy `lean_gen` output (the records carve-out
// in main.rs is removed on this branch). Gates the gap-1/2/3 fixes:
// record `structure`/`Inhabited` emission, indexed-record-field effect
// rendering, and requires-conjunct ordering.
#[test]
fn snapshot_percolator() {
    assert_or_update_snapshot("percolator", "examples/rust/percolator/percolator.qedspec");
}

#[test]
fn snapshot_bundled_stdlib_demo() {
    assert_or_update_snapshot(
        "bundled-stdlib-demo",
        "examples/rust/bundled-stdlib-demo/pool.qedspec",
    );
}

#[test]
fn snapshot_cross_program_vault() {
    assert_or_update_snapshot(
        "cross-program-vault",
        "examples/rust/cross-program-vault/vault.qedspec",
    );
}

#[test]
fn snapshot_escrow_split() {
    assert_or_update_snapshot("escrow-split", "examples/rust/escrow-split");
}
