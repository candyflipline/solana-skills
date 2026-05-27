//! M1 smoke test for `docs/design/quasar-cpi-spike.md` §11f.
//!
//! **Question this answers**: can `cargo kani` build + analyze a test
//! crate whose dependency tree includes a `#![no_std]` Pinocchio lib?
//!
//! **Answer**: yes, with two friction points captured below.
//!
//! ## Finding 1: `#![no_std]` libs need `extern crate kani`
//!
//! Without the `#[cfg(kani)] extern crate kani;` declaration below,
//! `cargo kani` fails with:
//!
//! > error: Failed to detect Kani functions.
//! >  = help: This project seems to be using #[no_std] but does not
//! >    import Kani. Try adding `crate extern kani` to the crate root
//! >    to explicitly import Kani.
//!
//! Implication for the brownfield Pinocchio Kani-impl emitter (slice
//! 8): the user's lib needs this one-line addition. We either (a) ask
//! them to add it, (b) inject it via codegen as a peer to `kani_impl.rs`,
//! or (c) require the harness to live in a separate crate that depends
//! on the user's lib. Decision lives in §11g of the design doc.
//!
//! ## Finding 2: Kani only scans the lib, not `tests/*.rs`
//!
//! The §11a assumption that the harness lives in the user's `tests/`
//! directory (relying on Cargo promoting integration tests to `std`
//! binaries) is wrong. `cargo kani`'s default harness discovery only
//! walks the lib's own modules; `tests/*.rs` is ignored entirely. With
//! the harness in `tests/`:
//!
//! > Manual Harness Summary:
//! > error: no harnesses matched the harness filter: `<name>`
//!
//! With the harness inside the lib (gated by `cfg(kani)`):
//!
//! > Check 1: ...assertion.1
//! > 	 - Status: SUCCESS
//! > SUMMARY: ** 0 of 1 failed
//! > VERIFICATION:- SUCCESSFUL
//!
//! Implication: the codegen emitter writes harnesses into the user's
//! `src/` (likely a new `src/kani_impl.rs` module, gated by
//! `#[cfg(kani)]`), not into `tests/`. This is a substantive deviation
//! from the Anchor `kani_impl.rs` shape today (which emits to `tests/`)
//! and from the prior design assumption. Captured in updated §11a.
//!
//! ## How to re-run this smoke
//!
//! ```sh
//! cd examples/pinocchio-fixtures/kani-smoke
//! cargo kani --harness smoke_kani_builds_against_no_std_pinocchio_lib
//! ```
//!
//! Expected: `VERIFICATION:- SUCCESSFUL` in under 1s.

#![no_std]

// Finding 1 — required for cargo kani against this #![no_std] lib.
#[cfg(kani)]
extern crate kani;

use pinocchio::{account_info::AccountInfo, ProgramResult};

/// Stub handler — exists only so the harness can take a function
/// pointer and force the linker to pull pinocchio in.
pub fn process_transfer(
    _accounts: &[AccountInfo],
    _data: &[u8],
) -> ProgramResult {
    Ok(())
}

// Finding 2 — harness must live inside the lib (not in tests/).
#[cfg(kani)]
mod kani_harness {
    #[kani::proof]
    fn smoke_kani_builds_against_no_std_pinocchio_lib() {
        // Forces linkage to pinocchio + reference to a real handler.
        let _handler: fn(
            &[pinocchio::account_info::AccountInfo],
            &[u8],
        ) -> pinocchio::ProgramResult = super::process_transfer;

        // Proves kani::any() + assume + assert work end-to-end.
        let amount: u64 = kani::any();
        kani::assume(amount > 0);
        assert!(amount > 0);
    }
}
