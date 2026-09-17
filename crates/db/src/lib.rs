//! The persistence boundary. Tenant operations scope every query to the named
//! organization; callers supply authorization before entering this crate.

mod audit;
mod connection;
mod context;
mod operations;

pub use connection::{ConnectError, connect};
pub use context::{InvalidTokenHash, TokenHash};
pub use operations::*;

#[cfg(test)]
extern crate self as db;
