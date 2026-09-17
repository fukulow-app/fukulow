//! Identities and membership rules. It must not know how data is stored or
//! transmitted, so the rules stay independent of persistence and transport.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! identifier {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);
    };
}
identifier!(ActorId);
identifier!(OrganizationId);
identifier!(TeamId);

macro_rules! enumeration {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }
        }
    };
}
enumeration!(OrganizationRole { Owner => "owner", Admin => "admin", Member => "member" });
enumeration!(MemberStatus { Active => "active", Suspended => "suspended", Left => "left" });
enumeration!(TeamRole { Manager => "manager", Member => "member" });

#[derive(Debug, Clone, Copy)]
pub enum OwnerChange {
    Demote(ActorId),
    Suspend(ActorId),
    Leave(ActorId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the last active owner cannot be removed")]
pub struct LastOwner;

pub fn check_owner_change(active_owners: &[ActorId], change: OwnerChange) -> Result<(), LastOwner> {
    let (OwnerChange::Demote(actor) | OwnerChange::Suspend(actor) | OwnerChange::Leave(actor)) =
        change;
    if active_owners.iter().any(|owner| *owner != actor) {
        Ok(())
    } else {
        Err(LastOwner)
    }
}
