//! Owns subscriptions and fan-out. Its public API must not expose `tokio`
//! channel types or `RecvError::Lagged`, so delivery contracts remain
//! independent of the internal channel implementation.
