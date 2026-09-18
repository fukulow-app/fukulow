//! The persistence boundary. Tenant operations scope every query to the named
//! organization, with two exceptions that discover it first: leaving the service reads the
//! actor's own memberships, and invite acceptance finds its invite by token hash. Every
//! write after either is scoped by the organization it found. A state-changing operation
//! decides its capability from the membership row it locks, in the transaction that writes.

mod audit;
mod connection;
mod context;
mod operations;

pub use connection::{ConnectError, connect};
pub use context::{InvalidTokenHash, TokenHash};
pub use operations::*;

#[cfg(test)]
extern crate self as db;
