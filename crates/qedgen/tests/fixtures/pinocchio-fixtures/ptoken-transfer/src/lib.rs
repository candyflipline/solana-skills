#![no_std]

// Per the M1 smoke test finding: `cargo kani` against a `#![no_std]`
// lib requires an explicit `extern crate kani` so the verifier can
// find its instrumentation entry. Gated by `cfg(kani)` so production
// builds (and non-Kani tests) don't pull in the kani dep.
#[cfg(kani)]
extern crate kani;

pub mod transfer;

#[cfg(kani)]
pub mod kani_impl;
