//! The persistence boundary. Tenant operations scope every query to the named
//! organization; callers supply authorization before entering this crate.

mod audit;
mod connection;
mod operations;

pub use connection::{ConnectError, connect};
pub use operations::*;
