//! Equivalence check for SolidCraft's change to the vendored tessellator's point-in-trim test.
//!
//! The vendored crates are outside the workspace, so their own tests never run in CI. The old
//! test is kept behind the crate's `test-hooks` feature (on only for kernel tests) and this
//! checks the faster one gives the same answer everywhere it is probed.

use truck_meshalgo::tessellation::include_test_hooks;

/// Point-in-trim with run boxes: same inside/outside answer as before on more than 100k points.
#[test]
fn tessellation_include_same_as_before() {
    match include_test_hooks::include_answers_as_before(0..120) {
        Ok(n) => assert!(n > 100_000, "only {n} points checked"),
        Err(e) => panic!("{e}"),
    }
}
