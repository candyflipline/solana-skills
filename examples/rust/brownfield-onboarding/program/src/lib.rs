//! Brownfield onboarding demo — FIXED variant.
//!
//! The shipped source below passes the spec at
//! `../onboarding.qedspec`. To reproduce the audit→spec→verify→fix
//! cycle, follow the README walkthrough's bug-injection diff (line
//! [BUG] below) to flip `bump` into the broken form. Re-run
//! `qedgen verify --proptest` to see `counter_monotonic` fire red,
//! then revert.

#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct State {
    pub counter: u64,
    pub paused: u8,
}

impl Default for State {
    fn default() -> Self {
        Self {
            counter: 0,
            paused: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Paused,
    Overflow,
}

/// `bump (delta)` — advance the counter by `delta` unless paused.
///
/// [BUG] To inject the audit's original bug, replace `checked_add`
/// with `wrapping_sub`. The counter advances *downward*, violating
/// the `counter_monotonic` property; the proptest harness fires red
/// with a counterexample like `(pre.counter = 0, delta = 1) →
/// post.counter = u64::MAX`.
pub fn bump(s: &mut State, delta: u64) -> Result<(), Error> {
    if s.paused == 1 {
        return Err(Error::Paused);
    }
    s.counter = s.counter.checked_add(delta).ok_or(Error::Overflow)?;
    Ok(())
}

/// `pause` — flip the gate so subsequent `bump` calls reject.
pub fn pause(s: &mut State) -> Result<(), Error> {
    s.paused = 1;
    Ok(())
}

/// `unpause` — reopen the gate.
pub fn unpause(s: &mut State) -> Result<(), Error> {
    s.paused = 0;
    Ok(())
}

#[cfg(test)]
mod sanity_tests {
    use super::*;

    #[test]
    fn bump_advances_when_unpaused() {
        let mut s = State::default();
        bump(&mut s, 5).unwrap();
        assert_eq!(s.counter, 5);
    }

    #[test]
    fn bump_rejects_when_paused() {
        let mut s = State::default();
        pause(&mut s).unwrap();
        assert_eq!(bump(&mut s, 5), Err(Error::Paused));
        assert_eq!(s.counter, 0);
    }

    #[test]
    fn bump_overflow_returns_err() {
        let mut s = State {
            counter: u64::MAX - 1,
            paused: 0,
        };
        assert_eq!(bump(&mut s, 2), Err(Error::Overflow));
        assert_eq!(s.counter, u64::MAX - 1);
    }

    #[test]
    fn pause_unpause_roundtrip() {
        let mut s = State::default();
        pause(&mut s).unwrap();
        assert_eq!(s.paused, 1);
        unpause(&mut s).unwrap();
        assert_eq!(s.paused, 0);
    }
}
