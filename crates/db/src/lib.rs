//! The persistence boundary. Tenant operations scope every query to the named
//! organization. Invite mutations authorize from locked membership in their writing transaction.

mod audit;
mod connection;
mod context;
mod operations;

pub use connection::{ConnectError, connect};
pub use context::{InvalidTokenHash, TokenHash};
pub use operations::*;

#[cfg(test)]
extern crate self as db;
