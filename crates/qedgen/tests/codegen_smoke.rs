use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("qedgen crate should live under <repo>/crates/qedgen")
        .to_path_buf()
}

fn run(command: &mut Command) {
    let output = command.output().expect("failed to spawn command");
    if !output.status.success() {
        panic!(
            "command failed with status {}\nstdout:\n{}\nstderr:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Generate `<example>` as an Anchor scaffold into a fresh tempdir, write
/// a `[patch]` config pointing `qedgen-macros` at the in-repo crate, and
/// run `cargo check` on the result. Don't pass `--offline` — CI runners
/// start cold, so the smoke needs to be allowed to fetch anchor-lang /
/// anchor-spl / solana-program on first run; locally cargo's registry
/// cache makes the second run fast.
fn smoke_anchor_scaffold(example: &str) {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec_src = repo_root()
        .join("examples/rust")
        .join(example)
        .join(format!("{example}.qedspec"));
    let spec_path = temp.path().join(format!("{example}.qedspec"));
    std::fs::copy(&spec_src, &spec_path).unwrap_or_else(|e| panic!("copy {} spec: {e}", example));

    std::fs::copy(
        repo_root()
            .join("examples/rust")
            .join(example)
            .join("qed.toml"),
        temp.path().join("qed.toml"),
    )
    .unwrap_or_else(|e| panic!("copy {} manifest: {e}", example));
    std::fs::create_dir(temp.path().join(".qed")).expect("create .qed");

    run(Command::new("git").arg("init").current_dir(temp.path()));

    let output_dir = temp.path().join("programs");
    run(Command::new(env!("CARGO_BIN_EXE_qedgen"))
        .arg("codegen")
        .arg("--spec")
        .arg(&spec_path)
        .arg("--target")
        .arg("anchor")
        .arg("--output-dir")
        .arg(&output_dir)
        .current_dir(temp.path()));

    // The generated Cargo.toml stamps `qedgen-macros = { git = ..., tag =
    // "v<current>" }`, but the tag for the in-progress version doesn't
    // exist on GitHub until release time. Rewrite the git dep to a path
    // dep before the smoke compile — cargo's `[patch]` mechanism didn't
    // reliably override a tagged git source even with the right config.
    redirect_macros_to_path(&output_dir.join("Cargo.toml"));

    run(Command::new("cargo")
        .arg("check")
        .arg("--manifest-path")
        .arg(output_dir.join("Cargo.toml")));
}

/// Anchor scaffold smoke + run the generated proptest harness against
/// it. Raises the floor from "compiles" to "tests pass" — catches
/// regressions in the predicate / transition rendering that pure
/// `cargo check` would miss (e.g., the Pubkey-effect-filter bug Day 1
/// surfaced on token-fundraiser).
fn smoke_anchor_scaffold_with_proptest(example: &str) {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec_src = repo_root()
        .join("examples/rust")
        .join(example)
        .join(format!("{example}.qedspec"));
    let spec_path = temp.path().join(format!("{example}.qedspec"));
    std::fs::copy(&spec_src, &spec_path).unwrap_or_else(|e| panic!("copy {} spec: {e}", example));

    std::fs::copy(
        repo_root()
            .join("examples/rust")
            .join(example)
            .join("qed.toml"),
        temp.path().join("qed.toml"),
    )
    .unwrap_or_else(|e| panic!("copy {} manifest: {e}", example));
    std::fs::create_dir(temp.path().join(".qed")).expect("create .qed");

    run(Command::new("git").arg("init").current_dir(temp.path()));

    let output_dir = temp.path().join("programs");
    run(Command::new(env!("CARGO_BIN_EXE_qedgen"))
        .arg("codegen")
        .arg("--spec")
        .arg(&spec_path)
        .arg("--target")
        .arg("anchor")
        .arg("--output-dir")
        .arg(&output_dir)
        .current_dir(temp.path()));
    run(Command::new(env!("CARGO_BIN_EXE_qedgen"))
        .arg("codegen")
        .arg("--spec")
        .arg(&spec_path)
        .arg("--proptest")
        .arg("--proptest-output")
        .arg(output_dir.join("tests/proptest.rs"))
        .current_dir(temp.path()));

    // proptest is a dev-dependency on the test crate; the generator
    // emits Cargo.toml without dev-deps because production Anchor
    // builds don't need it. Append it for the smoke run, and (see
    // `smoke_anchor_scaffold`) rewrite the qedgen-macros git dep to a
    // path dep so the unreleased tag doesn't fail to resolve.
    let cargo_toml = output_dir.join("Cargo.toml");
    redirect_macros_to_path(&cargo_toml);
    let mut manifest = std::fs::read_to_string(&cargo_toml).expect("read Cargo.toml");
    manifest.push_str("\n[dev-dependencies]\nproptest = \"1\"\n");
    std::fs::write(&cargo_toml, manifest).expect("rewrite Cargo.toml");

    run(Command::new("cargo")
        .arg("test")
        .arg("--manifest-path")
        .arg(&cargo_toml)
        .arg("--test")
        .arg("proptest"));
}

/// Generate `<spec>` as a Pinocchio scaffold into a fresh tempdir and run
/// `cargo build` on it. The Pinocchio path is MIR-native (slice 6): the
/// scaffold emits lib + entrypoint + byte-dispatch, zeropod state, guards,
/// errors, checked effects, and SPL Token CPIs (`call Token.transfer(...)`
/// → `pinocchio_token::instructions::Transfer { … }.invoke()?;`).
///
/// `cargo build` (not `check`) so the `#![no_std]` + `entrypoint!` crate is
/// exercised through codegen. The spec carries an inline SPL `interface`,
/// so no `qed.toml` is needed. Regenerating from the committed spec (rather
/// than building a checked-in tree) keeps the gate testing *current*
/// codegen output — it can't silently drift.
fn smoke_pinocchio_scaffold(fixture: &str, spec_file: &str) {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec_src = repo_root()
        .join("examples/pinocchio-fixtures")
        .join(fixture)
        .join(spec_file);
    let spec_path = temp.path().join(spec_file);
    std::fs::copy(&spec_src, &spec_path).unwrap_or_else(|e| panic!("copy {fixture} spec: {e}"));
    std::fs::create_dir(temp.path().join(".qed")).expect("create .qed");

    run(Command::new("git").arg("init").current_dir(temp.path()));

    let output_dir = temp.path().join("programs");
    run(Command::new(env!("CARGO_BIN_EXE_qedgen"))
        .arg("codegen")
        .arg("--spec")
        .arg(&spec_path)
        .arg("--target")
        .arg("pinocchio")
        .arg("--output-dir")
        .arg(&output_dir)
        .current_dir(temp.path()));

    // Same unreleased-tag problem as the Anchor smoke: rewrite the
    // `qedgen-macros` git dep to the in-repo path dep before compiling.
    redirect_macros_to_path(&output_dir.join("Cargo.toml"));

    run(Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(output_dir.join("Cargo.toml")));
}

/// Rewrite the `qedgen-macros` line in a generated Cargo.toml from a git
/// dep tagged at the current crate version (which doesn't exist on GitHub
/// until release time) to a `path` dep pointing at the in-repo crate.
fn redirect_macros_to_path(cargo_toml: &std::path::Path) {
    let manifest = std::fs::read_to_string(cargo_toml).expect("read Cargo.toml");
    let macros_path = repo_root().join("crates/qedgen-macros");
    let replacement = format!("qedgen-macros = {{ path = {:?} }}", macros_path);
    let mut found = false;
    let rewritten: String = manifest
        .lines()
        .map(|line| {
            if line.starts_with("qedgen-macros = {")
                && line.contains("git = \"https://github.com/qedgen/solana-skills\"")
            {
                found = true;
                replacement.clone()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        found,
        "expected qedgen-macros git line in {}",
        cargo_toml.display()
    );
    std::fs::write(cargo_toml, format!("{rewritten}\n")).expect("rewrite Cargo.toml");
}

/// sBPF programs are verified by Lean proofs over the assembly; their
/// runtime behavior is exercised by client-side tests, not generated
/// Kani/proptest harnesses (which have no sBPF awareness and would emit
/// meaningless Anchor-shaped output). `codegen --kani` / `--proptest`
/// must skip harness emission for assembly targets and print a note.
/// Fast test — no cargo check, just inspects the generated tree.
fn assert_sbpf_skips(flag: &str) {
    let temp = tempfile::tempdir().expect("tempdir");
    let spec_src = repo_root().join("examples/sbpf/counter/counter.qedspec");
    let spec_path = temp.path().join("counter.qedspec");
    std::fs::copy(&spec_src, &spec_path).expect("copy counter spec");
    std::fs::create_dir(temp.path().join(".qed")).expect("create .qed");
    run(Command::new("git").arg("init").current_dir(temp.path()));

    let output_dir = temp.path().join("programs");
    let out = Command::new(env!("CARGO_BIN_EXE_qedgen"))
        .arg("codegen")
        .arg("--spec")
        .arg(&spec_path)
        .arg(flag)
        .arg("--output-dir")
        .arg(&output_dir)
        .current_dir(temp.path())
        .output()
        .expect("spawn codegen");
    assert!(
        out.status.success(),
        "codegen {flag} on sBPF spec should succeed, got {}\nstderr:\n{}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("skipping") && stderr.contains("sBPF"),
        "expected a skip note for {flag} on an sBPF spec, got stderr:\n{stderr}"
    );
    // No Kani / proptest harness file may be emitted anywhere in the tree.
    let banned = if flag == "--kani" {
        "kani.rs"
    } else {
        "proptest.rs"
    };
    let found: Vec<PathBuf> = walk_files(temp.path())
        .into_iter()
        .filter(|p| p.file_name().and_then(|n| n.to_str()) == Some(banned))
        .collect();
    assert!(
        found.is_empty(),
        "expected no {banned} for sBPF {flag}, found: {found:?}"
    );
}

/// Minimal recursive file walk (avoids a dev-dep on `walkdir`).
fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn sbpf_codegen_skips_kani_harness() {
    assert_sbpf_skips("--kani");
}

#[test]
fn sbpf_codegen_skips_proptest_harness() {
    assert_sbpf_skips("--proptest");
}

#[test]
#[ignore = "runs qedgen codegen and cargo check on a generated Anchor crate"]
fn escrow_anchor_scaffold_compiles() {
    smoke_anchor_scaffold("escrow");
}

#[test]
#[ignore = "runs qedgen codegen and cargo check on a generated Anchor crate"]
fn multisig_anchor_scaffold_compiles() {
    smoke_anchor_scaffold("multisig");
}

#[test]
#[ignore = "runs qedgen codegen and cargo check on a generated Anchor crate"]
fn percolator_anchor_scaffold_compiles() {
    smoke_anchor_scaffold("percolator");
}

#[test]
#[ignore = "runs qedgen codegen + cargo test --test proptest on a generated Anchor crate"]
fn escrow_anchor_proptest_runs() {
    smoke_anchor_scaffold_with_proptest("escrow");
}

#[test]
#[ignore = "runs qedgen codegen + cargo build on a generated Pinocchio crate"]
fn vault_pinocchio_scaffold_compiles() {
    smoke_pinocchio_scaffold("vault-greenfield", "vault.qedspec");
}

/// Issue #71 regression: a spec with a `Pubkey` state field (raw `[u8; 32]`,
/// no Pod wrapper) + an account-key effect/guard must `cargo build`. Pre-fix
/// this emitted `__state.<pubkey>.get()` (no such method) and `ctx.<acct>` in
/// the `self`-bound effect body — both compile errors that passed `check` +
/// proptest. Locking it into the compile gate keeps the Pubkey path covered.
#[test]
#[ignore = "runs qedgen codegen + cargo build on a generated Pinocchio crate"]
fn config_pubkey_pinocchio_scaffold_compiles() {
    smoke_pinocchio_scaffold("config-pubkey", "config.qedspec");
}
