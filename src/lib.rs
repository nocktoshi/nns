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
//! (`docs/wallet-verification.md`). On chain mode, the first `%scan-block` after
//! genesis uses [`chain::NNS_GENESIS_HEIGHT`](crate::chain::NNS_GENESIS_HEIGHT),
//! mirroring `++nns-genesis-height` in the Hoon kernel.

const DEFAULT_QUIET_RUST_LOG: &str =
    "info,nockapp::kernel::form=warn,nockapp::utils::durability=warn,nockapp::snapshot=warn";

fn apply_default_tracy_no_invariant_check() {
    if std::env::var_os("TRACY_NO_INVARIANT_CHECK").is_none() {
        std::env::set_var("TRACY_NO_INVARIANT_CHECK", "1");
    }
}

fn apply_default_quiet_nockapp_kernel_logs() {
    use std::env;
    if env::var_os("RUST_LOG").is_some() {
        return;
    }
    if env::var_os("MINIMAL_LOG_FORMAT").is_none() {
        env::set_var("MINIMAL_LOG_FORMAT", "1");
    }
    env::set_var("RUST_LOG", DEFAULT_QUIET_RUST_LOG);
}

#[derive(Debug, Default, serde::Deserialize)]
struct VeslTomlTracingSnarf {
    #[serde(default)]
    tracing_env: Option<TracingEnvTable>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct TracingEnvTable {
    tracy_no_invariant_check: Option<String>,
    minimal_log_format: Option<String>,
    rust_log: Option<String>,
}

/// Reads the `NNS_CONFIG` path (default `nns.toml`) and applies the optional `[tracing_env]`
/// table: sets `TRACY_NO_INVARIANT_CHECK`, `MINIMAL_LOG_FORMAT`, and `RUST_LOG` only
/// when those variables are not already set in the process environment.
///
/// If the file is missing, unreadable, or has no `[tracing_env]` section, applies the same
/// built-in defaults the repo used to set before `nns.toml` carried them (Tracy TSC check off,
/// quieter nockapp kernel log targets).
///
/// Call before [`nockapp::kernel::boot::init_default_tracing`].
pub fn apply_nns_config() {
    use std::env;
    let path = env::var("NNS_CONFIG").unwrap_or_else(|_| "nns.toml".into());
    let Ok(contents) = std::fs::read_to_string(&path) else {
        apply_default_tracy_no_invariant_check();
        apply_default_quiet_nockapp_kernel_logs();
        return;
    };
    let snarf: VeslTomlTracingSnarf = toml::from_str(&contents).unwrap_or_default();
    let Some(table) = snarf.tracing_env else {
        apply_default_tracy_no_invariant_check();
        apply_default_quiet_nockapp_kernel_logs();
        return;
    };

    if env::var_os("TRACY_NO_INVARIANT_CHECK").is_none() {
        if let Some(v) = table.tracy_no_invariant_check.as_ref().filter(|s| !s.is_empty()) {
            env::set_var("TRACY_NO_INVARIANT_CHECK", v);
        } else {
            apply_default_tracy_no_invariant_check();
        }
    }

    if env::var_os("RUST_LOG").is_none() {
        if let Some(v) = table.rust_log.as_ref().filter(|s| !s.is_empty()) {
            if env::var_os("MINIMAL_LOG_FORMAT").is_none() {
                let mf = table
                    .minimal_log_format
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("1");
                env::set_var("MINIMAL_LOG_FORMAT", mf);
            }
            env::set_var("RUST_LOG", v);
        } else {
            apply_default_quiet_nockapp_kernel_logs();
        }
    } else if env::var_os("MINIMAL_LOG_FORMAT").is_none() {
        if let Some(v) = table.minimal_log_format.as_ref().filter(|s| !s.is_empty()) {
            env::set_var("MINIMAL_LOG_FORMAT", v);
        }
    }
}

pub mod api;
pub mod chain;
pub mod chain_follower;
pub mod claim_note;
pub mod freshness;
pub mod formula_nock;
pub mod kernel;
pub mod noun_access;
pub mod payment;
pub mod packed_blob;
pub mod state;
pub mod types;
pub mod wallet_y4;

pub use state::{AppState, SharedState};
