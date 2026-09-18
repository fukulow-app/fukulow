#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Clippy's test options do not cover integration-test helper functions"
)]

use domain::{ActorId, OwnerChange, check_owner_change};
use uuid::Uuid;

#[test]
fn last_owner_cannot_be_demoted_suspended_or_leave() {
    let actor = ActorId(Uuid::now_v7());
    for change in [
        OwnerChange::Demote(actor),
        OwnerChange::Suspend(actor),
        OwnerChange::Leave(actor),
    ] {
        assert!(check_owner_change(&[actor], change).is_err());
    }
}

#[test]
fn another_active_owner_allows_the_change() {
    let actor = ActorId(Uuid::now_v7());
    let other = ActorId(Uuid::now_v7());
    for change in [
        OwnerChange::Demote(actor),
        OwnerChange::Suspend(actor),
        OwnerChange::Leave(actor),
    ] {
        assert!(check_owner_change(&[actor, other], change).is_ok());
        assert!(check_owner_change(&[other], change).is_ok());
        assert!(check_owner_change(&[], change).is_err());
    }
}
