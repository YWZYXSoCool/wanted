//! Pure progress state machine tests: progress really advances, the total is
//! adopted, and phases reset to start.

use crate::report::{Progress, ProgressState};

const H_QUARTER: u64 = 1_000_000;
const HALF: u64 = 2_000_000;
const THREE_QUARTER: u64 = 3_000_000;
const TOTAL: u64 = 4_000_000;

fn bytes(done: u64, total: Option<u64>) -> Progress {
    Progress::Bytes { done, total }
}

#[test]
fn bytes_advances_position_monotonically() {
    let mut state = ProgressState::default();
    state = state.update(&bytes(H_QUARTER, Some(TOTAL)));
    state = state.update(&bytes(HALF, Some(TOTAL)));
    state = state.update(&bytes(THREE_QUARTER, Some(TOTAL)));
    state = state.update(&bytes(TOTAL, Some(TOTAL)));

    assert_eq!(
        state,
        ProgressState {
            total: Some(TOTAL),
            position: TOTAL
        }
    );
}

#[test]
fn total_is_adopted_once_and_not_regressed() {
    let mut state = ProgressState::default();
    state = state.update(&bytes(H_QUARTER, None));
    assert_eq!(state.total, None);

    state = state.update(&bytes(HALF, Some(TOTAL)));
    assert_eq!(state.total, Some(TOTAL));

    state = state.update(&bytes(THREE_QUARTER, None));
    assert_eq!(state.total, Some(TOTAL));
    assert_eq!(state.position, THREE_QUARTER);
}

#[test]
fn position_never_regresses_on_reentrant_reports() {
    let mut state = ProgressState::default();
    state = state.update(&bytes(THREE_QUARTER, Some(TOTAL)));
    state = state.update(&bytes(H_QUARTER, Some(TOTAL)));

    assert_eq!(state.position, THREE_QUARTER);
}

#[test]
fn phase_resets_state_to_start_fresh() {
    let state = ProgressState::default().update(&bytes(TOTAL, Some(TOTAL)));
    let state = state.update(&Progress::Phase("Extracting"));

    assert_eq!(state, ProgressState::default());
}
