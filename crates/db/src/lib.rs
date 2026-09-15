//! Owns persistence and is the only crate allowed to write SQL. Callers must
//! not bypass it: every organization-scoped read must go through this crate
//! and filter by `organization_id`, so tenant isolation has one boundary.
