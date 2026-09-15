//! Owns authentication, sessions, and invite tokens. It must never store or
//! log tokens in plaintext, so persistence and diagnostics cannot reveal them.
