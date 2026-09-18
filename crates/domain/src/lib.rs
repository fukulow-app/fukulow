//! Identities, conversation types, membership rules and the canonical name of each enumerated
//! value. It must not choose a serialization format or depend on serde.

use uuid::Uuid;

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub Uuid);
    };
}
identifier!(ActorId);
identifier!(OrganizationId);
identifier!(TeamId);

macro_rules! enumeration {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)+
                    _ => None,
                }
            }

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

identifier!(ChannelId);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("message id must be UUIDv7")]
pub struct NotV7;

impl MessageId {
    pub fn from_client(value: Uuid) -> Result<Self, NotV7> {
        if value.get_version() == Some(uuid::Version::SortRand)
            && value.get_variant() == uuid::Variant::RFC4122
        {
            Ok(Self(value))
        } else {
            Err(NotV7)
        }
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl TryFrom<Uuid> for MessageId {
    type Error = NotV7;
    fn try_from(value: Uuid) -> Result<Self, Self::Error> {
        Self::from_client(value)
    }
}

impl From<MessageId> for Uuid {
    fn from(value: MessageId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelScope {
    Organization,
    Team(TeamId),
}

impl ChannelScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Organization => "organization",
            Self::Team(_) => "team",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChannelSeq(i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("message sequence must be positive")]
pub struct InvalidChannelSeq;

impl ChannelSeq {
    /// The exclusive cursor before the first message; never a stored message sequence.
    pub const BEGINNING: Self = Self(0);

    pub fn new(value: i64) -> Result<Self, InvalidChannelSeq> {
        if value >= 1 {
            Ok(Self(value))
        } else {
            Err(InvalidChannelSeq)
        }
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

impl TryFrom<i64> for ChannelSeq {
    type Error = InvalidChannelSeq;
    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ChannelSeq> for i64 {
    fn from(value: ChannelSeq) -> Self {
        value.0
    }
}
