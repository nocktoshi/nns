//! Shared kernel boot and helpers for integration tests that poke/peek Hoon
//! predicates (`nns-predicates`, fee schedule, chain link, tx-in-page).
//!
//! ## Logging predicate API responses
//!
//! Kernel outcomes are logged with `tracing` at **DEBUG** under target
//! `nns_predicate_tests`. Enable with for example:
//!
//! ```text
//! RUST_LOG=nns_predicate_tests=debug cargo +nightly test ...
//! ```
//!
//! Alternatively set **`NNS_PREDICATE_TEST_LOG=1`** to print one JSON line per logged
//! kernel outcome to stderr (works without configuring `RUST_LOG`).

use std::sync::{Arc, Once};

use nockapp::kernel::boot;
use nockapp::kernel::boot::NockStackSize;
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use vesl_core::SettlementConfig;

use nns_vesl::kernel::{
    build_fee_for_name_peek, build_validate_claim_poke, build_verify_chain_link_poke,
    build_verify_tx_in_page_poke, decode_fee_for_name, first_chain_link_result,
    first_tx_in_page_result, first_validate_claim_result, AnchorHeader, ClaimBundle,
    ValidateClaimResult,
};
use nns_vesl::state::AppState;
use serde_json::json;

static TRACING_INIT: Once = Once::new();

/// Default boot name for Phase 3 predicate tests (`tests/phase3_predicates.rs`).
pub const BOOT_PHASE3: &str = "nns-phase3-test";

pub fn kernel_jam_bytes() -> Vec<u8> {
    let path = std::env::var("NNS_KERNEL_JAM").unwrap_or_else(|_| "out.jam".to_string());
    std::fs::read(&path)
        .or_else(|_| std::fs::read("../out.jam"))
        .unwrap_or_else(|e| panic!("could not read kernel jam at {path} or ../out.jam: {e}"))
}

/// Logs a kernel poke/peek result for manual inspection when debugging tests.
///
/// Output is a single JSON object per call (stderr when `NNS_PREDICATE_TEST_LOG=1`, and as the
/// `payload` field at DEBUG for `nns_predicate_tests`).
pub fn log_predicate_api(case_id: &str, operation: &str, response: &str) {
    let payload = json!({
        "target": "nns_predicate_tests",
        "case_id": case_id,
        "operation": operation,
        "response": response,
    });
    let line = payload.to_string();
    if std::env::var("NNS_PREDICATE_TEST_LOG").ok().as_deref() == Some("1") {
        eprintln!("{line}");
    }
    tracing::debug!(
        target: "nns_predicate_tests",
        payload = %line,
        "kernel predicate API"
    );
}

fn init_tracing_once() {
    TRACING_INIT.call_once(|| {
        nns_vesl::prepare_tracy_for_host_cpu();
        let cli = boot::default_boot_cli(true);
        boot::init_default_tracing(&cli);
    });
}

/// Boots the kernel with Tip5 jets (`zkvm-jetpack` hot state). Each test binary
/// shares one tracing initializer via [`TRACING_INIT`].
pub async fn boot_kernel(
    app_label: &'static str,
) -> (tempfile::TempDir, nns_vesl::state::SharedState) {
    init_tracing_once();
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cli = boot::default_boot_cli(true);
    cli.stack_size = NockStackSize::Large;
    let prover_hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let app: NockApp = boot::setup(
        &kernel_jam_bytes(),
        cli,
        prover_hot_state.as_slice(),
        app_label,
        Some(tmp.path().to_path_buf()),
    )
    .await
    .expect("kernel boot");
    let state = Arc::new(AppState::new(
        app,
        tmp.path().to_path_buf(),
        SettlementConfig::local(),
    ));
    (tmp, state)
}

pub async fn fee_for_name_via_kernel(state: &nns_vesl::state::SharedState, name: &str) -> u64 {
    let mut k = state.kernel.lock().await;
    let res = k
        .peek(build_fee_for_name_peek(name))
        .await
        .expect("fee peek");
    decode_fee_for_name(&res).expect("decode fee")
}

/// Synthetic 40-byte digest (same pattern everywhere in predicate tests).
pub fn digest(seed: u8) -> Vec<u8> {
    vec![seed; 40]
}

pub fn anchor_header(height: u64, seed: u8, parent_seed: u8) -> AnchorHeader {
    AnchorHeader {
        digest: digest(seed),
        height,
        parent: digest(parent_seed),
    }
}

pub async fn poke_verify_chain_link(
    state: &nns_vesl::state::SharedState,
    case_id: &str,
    claim_digest: &[u8],
    headers: &[AnchorHeader],
    anchored_tip: &[u8],
) -> bool {
    let poke = build_verify_chain_link_poke(claim_digest, headers, anchored_tip);
    let effects = {
        let mut k = state.kernel.lock().await;
        k.poke(SystemWire.to_wire(), poke)
            .await
            .expect("chain-link poke")
    };
    let ok = first_chain_link_result(&effects).expect("chain-link-result effect");
    log_predicate_api(case_id, "poke/verify-chain-link", &ok.to_string());
    ok
}

pub async fn poke_verify_tx_in_page(
    state: &nns_vesl::state::SharedState,
    case_id: &str,
    page_digest: &[u8],
    tx_ids: &[Vec<u8>],
    claimed: &[u8],
) -> bool {
    let poke = build_verify_tx_in_page_poke(page_digest, tx_ids, claimed);
    let effects = {
        let mut k = state.kernel.lock().await;
        k.poke(SystemWire.to_wire(), poke)
            .await
            .expect("tx-in-page poke")
    };
    let ok = first_tx_in_page_result(&effects).expect("tx-in-page-result effect");
    log_predicate_api(case_id, "poke/verify-tx-in-page", &ok.to_string());
    ok
}

/// Baseline bundle satisfying `+validate-claim-bundle` in `nns-predicates.hoon`
/// (G1, C2, Level B, Level A chain, Level C-A witness).
pub fn good_claim_bundle() -> ClaimBundle {
    let page_digest = vec![0x42_u8];
    let tx_hash = vec![0x07_u8];
    let other_tx = vec![0x08_u8];

    ClaimBundle {
        name: "ab.nock".to_string(),
        owner: "owner-address".to_string(),
        fee: 327_680_000,
        tx_hash: tx_hash.clone(),
        claim_block_digest: page_digest.clone(),
        anchor_headers: vec![],
        page_digest: page_digest.clone(),
        page_tx_ids: vec![tx_hash, other_tx],
        anchored_tip: page_digest,
        anchored_tip_height: 0,
        witness: nns_vesl::kernel::ClaimWitness {
            tx_id: vec![0x07],
            spender_pkh: b"owner-address".to_vec(),
            treasury_amount: 327_680_000,
            output_lock_root: "A3LoWjxurwiyzhkv8sgDv2MVu9PwgWHmqoncXw9GEQ5M3qx46svvadE".to_string(),
        },
    }
}

pub async fn poke_validate_claim(
    state: &nns_vesl::state::SharedState,
    case_id: &str,
    bundle: &ClaimBundle,
) -> ValidateClaimResult {
    let poke = build_validate_claim_poke(bundle);
    let effects = {
        let mut k = state.kernel.lock().await;
        k.poke(SystemWire.to_wire(), poke)
            .await
            .expect("validate-claim poke")
    };
    let res = first_validate_claim_result(&effects).expect("validate-claim effect");
    log_predicate_api(case_id, "poke/validate-claim", &format!("{res:?}"));
    res
}
