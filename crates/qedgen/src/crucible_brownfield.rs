//! Brownfield Crucible harness emission — v2.21 Slice 1.
//!
//! Lifts the `qedgen probe --fuzz requires --spec` gate by synthesising
//! a minimal [`ParsedSpec`] from a brownfield project root (no
//! `.qedspec` required). The synthesised spec carries enough handler
//! metadata for [`crucible_gen::generate`] to emit a working harness
//! whose `invariant_test()` body is empty — Crucible's intrinsic
//! crash detector (panic / unwrap-on-None / BorrowMutError / arithmetic
//! overflow) does the lifting. See PRD-v2.21 §"Slice 1".
//!
//! ## Runtime coverage in v2.21
//!
//! - **Anchor / Quasar / qedgen-codegen** — handler enumeration via the
//!   regex used by `anchor_extractor::scan_handler_context_map`. Anchor
//!   IDL discovery hooks the same `target/idl/<prog>.json` lookup as
//!   spec-mode.
//! - **Pinocchio / Native / sBPF** — deferred (errors with a clear
//!   message). The site-catalogue path
//!   (`pinocchio_probe::scan_program`) gives us entry-point names but
//!   Crucible's existing `declare_fuzz_program!` macro expects an
//!   Anchor-shaped IDL. A separate `crucible_idl_gen` template for
//!   Pinocchio is a v2.22 item.

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use std::path::{Path, PathBuf};

use crate::check::{ParsedHandler, ParsedSpec};
use crate::probe::Runtime;

/// Synthesise a [`ParsedSpec`] from a brownfield project root. The
/// resulting spec has:
///
/// - `program_name` derived from `Cargo.toml`'s `[package] name`
///   (falling back to the root's leaf directory name).
/// - `handlers[]` populated from `pub fn <name>(ctx: Context<X>, ...)`
///   signatures scanned across the crate's `src/` tree. Handler params
///   are intentionally left empty in v2.21 — Crucible's IDL-derived
///   typed builders generate the param payload at fuzz time, and the
///   per-action stub gets `agent-fill` `todo!()` for the typed accounts
///   literal (same shape as spec-mode).
/// - No invariants, properties, account types, or PDAs — protocol mode
///   doesn't assert spec invariants. See
///   [`crate::crucible_gen::InvariantMode::Protocol`].
///
/// Errors when the runtime is not Anchor-family. Pinocchio / Native /
/// sBPF return an explanatory error pointing the user at v2.22.
pub fn synthesize_spec(project_root: &Path, runtime: Runtime) -> Result<ParsedSpec> {
    if !matches!(
        runtime,
        Runtime::Anchor | Runtime::Quasar | Runtime::QedgenCodegen
    ) {
        bail!(
            "Crucible brownfield mode (`--fuzz --root`) ships Anchor / Quasar / qedgen-codegen \
             in v2.21. Detected runtime: {runtime:?}. \
             Pinocchio / Native / sBPF brownfield support is tracked for v2.22+; \
             until then, fall back to `qedgen probe --program <path>` for the \
             site-catalogue audit envelope."
        );
    }

    let program_name = program_name_from_root(project_root)?;
    let handlers = scan_anchor_handlers(project_root)?;
    if handlers.is_empty() {
        bail!(
            "No `pub fn <name>(ctx: Context<X>, ...)` handlers found under {}. \
             Brownfield mode needs at least one Anchor handler to fuzz; \
             confirm `--root` points at the program crate (e.g. `programs/my_prog/`).",
            project_root.display()
        );
    }

    let mut spec = ParsedSpec {
        program_name,
        ..Default::default()
    };
    spec.handlers = handlers
        .into_iter()
        .map(|name| ParsedHandler {
            name,
            doc: None,
            who: None,
            on_account: None,
            pre_status: None,
            post_status: None,
            takes_params: vec![],
            guard_str: None,
            guard_str_rust: None,
            aborts_if: vec![],
            requires: vec![],
            ensures: vec![],
            modifies: None,
            let_bindings: vec![],
            aborts_total: false,
            permissionless: true,
            effects: vec![],
            accounts: vec![],
            transfers: vec![],
            emits: vec![],
            invariants: vec![],
            establishes: vec![],
            properties: vec![],
            calls: vec![],
            effect_branches: None,
        })
        .collect();
    Ok(spec)
}

/// Read `Cargo.toml`'s `[package] name`. Falls back to the root's
/// leaf-directory name (lowercased, hyphens kept) when `Cargo.toml` is
/// missing or unparseable — both happen on multi-program workspaces
/// where the user pointed `--root` at a workspace-level path. The
/// downstream caller surfaces a cleaner error if the program crate
/// can't be resolved at IDL-discovery time.
fn program_name_from_root(root: &Path) -> Result<String> {
    let manifest = root.join("Cargo.toml");
    if manifest.exists() {
        let raw = std::fs::read_to_string(&manifest)
            .with_context(|| format!("reading {}", manifest.display()))?;
        if let Some(name) = parse_package_name(&raw) {
            return Ok(name);
        }
    }
    root.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            anyhow!(
                "Could not determine program name from {} (no Cargo.toml, no leaf-directory name).",
                root.display()
            )
        })
}

/// Extract `name = "..."` from a `[package]` section. Hand-rolled
/// rather than pulling `toml` as a dep — qedgen already vends a regex
/// + manual parser for similar single-key reads (`anchor_resolver.rs`).
fn parse_package_name(toml_str: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml_str.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_package = trimmed.starts_with("[package");
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start_matches([' ', '\t']);
            let rest = rest.strip_prefix('=')?;
            let rest = rest.trim().trim_matches(['"', '\''].as_ref());
            if !rest.is_empty() {
                return Some(rest.to_string());
            }
        }
    }
    None
}

/// Walk `<root>/src/**/*.rs` and collect handler names from
/// `pub fn <name>(ctx: Context<X>, ...)` signatures. De-dupes by name
/// (Anchor sometimes splits handlers across module re-exports).
fn scan_anchor_handlers(root: &Path) -> Result<Vec<String>> {
    let src_dir = root.join("src");
    if !src_dir.exists() {
        bail!(
            "Brownfield root {} has no `src/` — confirm `--root` points at a Rust crate.",
            root.display()
        );
    }
    let pat =
        Regex::new(r"(?m)^\s*pub\s+fn\s+(\w+)\s*\(\s*(?:mut\s+)?ctx\s*:\s*Context\s*<\s*\w+\s*>")
            .expect("static regex");
    let mut handlers: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for file in collect_rust_files(&src_dir)? {
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        for caps in pat.captures_iter(&src) {
            let name = caps.get(1).unwrap().as_str().to_string();
            if seen.insert(name.clone()) {
                handlers.push(name);
            }
        }
    }
    Ok(handlers)
}

fn collect_rust_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().and_then(|s| s.to_str()) == Some("target") {
                    continue;
                }
                walk(&path, out)?;
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    walk(dir, &mut out)?;
    out.sort();
    Ok(out)
}

/// Default harness location for brownfield mode: `<root>/.qed/fuzz/`.
/// `crucible_gen::generate` appends the program-name leaf, so this
/// returns the parent directory (matching the spec-mode convention).
pub fn brownfield_harness_parent(root: &Path) -> PathBuf {
    root.join(".qed").join("fuzz")
}

/// Best-effort project-root discovery from `--root`: if the user
/// pointed at a Cargo workspace (with `programs/<prog>/`), walk down
/// to the first `pub mod ... declare_id!` crate. v2.21 returns the
/// input unchanged — workspace traversal is a v2.22 polish. We keep
/// the function defined so the caller can swap in a smarter walker
/// without a CLI shape change.
pub fn resolve_program_root(input: &Path) -> Result<PathBuf> {
    Ok(input.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(dir: &Path, body: &str) {
        std::fs::write(dir.join("Cargo.toml"), body).unwrap();
    }

    #[test]
    fn parses_simple_package_name() {
        let toml = r#"
[package]
name = "my_program"
version = "0.1.0"
"#;
        assert_eq!(parse_package_name(toml).as_deref(), Some("my_program"));
    }

    #[test]
    fn parses_with_dependencies_section_after_package() {
        let toml = r#"
[package]
name = "buggy_anchor"
version = "0.1.0"

[dependencies]
anchor-lang = "0.30"
"#;
        assert_eq!(parse_package_name(toml).as_deref(), Some("buggy_anchor"));
    }

    #[test]
    fn ignores_name_outside_package_section() {
        let toml = r#"
[lib]
name = "shouldnt_match"

[package]
name = "real_name"
"#;
        assert_eq!(parse_package_name(toml).as_deref(), Some("real_name"));
    }

    #[test]
    fn returns_none_for_missing_package_block() {
        let toml = r#"
[workspace]
members = ["programs/*"]
"#;
        assert_eq!(parse_package_name(toml), None);
    }

    #[test]
    fn program_name_falls_back_to_leaf_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let crate_dir = tmp.path().join("standalone_prog");
        std::fs::create_dir_all(&crate_dir).unwrap();
        // No Cargo.toml — fallback path.
        let name = program_name_from_root(&crate_dir).unwrap();
        assert_eq!(name, "standalone_prog");
    }

    #[test]
    fn scan_anchor_handlers_collects_unique_names() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            r#"
#[program]
pub mod my_prog {
    use super::*;

    pub fn initialize(ctx: Context<Init>) -> Result<()> { Ok(()) }
    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> { Ok(()) }
    pub fn withdraw(mut ctx: Context<Withdraw>, amount: u64) -> Result<()> { Ok(()) }
}
"#,
        )
        .unwrap();
        let handlers = scan_anchor_handlers(tmp.path()).unwrap();
        assert_eq!(handlers, vec!["initialize", "deposit", "withdraw"]);
    }

    #[test]
    fn scan_anchor_handlers_dedupes_re_exports() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(src.join("handlers")).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "pub fn deposit(ctx: Context<Deposit>, amt: u64) -> Result<()> { Ok(()) }\n",
        )
        .unwrap();
        std::fs::write(
            src.join("handlers").join("deposit.rs"),
            "pub fn deposit(ctx: Context<Deposit>, amt: u64) -> Result<()> { Ok(()) }\n",
        )
        .unwrap();
        let handlers = scan_anchor_handlers(tmp.path()).unwrap();
        assert_eq!(handlers, vec!["deposit"]);
    }

    #[test]
    fn scan_errors_when_src_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let err = scan_anchor_handlers(tmp.path()).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("no `src/`"), "got: {msg}");
    }

    #[test]
    fn synthesize_spec_rejects_pinocchio() {
        let tmp = tempfile::tempdir().unwrap();
        let err = synthesize_spec(tmp.path(), Runtime::Pinocchio).unwrap_err();
        assert!(format!("{err:#}").contains("Pinocchio"));
    }

    #[test]
    fn synthesize_spec_builds_handler_list_for_anchor() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            r#"
[package]
name = "buggy_anchor"
version = "0.1.0"
"#,
        );
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "pub fn run(ctx: Context<Run>) -> Result<()> { Ok(()) }\n",
        )
        .unwrap();
        let spec = synthesize_spec(tmp.path(), Runtime::Anchor).unwrap();
        assert_eq!(spec.program_name, "buggy_anchor");
        assert_eq!(spec.handlers.len(), 1);
        assert_eq!(spec.handlers[0].name, "run");
        // Brownfield handlers are `permissionless` — no `auth` to lift.
        assert!(spec.handlers[0].permissionless);
        assert!(spec.invariants.is_empty());
        assert!(spec.properties.is_empty());
    }

    #[test]
    fn synthesize_errors_when_no_handlers_found() {
        let tmp = tempfile::tempdir().unwrap();
        write_manifest(
            tmp.path(),
            r#"
[package]
name = "empty"
version = "0.1.0"
"#,
        );
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "// no anchor handlers\n").unwrap();
        let err = synthesize_spec(tmp.path(), Runtime::Anchor).unwrap_err();
        assert!(format!("{err:#}").contains("No `pub fn"));
    }

    #[test]
    fn brownfield_harness_parent_is_qed_fuzz() {
        let root = Path::new("/workspace/my_prog");
        assert_eq!(
            brownfield_harness_parent(root),
            PathBuf::from("/workspace/my_prog/.qed/fuzz")
        );
    }
}
