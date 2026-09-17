macro_rules! operation_error {
    ($name:ident { $($variant:ident => $message:literal),* $(,)? }) => {
        #[derive(thiserror::Error)]
        pub enum $name {
            $(#[error($message)] $variant,)*
            // PostgreSQL details can contain a rejected email or password hash.
            #[error("database operation failed")]
            Database(sqlx::Error),
        }
        impl From<sqlx::Error> for $name {
            fn from(error: sqlx::Error) -> Self { Self::Database(error) }
        }
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(self, f)
            }
        }
    };
}
operation_error!(CreatePersonError { EmailTaken => "email is already registered" });
operation_error!(CreateOrganizationError {
    InvalidSlug => "invalid organization slug", SlugTaken => "organization slug is already used",
    ActorNotFound => "actor not found", ActorDeparted => "actor has departed"
});
operation_error!(ChangeMemberRoleError { NotFound => "member not found", LastOwner => "last active owner" });
operation_error!(SuspendMemberError {
    NotFound => "member not found", NotActive => "member is not active", LastOwner => "last active owner"
});
operation_error!(ReactivateMemberError {
    NotFound => "member not found", NotSuspended => "member is not suspended", ActorDeparted => "actor has departed"
});
operation_error!(CreateTeamError { NotFound => "organization not found" });
operation_error!(AddTeamMemberError {
    NotFound => "team or member not found", NotAnActiveMember => "organization member is not active",
    AlreadyMember => "already a team member", ActorDeparted => "actor has departed"
});
operation_error!(ChangeTeamMemberRoleError { NotFound => "team member not found" });
operation_error!(RemoveTeamMemberError { NotFound => "team member not found" });

#[derive(thiserror::Error)]
pub enum LeaveServiceError {
    #[error("actor not found")]
    NotFound,
    #[error("last active owner of organization {organization_id:?}")]
    LastOwner {
        organization_id: domain::OrganizationId,
    },
    #[error("database operation failed")]
    Database(sqlx::Error),
}
impl From<sqlx::Error> for LeaveServiceError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}
impl std::fmt::Debug for LeaveServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl From<domain::LastOwner> for ChangeMemberRoleError {
    fn from(_: domain::LastOwner) -> Self {
        Self::LastOwner
    }
}
impl From<domain::LastOwner> for SuspendMemberError {
    fn from(_: domain::LastOwner) -> Self {
        Self::LastOwner
    }
}

pub(super) fn constraint(error: &sqlx::Error, name: &str) -> bool {
    error
        .as_database_error()
        .is_some_and(|error| error.constraint() == Some(name))
}
