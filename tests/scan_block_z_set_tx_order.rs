//! Z-set tx-id order for `%scan-block` (40-byte atoms) — same visit order as
//! Hoon `~(tap z-in tx-ids)` / `nockchain_math::zoon::zset::ZSet`.
//!
//! Requires `nns.jam` (or `NNS_KERNEL_JAM`).

use std::sync::{Arc, Once};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use nns_vesl::chain::{
    canonical_z_set_tx_order, tip5_hash_to_tx_atom_bytes, NNS_GENESIS_HEIGHT as H0,
};
use nns_vesl::chain_follower::apply_prefetched_scan_blocks_with_candidates;
use nns_vesl::kernel::build_scan_state_peek;
use nns_vesl::kernel::{decode_scan_state, ClaimCandidate, ClaimWitness};
use nns_vesl::payment::{fee_for_name, TREASURY_LOCK_ROOT_B58};
use nns_vesl::state::AppState;
use nns_vesl::{api, chain::ScanBlockFetch};
use nockapp::kernel::boot;
use nockapp::kernel::boot::NockStackSize;
use nockchain_types::tx_engine::common::Hash as Tip5Hash;
use tower::util::ServiceExt;
use vesl_core::SettlementConfig;

static INIT_TRACING: Once = Once::new();

fn kernel_jam() -> Vec<u8> {
    let path = std::env::var("NNS_KERNEL_JAM").unwrap_or_else(|_| "nns.jam".into());
    match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => std::fs::read("../nns.jam")
            .unwrap_or_else(|e| panic!("could not read kernel jam at {path} or ../nns.jam: {e}")),
    }
}

fn digest40(seed: u8) -> Vec<u8> {
    vec![seed; 40]
}

fn atom_from_limbs(l: [u64; 5]) -> Vec<u8> {
    let h = Tip5Hash::from_limbs(&l);
    tip5_hash_to_tx_atom_bytes(&h)
}

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

fn claim40(
    name: &str,
    owner: &str,
    tx_atom: Vec<u8>,
    treasury_nicks: u64,
) -> ClaimCandidate {
    let fee = fee_for_name(name);
    ClaimCandidate {
        name: name.to_string(),
        owner: owner.to_string(),
        fee,
        tx_hash: tx_atom.clone(),
        witness: ClaimWitness {
            tx_id: tx_atom,
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
        nns_vesl::apply_nns_config();
        let trace_cli = boot::default_boot_cli(true);
        boot::init_default_tracing(&trace_cli);
    });
    let prover_hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let app: nockapp::NockApp = boot::setup(
        &kernel_jam(),
        cli,
        prover_hot_state.as_slice(),
        "nns-vesl-zset-order",
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

async fn get_accumulator_owner(state: nns_vesl::SharedState, name: &str) -> String {
    let router = api::router(state);
    let uri = format!("/accumulator/{name}");
    let req = Request::builder()
        .method("GET")
        .uri(&uri)
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.expect("route");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .expect("body");
    assert_eq!(
        status,
        StatusCode::OK,
        "GET {uri} -> {status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    let body: Value = serde_json::from_slice(&bytes).expect("json");
    body["value"]["owner"]
        .as_str()
        .expect("owner")
        .to_string()
}

/// Two Tip5 tx atoms in one block claim the same name. Candidates are passed in
/// **reverse** z-set order; the follower must reorder so the chain-first tx wins.
#[tokio::test]
async fn same_block_same_name_z_set_order_wins_not_rpc_list_order() {
    const NAME: &str = "xy.nock";
    let treasury = fee_for_name(NAME);

    let atom_lo = atom_from_limbs([11, 0, 0, 0, 0]);
    let atom_hi = atom_from_limbs([77, 0, 0, 0, 0]);

    let canon = canonical_z_set_tx_order(vec![atom_hi.clone(), atom_lo.clone()]).unwrap();
    assert_eq!(canon.len(), 2);

    let (c_winner, c_loser, winner_owner) = if canon[0] == atom_lo {
        (
            claim40(NAME, "lo-owner", atom_lo.clone(), treasury),
            claim40(NAME, "hi-owner", atom_hi.clone(), treasury),
            "lo-owner",
        )
    } else {
        (
            claim40(NAME, "hi-owner", atom_hi.clone(), treasury),
            claim40(NAME, "lo-owner", atom_lo.clone(), treasury),
            "hi-owner",
        )
    };

    // Intentionally pass candidates in **wrong** scan order (loser before winner)
    let candidates_wrong = vec![c_loser, c_winner];

    let (_tmp, state) = setup().await;
    let mut parent = peek_last_proved_digest(&state).await;

    let b1 = scan_block_fetch_stub(H0, parent.clone(), digest40(0xE1), vec![]);
    apply_prefetched_scan_blocks_with_candidates(&state, vec![b1], vec![vec![]])
        .await
        .expect("block 1")
        .expect("outcome");
    parent = peek_last_proved_digest(&state).await;

    let b2 = scan_block_fetch_stub(
        H0 + 1,
        parent,
        digest40(0xE2),
        vec![atom_hi.clone(), atom_lo.clone()],
    );
    apply_prefetched_scan_blocks_with_candidates(&state, vec![b2], vec![candidates_wrong])
        .await
        .expect("block 2")
        .expect("outcome");

    let owner = get_accumulator_owner(state, NAME).await;
    assert_eq!(
        owner, winner_owner,
        "owner must match z-set-first tx, not RPC list order"
    );
}
