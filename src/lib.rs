//! nns-vesl — NNS hull (Nockchain Name Service): read-only scanner over
//! `.nock` registrations.
//!
//! The kernel in `hoon/app/app.hoon` holds **`v0-state`**: an
//! **`nns-accumulator`**, chain-scan cursor
//! (`last-proved-height`, `last-proved-digest`), Vesl graft fragment, and
//! STARK helper state. Users register names by submitting **`nns/v1/claim`**
//! notes on Nockchain; the follower walks blocks and pokes **`%scan-block`**
//! so valid claims fold into the accumulator in canonical chain order.
//!
//! The Rust hull boots the kernel, runs [`chain_follower`](crate::chain_follower),
//! and exposes **`GET /health`**, **`GET /status`**, **`GET /accumulator/:name`**
//! only — see [`api`](crate::api). Offline verification is **`light_verify`**
//! (`docs/wallet-verification.md`).

pub mod api;
pub mod chain;
pub mod chain_follower;
pub mod claim_note;
pub mod freshness;
pub mod kernel;
pub mod payment;
pub mod packed_blob;
pub mod state;
pub mod types;
pub mod wallet_y4;

pub use state::{AppState, SharedState};
