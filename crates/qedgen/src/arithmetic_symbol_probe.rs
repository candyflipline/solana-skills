//! Arithmetic-symbol catalog probes — v2.22 Slice 1.
//!
//! Runtime-agnostic source scanners that fire on arithmetic operators
//! whose *symbol* (not result) is the bug. Closes the subscriptions
//! bench's two firm-High misses (CAN-H1 `saturating_sub`, CAN-H3
//! `checked_sub`) — both arithmetic operators that are correct in
//! isolation; the bug is the failure-mode interaction with the
//! surrounding control flow.
//!
//! ## Rules in this module
//!
//! - **`silent_success_arithmetic`** (S1.1, HIGH) — `saturating_sub` /
//!   `saturating_add` on a timestamp-shape receiver whose result gates
//!   a non-trivial effect. The 0-or-MAX boundary value silently opens
//!   a fund-flow gate that should have stayed closed.
//!
//! - **`graceful_error_as_dos`** (S1.2, HIGH; v2.22 follow-up) —
//!   `checked_sub` / `checked_add` / `checked_mul` in an init / create
//!   path where `Err` propagation permanently bricks a deterministic
//!   PDA. Pending.
//!
//! - **`unchecked_arith_with_fund_flow`** (S1.3, INFO; v2.22
//!   follow-up) — unchecked `*` / `+` / `-` whose result gates a CPI.
//!   Pending.
//!
//! ## Why source-scan, not spec
//!
//! These bugs fire on the deployed Rust source, not on `.qedspec`
//! state. Mirrors `pinocchio_probe::scan_program`'s shape — walk
//! `*.rs` under the project root, regex-match the pattern, emit
//! `Finding`s into the same envelope as the spec-aware probes.
//!
//! ## Reproducers
//!
//! Each finding ships a `Reproducer::MolluskPrompt` (same template
//! mechanism the Pinocchio probes use) pointing at a per-rule markdown
//! under `references/probes/arithmetic_symbol/<rule>.md`. The agent
//! fills the litesvm test body using the cited handler, account,
//! and timestamp source. Time-to-fired-repro target: ≤ 20 min per
//! finding.

use anyhow::Result;
use regex::Regex;
use std::path::{Path, PathBuf};

use crate::probe::{Category, Finding, Reproducer, Severity};

/// Entry point: walk `<root>/src/**/*.rs` and emit findings. Returns
/// an empty vec when no matches surface (no errors — the absence of a
/// match is informational, not a failure).
pub fn scan_program(project_root: &Path) -> Result<Vec<Finding>> {
    let src_dir = project_root.join("src");
    if !src_dir.exists() {
        // No `src/` — not a Rust crate root. Defer to the caller's
        // bootstrap envelope; this probe has nothing to say.
        return Ok(Vec::new());
    }
    let rs_files = collect_rust_files(&src_dir)?;
    let mut findings = Vec::new();
    for file in &rs_files {
        let Ok(source) = std::fs::read_to_string(file) else {
            continue;
        };
        let rel = file
            .strip_prefix(project_root)
            .unwrap_or(file)
            .to_path_buf();
        findings.extend(scan_silent_success_arithmetic(&rel, &source));
    }
    Ok(findings)
}

/// S1.1 — `silent_success_arithmetic` scanner. Pattern criteria:
///
/// 1. A call site of `saturating_sub` or `saturating_add`.
/// 2. The receiver matches a timestamp-shape pattern (`current_ts`,
///    `now`, `Clock::get()?.unix_timestamp`, `clock.slot`, `slot`,
///    `epoch`, `block_height`, or an identifier ending in `_ts` / `_secs`).
/// 3. The site is inside a function whose body, in the lines AFTER
///    the call, contains a comparison (`>=`, `>`) gating an
///    `if`/`else` branch — the canonical "elapsed time opens a gate"
///    shape.
///
/// False-positive guard: the receiver-type check rejects calls on
/// counter / amount values that happen to use `saturating_sub` for
/// fee accounting. Only timestamp-shape receivers fire.
///
/// For v2.22 first ship the scanner emits one finding per call site
/// (not per gated branch). Iteration on bench feedback may tighten
/// the gate-detection step in v2.22.x.
pub(crate) fn scan_silent_success_arithmetic(rel_file: &Path, source: &str) -> Vec<Finding> {
    // Receiver pattern: identifier or `Clock::get()?.unix_timestamp` /
    // `*deref` of one of those. We accept up to ~64 chars of receiver
    // to keep the regex tractable; longer expressions (chained
    // method calls) won't match, which is a deliberate
    // false-negative.
    //
    // The body group `(?P<recv>...)` captures the receiver text so we
    // can re-check the timestamp-shape predicate at filter time.
    let call_re = Regex::new(
        r"(?m)\b(?P<recv>\*?[\w\.\?\(\)\:]{1,64})\.(?P<op>saturating_sub|saturating_add)\s*\(",
    )
    .expect("static regex compiles");

    let mut out = Vec::new();
    for caps in call_re.captures_iter(source) {
        let m = caps.get(0).unwrap();
        let recv = caps.name("recv").unwrap().as_str();
        if !is_timestamp_shape(recv) {
            continue;
        }
        let line = byte_offset_to_line(source, m.start());
        let fn_name = enclosing_fn_name(source, m.start());
        // Look at the next ~400 chars of source for the gating
        // comparison shape. This is the "elapsed >= threshold opens
        // a non-trivial effect" tell.
        let window = &source[m.end()..source.len().min(m.end() + 400)];
        if !window_has_gating_comparison(window) {
            continue;
        }

        let finding_id = make_id(rel_file, line, "silent_success_arithmetic");
        let mut subs = std::collections::BTreeMap::new();
        subs.insert("FILE".to_string(), rel_file.display().to_string());
        subs.insert("LINE".to_string(), line.to_string());
        subs.insert("RECEIVER".to_string(), recv.to_string());
        subs.insert(
            "OPERATOR".to_string(),
            caps.name("op").unwrap().as_str().to_string(),
        );
        subs.insert(
            "FN".to_string(),
            fn_name.clone().unwrap_or_else(|| "<unknown>".into()),
        );

        out.push(Finding {
            id: finding_id.clone(),
            category: Category::SilentSuccessArithmetic,
            severity: Severity::High,
            handler: fn_name.unwrap_or_else(|| "<unknown>".into()),
            spec_silent_on: format!(
                "`{}.{}(...)` at {}:{} returns the boundary value (0 / MAX) \
                 when the conceptual operation underflows. The downstream \
                 `>=` comparison fires for both 'no time elapsed' and 'an \
                 undefined amount of negative time elapsed' — collapsing \
                 two semantically distinct states.",
                recv,
                caps.name("op").unwrap().as_str(),
                rel_file.display(),
                line
            ),
            suppression_hint: "Replace `saturating_*` with an explicit underflow check: \
                 `if current_ts < start_ts { return Err(...) }`. The early \
                 return makes the 'time hasn't elapsed' branch distinguishable \
                 from the 'time has elapsed' branch."
                .to_string(),
            investigation_hint: format!(
                "Trace the gated effect downstream of the comparison at \
                 {}:{}. Confirm whether the boundary value (0 / MAX) is \
                 ever a valid input or always a bug-condition signal. \
                 If the gate touches funds (transfer / mint / state \
                 advance), this is a fund-flow leak.",
                rel_file.display(),
                line
            ),
            category_tag: "silent_success_arithmetic".to_string(),
            reproducer: Some(Reproducer::MolluskPrompt {
                template_path:
                    "references/probes/arithmetic_symbol/silent_success_arithmetic.md#reproducer"
                        .to_string(),
                substitutions: subs,
                repro_path: format!(".qed/probes/arithmetic_symbol/{}/repro.rs", finding_id),
            }),
        });
    }
    out
}

/// Receiver-shape predicate. Returns true for identifiers and
/// expressions that look like a Solana timestamp / clock value.
fn is_timestamp_shape(recv: &str) -> bool {
    let r = recv.trim();
    let r = r.strip_prefix('*').unwrap_or(r);
    // Common identifier patterns.
    let known = [
        "current_ts",
        "current_time",
        "now",
        "now_ts",
        "ts",
        "clock_ts",
        "unix_timestamp",
        "slot",
        "epoch",
        "block_height",
        "current_slot",
        "current_epoch",
    ];
    if known.contains(&r) {
        return true;
    }
    // Suffix shapes: `_ts`, `_secs`, `_at`, `_time`, `_timestamp`.
    let id = r
        .rsplit('.')
        .next()
        .unwrap_or(r)
        .trim_start_matches('*')
        .trim_end_matches('?');
    if id.ends_with("_ts")
        || id.ends_with("_secs")
        || id.ends_with("_seconds")
        || id.ends_with("_time")
        || id.ends_with("_timestamp")
        || id.ends_with("_slot")
        || id.ends_with("_epoch")
    {
        return true;
    }
    // Clock accessor patterns.
    if r.contains("Clock::get()") && r.contains("unix_timestamp") {
        return true;
    }
    if r.contains(".unix_timestamp")
        || r.contains(".slot")
        || r.contains(".epoch")
        || r.contains(".block_height")
    {
        return true;
    }
    false
}

/// Window-after-call check: does the next chunk of source contain a
/// comparison shape we associate with "elapsed time opens a gate"?
///
/// Conservative: looks for `>=` or `>` followed (within a short
/// window) by `{` (block body) or `return` (early return inverted to
/// guard). Misses sophisticated re-binding patterns; future bench
/// evidence tightens this.
fn window_has_gating_comparison(window: &str) -> bool {
    let cmp = Regex::new(r">=|>").expect("static regex");
    if let Some(m) = cmp.find(window) {
        let after = &window[m.end()..window.len().min(m.end() + 120)];
        return after.contains('{')
            || after.contains("return ")
            || after.contains("Ok(")
            || after.contains("Err(");
    }
    false
}

/// Resolve the byte offset to a 1-indexed line number.
fn byte_offset_to_line(source: &str, offset: usize) -> u32 {
    let prefix = &source[..offset.min(source.len())];
    1 + prefix.chars().filter(|c| *c == '\n').count() as u32
}

/// Walk backward from the given offset to find the nearest enclosing
/// `fn <name>(`. Returns the function name; falls back to `None` when
/// the offset isn't inside a function.
fn enclosing_fn_name(source: &str, offset: usize) -> Option<String> {
    let head = &source[..offset.min(source.len())];
    let re = Regex::new(r"fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(").expect("static regex");
    re.captures_iter(head).last().map(|c| c[1].to_string())
}

/// Stable id: hash of (file, line, rule). Matches the Pinocchio probe
/// shape so suppression files are uniform across rule families.
fn make_id(rel_file: &Path, line: u32, rule: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(rel_file.display().to_string().as_bytes());
    h.update(b":");
    h.update(line.to_string().as_bytes());
    h.update(b":");
    h.update(rule.as_bytes());
    let id = format!("{:x}", h.finalize());
    id[..16].to_string()
}

fn collect_rust_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    walk(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| matches!(n, "target" | ".git" | "node_modules" | "tests" | ".qed"))
        {
            continue;
        }
        if path.is_dir() {
            walk(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn fires_on_canonical_subscriptions_can_h1_shape() {
        // Mirrors transfer_validation.rs:61 from the subscriptions
        // bench — the CAN-H1 firm-High miss.
        let src = r#"
fn process_transfer(ctx: Context, current_ts: i64) -> Result<()> {
    let time_since_start = current_ts.saturating_sub(*current_period_start_ts);
    if time_since_start >= period_length {
        // period advancement — gives the merchant fresh budget
        advance_period(ctx)?;
    }
    Ok(())
}
"#;
        let findings = scan_silent_success_arithmetic(&p("transfer.rs"), src);
        assert_eq!(findings.len(), 1, "expected 1 finding, got {findings:#?}");
        let f = &findings[0];
        assert_eq!(f.category_tag, "silent_success_arithmetic");
        assert!(matches!(f.severity, Severity::High));
        assert_eq!(f.handler, "process_transfer");
        assert!(matches!(
            f.reproducer,
            Some(Reproducer::MolluskPrompt { .. })
        ));
    }

    #[test]
    fn ignores_saturating_sub_on_non_timestamp_receiver() {
        // Counter difference is a legitimate use of saturating_sub and
        // shouldn't flag.
        let src = r#"
fn deduct_fee(balance: u64, fee: u64) -> u64 {
    if balance.saturating_sub(fee) > 0 {
        balance - fee
    } else {
        0
    }
}
"#;
        let findings = scan_silent_success_arithmetic(&p("fees.rs"), src);
        assert!(
            findings.is_empty(),
            "balance/fee receiver should NOT fire, got {findings:#?}"
        );
    }

    #[test]
    fn fires_on_clock_get_unix_timestamp_receiver() {
        let src = r#"
fn check_expiry(ctx: Context) -> Result<()> {
    let elapsed = Clock::get()?.unix_timestamp.saturating_sub(start_ts);
    if elapsed >= duration {
        return Err(Expired.into());
    }
    Ok(())
}
"#;
        let findings = scan_silent_success_arithmetic(&p("expiry.rs"), src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn fires_on_suffix_named_timestamp() {
        let src = r#"
fn advance(state: &mut State, last_seen_ts: i64) -> Result<()> {
    let delta = last_seen_ts.saturating_sub(state.previous_ts);
    if delta >= MIN_INTERVAL {
        state.advance();
    }
    Ok(())
}
"#;
        let findings = scan_silent_success_arithmetic(&p("advance.rs"), src);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn does_not_fire_without_gating_comparison() {
        // Saturating_sub on timestamp but the result is logged, not
        // gated — no fund-flow leak shape.
        let src = r#"
fn log_elapsed(current_ts: i64, start_ts: i64) {
    let elapsed = current_ts.saturating_sub(start_ts);
    msg!("elapsed: {}", elapsed);
}
"#;
        let findings = scan_silent_success_arithmetic(&p("log.rs"), src);
        assert!(
            findings.is_empty(),
            "no gating comparison should NOT fire; got {findings:#?}"
        );
    }

    #[test]
    fn timestamp_shape_predicate_recognises_common_idents() {
        assert!(is_timestamp_shape("current_ts"));
        assert!(is_timestamp_shape("now"));
        assert!(is_timestamp_shape("start_ts"));
        assert!(is_timestamp_shape("clock.slot"));
        assert!(is_timestamp_shape("Clock::get()?.unix_timestamp"));
        assert!(is_timestamp_shape("*current_period_start_ts"));
        assert!(!is_timestamp_shape("balance"));
        assert!(!is_timestamp_shape("amount"));
        assert!(!is_timestamp_shape("fee_lamports"));
    }
}
