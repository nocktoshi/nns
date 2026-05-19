//! Hull application state for the Path Y scanner.
//!
//! The Hoon kernel owns the accumulator and scan cursor. The Rust hull keeps
//! only runtime configuration and follower telemetry around the shared
//! `NockApp`.

use std::path::PathBuf;
use std::sync::Arc;

use nockapp::NockApp;
use serde::Serialize;
use tokio::sync::Mutex;

/// Hull-side state: settlement config and follower telemetry. **Lock
/// ordering:** never acquire [`AppState::hull`] while holding
/// [`AppState::kernel`].
pub struct HullState {
    pub output_dir: PathBuf,
    pub settlement: vesl_core::SettlementConfig,
    /// First block height to `%scan-block` after a genesis cursor; mirrors
    /// [`crate::chain::NNS_GENESIS_HEIGHT`] (same value as `++nns-genesis-height` in Hoon).
    pub nns_genesis_height: u64,
    pub follower: FollowerObservability,
}

/// **Phase 7.1 — Operator observability.** Runtime follower
/// telemetry exposed through `/status` so operators can answer "is the
/// follower stuck?" with a single HTTP call.
///
/// Not persisted. Resets on process restart — which is the right
/// behaviour, because staleness of a "last scan batch at T" timestamp
/// across restarts would be misleading. The authoritative scan cursor
/// lives in kernel state; this only tracks what the follower process
/// observed during its lifetime.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FollowerObservability {
    /// Most recent chain-tip height the follower learned from the
    /// chain endpoint. `None` in local mode (no endpoint) or before
    /// the first successful `fetch_current_tip_height`.
    pub last_chain_tip_height: Option<u64>,
    /// Epoch-millis timestamp of [`last_chain_tip_height`].
    pub last_chain_tip_observed_at_epoch_ms: Option<u64>,
    /// Epoch-millis timestamp of the most recent successful `%scan-block`
    /// batch recorded by the follower. `None` until the first such batch
    /// completes.
    pub last_advance_at_epoch_ms: Option<u64>,
    /// Scan cursor height after that batch (`last-proved-height` in the kernel).
    pub last_advance_tip_height: Option<u64>,
    /// Number of blocks applied in that batch.
    pub last_advance_count: Option<u64>,
    /// Most recent follower-tick failure message. Cleared on the
    /// next successful tick so stale errors don't confuse operators.
    pub last_error: Option<String>,
    /// Epoch-millis timestamp of [`last_error`].
    pub last_error_at_epoch_ms: Option<u64>,
    /// Which follower phase the last error came from. One of
    /// `"scan_block"`, `"scan_peek"`, `"plan"`, or `"scan_poke"`.
    /// Strongly typed as a static string so log aggregators can
    /// histogram on it.
    pub last_error_phase: Option<&'static str>,
}

impl FollowerObservability {
    pub fn record_advance(&mut self, tip_height: u64, count: u64, now_ms: u64) {
        self.last_advance_at_epoch_ms = Some(now_ms);
        self.last_advance_tip_height = Some(tip_height);
        self.last_advance_count = Some(count);
        self.last_error = None;
        self.last_error_at_epoch_ms = None;
        self.last_error_phase = None;
    }

    pub fn record_chain_tip(&mut self, tip: u64, now_ms: u64) {
        self.last_chain_tip_height = Some(tip);
        self.last_chain_tip_observed_at_epoch_ms = Some(now_ms);
    }

    pub fn record_error(&mut self, phase: &'static str, err: String, now_ms: u64) {
        self.last_error = Some(err);
        self.last_error_at_epoch_ms = Some(now_ms);
        self.last_error_phase = Some(phase);
    }
}

/// Shared hull + kernel state. The kernel mutex serializes all Nock I/O;
/// the hull mutex covers settlement config and follower telemetry.
pub struct AppState {
    pub kernel: Mutex<NockApp>,
    pub hull: Mutex<HullState>,
}

pub type SharedState = Arc<AppState>;

impl AppState {
    pub fn new(app: NockApp, output_dir: PathBuf, settlement: vesl_core::SettlementConfig) -> Self {
        Self {
            kernel: Mutex::new(app),
            hull: Mutex::new(HullState {
                output_dir,
                settlement,
                nns_genesis_height: crate::chain::NNS_GENESIS_HEIGHT.max(1),
                follower: FollowerObservability::default(),
            }),
        }
    }

    /// Current monotonic-ish timestamp for telemetry. Wall-clock
    /// `SystemTime` is fine here because we use it for human-readable
    /// "last advanced N seconds ago" math, not anything requiring
    /// strict ordering. Falls back to `0` if the system clock is
    /// before 1970 (shouldn't happen, but don't panic the follower).
    pub fn now_epoch_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// Encode raw atom bytes as lowercase hex. Kept local (no extra
/// dep) because we only need this for the snapshot + settlement
/// JSON surfaces.
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}
