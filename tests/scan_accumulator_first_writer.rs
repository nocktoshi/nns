//! Mock “RPC” by building [`nns_vesl::chain::ScanBlockFetch`] in memory.
//!
//! ## Full-chain claim scan (`%scan-block` with passing candidates)
//!
//! The kernel wraps `+claim-scanner` and `+root-atom:na` in `+mule`. Successful inserts compute
//! the accumulator root via `+root-from-hashable:na` (Tip5 `hash-hashable` over a canonical
//! encoding). Predicate failures (underpaid treasury, sender mismatch, tx-id mismatch, etc.)
//! are skipped and **do not** trap.
//!
//! Tests load `out.jam` (or `NNS_KERNEL_JAM`). Rebuild the kernel jam after Hoon changes:
//! `make kernel-jam` or `hoonc --new hoon/app/app.hoon hoon/`.
//!
//! **`GET /accumulator/:name` JSON** is logged as one line per request when
//! `NNS_PREDICATE_TEST_LOG=1` or `NNS_HTTP_TEST_LOG=1`, or at tracing DEBUG for
//! `nns_accumulator_http_tests`.

use std::sync::{Arc, Once};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use nns_vesl::chain::NNS_GENESIS_HEIGHT as H0;
use nns_vesl::chain::ScanBlockFetch;
use nns_vesl::chain_follower::{
    apply_prefetched_scan_blocks, apply_prefetched_scan_blocks_with_candidates,
};
use nns_vesl::kernel::{
    build_scan_block_poke, build_scan_state_peek, first_scan_block_done, first_scan_block_error,
    ClaimCandidate, ClaimWitness, decode_scan_state,
};
use nns_vesl::payment::{fee_for_name, TREASURY_LOCK_ROOT_B58};
use nns_vesl::{api, state::AppState};
use nockapp::kernel::boot;
use nockapp::kernel::boot::NockStackSize;
use nockapp::wire::{SystemWire, Wire};
use nockapp::NockApp;
use tower::util::ServiceExt;
use vesl_core::SettlementConfig;

static INIT_TRACING: Once = Once::new();

fn kernel_jam() -> Vec<u8> {
    let path = std::env::var("NNS_KERNEL_JAM").unwrap_or_else(|_| "out.jam".into());
    match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => std::fs::read("../out.jam")
            .unwrap_or_else(|e| panic!("could not read kernel jam at {path} or ../out.jam: {e}")),
    }
}

fn digest40(seed: u8) -> Vec<u8> {
    vec![seed; 40]
}

/// Header-only block when using [`apply_prefetched_scan_blocks_with_candidates`].
fn scan_block_fetch_stub(
    height: u64,
    parent: Vec<u8>,
    page_digest: Vec<u8>,
    page_tx_ids: Vec<Vec<u8>>,
) -> ScanBlockFetch {
    ScanBlockFetch {
        height,
        page_digest,
        parent,
        page_tx_ids,
        tx_details: vec![],
    }
}

fn synthetic_claim_candidate(
    name: &str,
    owner: &str,
    tx_atom_byte: u8,
    treasury_nicks: u64,
) -> ClaimCandidate {
    let fee = fee_for_name(name);
    let tx = vec![tx_atom_byte];
    ClaimCandidate {
        name: name.to_string(),
        owner: owner.to_string(),
        fee,
        tx_hash: tx.clone(),
        witness: ClaimWitness {
            tx_id: tx.clone(),
            spender_pkh: owner.as_bytes().to_vec(),
            treasury_amount: treasury_nicks,
            output_lock_root: TREASURY_LOCK_ROOT_B58.to_string(),
        },
    }
}

async fn setup() -> (tempfile::TempDir, nns_vesl::SharedState) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cli = boot::default_boot_cli(true);
    cli.stack_size = NockStackSize::Large;
    INIT_TRACING.call_once(|| {
        let trace_cli = boot::default_boot_cli(true);
        boot::init_default_tracing(&trace_cli);
    });
    let prover_hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let app: NockApp = boot::setup(
        &kernel_jam(),
        cli,
        prover_hot_state.as_slice(),
        "nns-vesl-scan-mock",
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

async fn peek_last_proved_digest(state: &nns_vesl::SharedState) -> Vec<u8> {
    let mut k = state.kernel.lock().await;
    let slab = k
        .peek(build_scan_state_peek())
        .await
        .expect("scan-state peek");
    decode_scan_state(&slab)
        .expect("decode scan-state")
        .last_proved_digest
}

/// Logs the JSON body from `GET /accumulator/<name>` when `NNS_PREDICATE_TEST_LOG=1`,
/// `NNS_HTTP_TEST_LOG=1`, or tracing DEBUG for target `nns_accumulator_http_tests`.
fn log_accumulator_http_response(case_id: &str, uri: &str, status: StatusCode, body: &Value) {
    let payload = json!({
        "target": "nns_accumulator_http_tests",
        "case_id": case_id,
        "uri": uri,
        "status": status.as_u16(),
        "body": body,
    });
    let line = payload.to_string();
    let stderr_json = std::env::var("NNS_PREDICATE_TEST_LOG").ok().as_deref() == Some("1")
        || std::env::var("NNS_HTTP_TEST_LOG").ok().as_deref() == Some("1");
    if stderr_json {
        eprintln!("{line}");
    }
    tracing::debug!(
        target: "nns_accumulator_http_tests",
        payload = %line,
        "GET /accumulator HTTP JSON response"
    );
}

async fn get_accumulator_json(
    state: nns_vesl::SharedState,
    name: &str,
    log_case: &str,
) -> Value {
    let router = api::router(state);
    let uri = format!("/accumulator/{name}");
    let req = Request::builder()
        .method("GET")
        .uri(&uri)
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.expect("route");
    let status = resp.status();
    assert_eq!(status, StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body");
    let body: Value = serde_json::from_slice(&bytes).expect("json");
    log_accumulator_http_response(log_case, &uri, status, &body);
    body
}

// --- Regression: invalid candidates are skipped (no trap) ------------------------------------

#[tokio::test]
async fn scan_block_skips_underpaid_witness() {
    const NAME: &str = "ab.nock";
    let fee_amt = fee_for_name(NAME);
    let (_tmp, state) = setup().await;
    let parent0 = peek_last_proved_digest(&state).await;
    let poke1 = build_scan_block_poke(&parent0, H0, &digest40(0x11), &[], &[]);
    {
        let mut k = state.kernel.lock().await;
        let fx = k.poke(SystemWire.to_wire(), poke1).await.expect("height-1 poke");
        assert!(first_scan_block_done(&fx).is_some());
    }
    let parent1 = peek_last_proved_digest(&state).await;
    let mut cand = synthetic_claim_candidate(NAME, "owner-address", 0x07, fee_amt);
    cand.witness.treasury_amount = 0;
    let poke2 = build_scan_block_poke(
        &parent1,
        H0 + 1,
        &digest40(0x22),
        &[vec![0x07], vec![0x08]],
        std::slice::from_ref(&cand),
    );
    let mut k = state.kernel.lock().await;
    let fx = k.poke(SystemWire.to_wire(), poke2).await.expect("height-2 poke");
    assert!(first_scan_block_error(&fx).is_none());
    assert!(first_scan_block_done(&fx).is_some());
}

#[tokio::test]
async fn scan_block_skips_sender_mismatch() {
    const NAME: &str = "ab.nock";
    let fee_amt = fee_for_name(NAME);
    let (_tmp, state) = setup().await;
    let parent0 = peek_last_proved_digest(&state).await;
    let poke1 = build_scan_block_poke(&parent0, H0, &digest40(0x11), &[], &[]);
    {
        let mut k = state.kernel.lock().await;
        k.poke(SystemWire.to_wire(), poke1)
            .await
            .expect("height-1 poke");
    }
    let parent1 = peek_last_proved_digest(&state).await;
    let mut cand = synthetic_claim_candidate(NAME, "owner-address", 0x07, fee_amt);
    cand.witness.spender_pkh = b"not-the-owner".to_vec();
    let poke2 = build_scan_block_poke(
        &parent1,
        H0 + 1,
        &digest40(0x22),
        &[vec![0x07], vec![0x08]],
        std::slice::from_ref(&cand),
    );
    let mut k = state.kernel.lock().await;
    let fx = k.poke(SystemWire.to_wire(), poke2).await.expect("height-2 poke");
    assert!(first_scan_block_error(&fx).is_none());
    assert!(first_scan_block_done(&fx).is_some());
}

#[tokio::test]
async fn scan_block_skips_witness_tx_id_mismatch() {
    const NAME: &str = "ab.nock";
    let fee_amt = fee_for_name(NAME);
    let (_tmp, state) = setup().await;
    let parent0 = peek_last_proved_digest(&state).await;
    let poke1 = build_scan_block_poke(&parent0, H0, &digest40(0x11), &[], &[]);
    {
        let mut k = state.kernel.lock().await;
        k.poke(SystemWire.to_wire(), poke1)
            .await
            .expect("height-1 poke");
    }
    let parent1 = peek_last_proved_digest(&state).await;
    let mut cand = synthetic_claim_candidate(NAME, "owner-address", 0x07, fee_amt);
    cand.witness.tx_id = vec![0x99];
    let poke2 = build_scan_block_poke(
        &parent1,
        H0 + 1,
        &digest40(0x22),
        &[vec![0x07], vec![0x08]],
        std::slice::from_ref(&cand),
    );
    let mut k = state.kernel.lock().await;
    let fx = k.poke(SystemWire.to_wire(), poke2).await.expect("height-2 poke");
    assert!(first_scan_block_error(&fx).is_none());
    assert!(first_scan_block_done(&fx).is_some());
}

/// Block 1 empty scan + block 2–3 with injected candidates; GET `/accumulator/<name>` → first
/// owner (first-writer-wins).
#[tokio::test]
async fn mock_chain_three_blocks_first_claim_address_wins_in_accumulator() {
    const NAME: &str = "ab.nock";
    const ADDR1: &str = "owner-address";
    const ADDR2: &str = "other-owner";

    let fee_amt = fee_for_name(NAME);
    let treasury = fee_amt;

    let d1 = digest40(0xA1);
    let d2 = digest40(0xA2);
    let d3 = digest40(0xA3);

    let (_tmp, state) = setup().await;
    let mut parent = peek_last_proved_digest(&state).await;

    let b1 = scan_block_fetch_stub(H0, parent.clone(), d1.clone(), vec![]);
    apply_prefetched_scan_blocks(&state, vec![b1])
        .await
        .expect("apply block 1")
        .expect("scan outcome");
    parent = peek_last_proved_digest(&state).await;

    let b2 = scan_block_fetch_stub(
        H0 + 1,
        parent.clone(),
        d2,
        vec![vec![0x07], vec![0x08]],
    );
    let cand2 = vec![synthetic_claim_candidate(NAME, ADDR1, 0x07, treasury)];
    apply_prefetched_scan_blocks_with_candidates(&state, vec![b2], vec![cand2])
        .await
        .expect("apply block 2")
        .expect("scan outcome");
    parent = peek_last_proved_digest(&state).await;

    let b3 = scan_block_fetch_stub(H0 + 2, parent, d3, vec![vec![0x09]]);
    let cand3 = vec![synthetic_claim_candidate(NAME, ADDR2, 0x09, treasury)];
    apply_prefetched_scan_blocks_with_candidates(&state, vec![b3], vec![cand3])
        .await
        .expect("apply block 3")
        .expect("scan outcome");

    let body = get_accumulator_json(state, NAME, "mock_chain_three_blocks").await;
    assert_eq!(body["name"], NAME);
    let value = body["value"].as_object().expect("registered value");
    assert_eq!(value["owner"], ADDR1, "first successful claim must win the row");
}

/// Block 1 advances the anchored digest; block 2 contains two valid claims for the **same** name
/// from different owners. `+claim-scanner:np` folds candidates in list order (same as tx order in
/// the poke); `+insert:na` is first-winner-wins, so the **first** passing candidate wins the row.
#[tokio::test]
async fn same_block_two_addresses_claim_same_name_first_in_block_wins() {
    const NAME: &str = "xy.nock";
    const ADDR1: &str = "first-claimer-address";
    const ADDR2: &str = "second-claimer-address";

    let treasury = fee_for_name(NAME);

    let (_tmp, state) = setup().await;
    let mut parent = peek_last_proved_digest(&state).await;

    let b1 = scan_block_fetch_stub(H0, parent.clone(), digest40(0xC1), vec![]);
    apply_prefetched_scan_blocks(&state, vec![b1])
        .await
        .expect("apply block 1")
        .expect("scan outcome");
    parent = peek_last_proved_digest(&state).await;

    let b2 = scan_block_fetch_stub(
        H0 + 1,
        parent,
        digest40(0xC2),
        vec![vec![0x07], vec![0x08]],
    );
    let candidates = vec![
        synthetic_claim_candidate(NAME, ADDR1, 0x07, treasury),
        synthetic_claim_candidate(NAME, ADDR2, 0x08, treasury),
    ];
    apply_prefetched_scan_blocks_with_candidates(&state, vec![b2], vec![candidates])
        .await
        .expect("apply block 2")
        .expect("scan outcome");

    let body = get_accumulator_json(state, NAME, "same_block_duplicate_name").await;
    assert_eq!(body["name"], NAME);
    let value = body["value"].as_object().expect("registered value");
    assert_eq!(
        value["owner"], ADDR1,
        "first candidate in block tx order must win when two claims target the same name"
    );
}
