use domain::{Capability, OrganizationRole};

#[test]
fn only_owner_and_admin_hold_invite_member() {
    assert!(OrganizationRole::Owner.holds(Capability::InviteMember));
    assert!(OrganizationRole::Admin.holds(Capability::InviteMember));
    assert!(!OrganizationRole::Member.holds(Capability::InviteMember));
}
