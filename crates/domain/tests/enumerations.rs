#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Clippy's test options do not cover integration-test helper functions"
)]

use domain::{ActorType, InviteKind, MemberStatus, OrganizationRole, TeamRole};

macro_rules! round_trip {
    ($test:ident, $enumeration:ident) => {
        #[test]
        fn $test() {
            for &value in $enumeration::ALL {
                assert_eq!($enumeration::parse(value.as_str()), Some(value));
            }
            for unknown in ["", "unknown", "OWNER", " active"] {
                assert_eq!($enumeration::parse(unknown), None);
            }
        }
    };
}

round_trip!(organization_roles_round_trip, OrganizationRole);
round_trip!(member_statuses_round_trip, MemberStatus);
round_trip!(team_roles_round_trip, TeamRole);

round_trip!(actor_types_round_trip, ActorType);

round_trip!(invite_kinds_round_trip, InviteKind);
