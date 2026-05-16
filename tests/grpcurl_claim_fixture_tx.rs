//! Regression: real `GetTransactionDetails`-shaped v1 claim tx (grpcurl JSON fixture).
//!
//! Tx id `8FGo74Qm29C8XTsDTdaaeXvGix2rS7reWQuF2Qj84GypetGW3ufFB5d` — same blobs as
//! `grpcurl GetTransactionDetails` sample (memo / lock / claim `blob` on one output).
//!
//! 1. Decodes the wallet `blob` into [`ClaimNoteV1`] (path `nns/v1/claim/<name>.nock` on this tx).
//! 2. Builds matching [`TransactionDetails`] (treasury `lock` jam + amount + signers).
//! 3. Runs two `%scan-block` steps (empty block then claim block) and asserts the
//!    accumulator records the claimed name.
//!
//! Requires `nns.jam` / `NNS_KERNEL_JAM` (same as other kernel integration tests).

use std::sync::{Arc, Once};

use base64::Engine;
use nockapp::kernel::boot;
use nockapp::kernel::boot::NockStackSize;
use nockapp::wire::Wire;
use nockapp_grpc::pb::common::v1::{Belt, Hash as PbHash, Nicks};
use nockapp_grpc::pb::common::v2::{NoteData as PbNoteData, NoteDataEntry as PbNoteDataEntry};
use nockapp_grpc::pb::public::v2::transaction_details::{FeeRequired, TotalOutputRequired};
use nockapp_grpc::pb::public::v2::transaction_output::AmountRequired;
use nockapp_grpc::pb::public::v2::{
    TransactionDetails, TransactionInput, TransactionOutput,
};
use nns_vesl::chain::{canonical_z_set_tx_order, ScanBlockFetch, NNS_GENESIS_HEIGHT as H0};
use nns_vesl::chain_follower::{apply_prefetched_scan_blocks, claim_candidates_from_fetch};
use nns_vesl::claim_note::ClaimNoteV1;
use nns_vesl::kernel::{
    build_scan_block_poke, build_scan_state_peek, decode_scan_state, first_scan_block_done,
};
use nns_vesl::payment::fee_for_name;
use nns_vesl::state::AppState;
use nockchain_client_rs::{NoteData, NoteDataEntry};
use vesl_core::SettlementConfig;

/// Synthetic owner for the fixture tx: must match every input's `signer_pubkey_b58` so
/// path-style blobs (empty owner in note-data) still produce a kernel-consistent candidate.
const GRPCURL_FIXTURE_OWNER_B58: &str = "grpcurlFixtureSignerPubkeyB58Placeholder";

static INIT_TRACING: Once = Once::new();

fn kernel_jam() -> Vec<u8> {
    let path = std::env::var("NNS_KERNEL_JAM").unwrap_or_else(|_| "nns.jam".into());
    match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => std::fs::read("../nns.jam")
            .unwrap_or_else(|e| panic!("could not read kernel jam at {path} or ../nns.jam: {e}")),
    }
}

fn b64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s.trim())
        .unwrap_or_else(|e| panic!("base64 decode: {e}"))
}

fn pb_hash(limbs: [u64; 5]) -> PbHash {
    PbHash {
        belt_1: Some(Belt { value: limbs[0] }),
        belt_2: Some(Belt { value: limbs[1] }),
        belt_3: Some(Belt { value: limbs[2] }),
        belt_4: Some(Belt { value: limbs[3] }),
        belt_5: Some(Belt { value: limbs[4] }),
    }
}

/// Decode claim triple from the fixture `blob` entry (wallet-packed wire inside jammed atom).
fn fixture_claim_note() -> ClaimNoteV1 {
    let memo_blob = b64("gYAN+E5vY+uAP4YWlg74biBp84APAiX3DvhvZiDvgG8GUjcP+GVmdeyAD3L1Jg/4ayBm74AvBwInD/hpdmH0gF/GAlIP+G5zdO+ADwcXJg74bGUg4YAPB8eWDvhjYXTpgP7mNucGfBChunTAZ6MDCQf8ORA6dMAvAzFLB3S5OTow4GsttUADvtzbmzmgt7yZtQH/6k1uHfBBUPTWAR3q7SoN+CBwcu+AT/c29g74bCwg4YDPJlcWDvhkeSD0gI9WBsIO+GFyZ+WAP0cHAg/4cm926YDvdgbiDvhldHfvgC+3BpIO+G4gdOiAXwZy9w7ocmxkrg==");
    let lock_blob = b64("WcCDW0PHBRBQex8fOM4VbgzQa5Wq3dMGf88B9qCRR0SBZMoYQICvJaX+9cslH1Dfad/bp7Tpmgo=");
    let claim_blob_wire = b64("wXZAd3ObewO+XczLOOCzhaW1A/6L29s44K+NoYUDfpqbizvgvY2tBQ==");

    let nd = NoteData::new(vec![
        NoteDataEntry::new("memo".into(), memo_blob.into()),
        NoteDataEntry::new("lock".into(), lock_blob.into()),
        NoteDataEntry::new("blob".into(), claim_blob_wire.into()),
    ]);
    ClaimNoteV1::from_note_data(&nd).expect("fixture claim blob must decode")
}

/// `TransactionDetails` matching the grpcurl fixture (plus `signer_pubkey_b58` on inputs).
fn fixture_transaction_details() -> TransactionDetails {
    let block_id = pb_hash([
        13401731103746172235,
        11120624753365347290,
        15862456427512461353,
        2194106849203569876,
        13149414720405213383,
    ]);
    let parent = pb_hash([
        9835645607722547979,
        11578373265973268225,
        463902109858752446,
        13922555901868572272,
        6952742624985174228,
    ]);

    let note_name = "9vXqzHoeNn6RvZVrrs2SJuCAMmaWYcAn6YoF7ch7ALEt3KbCjtLuoC4".to_string();
    let memo_blob = b64("gYAN+E5vY+uAP4YWlg74biBp84APAiX3DvhvZiDvgG8GUjcP+GVmdeyAD3L1Jg/4ayBm74AvBwInD/hpdmH0gF/GAlIP+G5zdO+ADwcXJg74bGUg4YAPB8eWDvhjYXTpgP7mNucGfBChunTAZ6MDCQf8ORA6dMAvAzFLB3S5OTow4GsttUADvtzbmzmgt7yZtQH/6k1uHfBBUPTWAR3q7SoN+CBwcu+AT/c29g74bCwg4YDPJlcWDvhkeSD0gI9WBsIO+GFyZ+WAP0cHAg/4cm926YDvdgbiDvhldHfvgC+3BpIO+G4gdOiAXwZy9w7ocmxkrg==");
    let lock_blob = b64("WcCDW0PHBRBQex8fOM4VbgzQa5Wq3dMGf88B9qCRR0SBZMoYQICvJaX+9cslH1Dfad/bp7Tpmgo=");
    let claim_blob_wire = b64("wXZAd3ObewO+XczLOOCzhaW1A/6L29s44K+NoYUDfpqbizvgvY2tBQ==");

    let signer = vec![GRPCURL_FIXTURE_OWNER_B58.to_string()];

    let outputs = vec![TransactionOutput {
        note_name_b58: note_name.clone(),
        amount_required: Some(AmountRequired::Amount(Nicks {
            value: 658_755_463,
        })),
        lock_summary: "lock:9vXqzHoe".into(),
        note_data: Some(PbNoteData {
            entries: vec![
                PbNoteDataEntry {
                    key: "memo".into(),
                    blob: memo_blob,
                },
                PbNoteDataEntry {
                    key: "lock".into(),
                    blob: lock_blob,
                },
                PbNoteDataEntry {
                    key: "blob".into(),
                    blob: claim_blob_wire,
                },
            ],
        }),
    }];

    let inputs = vec![
        TransactionInput {
            note_name_b58: note_name.clone(),
            amount: None,
            source_tx_id: String::new(),
            coinbase: false,
            signer_pubkey_b58: signer.clone(),
        },
        TransactionInput {
            note_name_b58: note_name,
            amount: None,
            source_tx_id: String::new(),
            coinbase: false,
            signer_pubkey_b58: signer,
        },
    ];

    TransactionDetails {
        tx_id: "8FGo74Qm29C8XTsDTdaaeXvGix2rS7reWQuF2Qj84GypetGW3ufFB5d".into(),
        block_id: Some(block_id),
        parent: Some(parent),
        height: 63900,
        timestamp: 9_223_372_093_638_336_313,
        version: 1,
        size_bytes: 8652,
        total_input: None,
        total_output_required: Some(TotalOutputRequired::TotalOutput(Nicks {
            value: 658_755_463,
        })),
        fee_required: Some(FeeRequired::Fee(Nicks {
            value: 1_654_784,
        })),
        inputs,
        outputs,
    }
}

fn digest40(seed: u8) -> Vec<u8> {
    vec![seed; 40]
}

#[test]
fn grpcurl_fixture_blob_wire_unpacks() {
    let wire = include_bytes!("fixtures/grpcurl_claim_blob.wire");
    let inner = nns_vesl::packed_blob::unpack_wallet_blob_jam(wire).expect("unpack wallet blob");
    assert_eq!(
        std::str::from_utf8(&inner).unwrap(),
        "nns/v1/claim/nockchain.nock"
    );
}

#[test]
fn grpcurl_fixture_claim_decodes_and_matches_tx_id() {
    let claim = fixture_claim_note();
    assert_eq!(claim.name, "nockchain.nock");
    assert!(claim.owner.is_empty() && claim.tx_hash.is_empty());
    let details = fixture_transaction_details();
    let cands = claim_candidates_from_fetch(&nns_vesl::chain::ScanBlockFetch {
        height: 0,
        page_digest: vec![],
        parent: vec![],
        page_tx_ids: vec![],
        tx_details: vec![details],
    })
    .expect("candidates");
    assert_eq!(cands.len(), 1);
    assert_eq!(cands[0].name, "nockchain.nock");
    assert_eq!(cands[0].owner, GRPCURL_FIXTURE_OWNER_B58);
    assert_eq!(
        cands[0].tx_hash,
        nns_vesl::chain::base58_hash_to_atom_bytes(
            "8FGo74Qm29C8XTsDTdaaeXvGix2rS7reWQuF2Qj84GypetGW3ufFB5d"
        )
        .unwrap()
    );
    let min_fee = fee_for_name(&claim.name);
    assert!(
        658_755_463 >= min_fee,
        "fixture output pays at least fee schedule {min_fee} nicks for {:?}",
        claim.name
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grpcurl_fixture_accumulator_inserts_claimed_name() {
    INIT_TRACING.call_once(|| {
        nns_vesl::apply_nns_config();
        let cli = boot::default_boot_cli(true);
        boot::init_default_tracing(&cli);
    });

    let claim = fixture_claim_note();
    let details = fixture_transaction_details();

    let tx_atom =
        nns_vesl::chain::base58_hash_to_atom_bytes(&details.tx_id).expect("tx id atom");
    let page_tx_ids = canonical_z_set_tx_order(vec![tx_atom.clone()]).expect("z-set order");

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cli = boot::default_boot_cli(true);
    cli.stack_size = NockStackSize::Large;
    let prover_hot_state = zkvm_jetpack::hot::produce_prover_hot_state();
    let app = boot::setup(
        &kernel_jam(),
        cli,
        prover_hot_state.as_slice(),
        "nns-grpcurl-claim-fixture",
        Some(tmp.path().to_path_buf()),
    )
    .await
    .expect("kernel boot");
    let state = Arc::new(AppState::new(
        app,
        tmp.path().to_path_buf(),
        SettlementConfig::local(),
    ));

    let genesis_parent = {
        let mut k = state.kernel.lock().await;
        let slab = k.peek(build_scan_state_peek()).await.expect("peek genesis");
        decode_scan_state(&slab)
            .expect("decode scan-state")
            .last_proved_digest
    };

    let d1 = digest40(0xC1);
    let b1 = ScanBlockFetch {
        height: H0,
        page_digest: d1.clone(),
        parent: genesis_parent,
        page_tx_ids: vec![],
        tx_details: vec![],
    };
    apply_prefetched_scan_blocks(&state, vec![b1])
        .await
        .expect("apply block 1")
        .expect("outcome 1");

    let d2 = digest40(0xC2);
    let fetch2 = ScanBlockFetch {
        height: H0 + 1,
        page_digest: d2.clone(),
        parent: d1.clone(),
        page_tx_ids: page_tx_ids.clone(),
        tx_details: vec![details],
    };

    let candidates = claim_candidates_from_fetch(&fetch2).expect("extract");
    assert_eq!(candidates.len(), 1);
    assert!(
        candidates[0].witness.treasury_amount >= fee_for_name(&candidates[0].name),
        "witness treasury must cover fee"
    );

    apply_prefetched_scan_blocks(&state, vec![fetch2])
        .await
        .expect("apply block 2")
        .expect("outcome 2");

    let st = {
        let mut k = state.kernel.lock().await;
        let slab = k.peek(build_scan_state_peek()).await.expect("peek final");
        decode_scan_state(&slab).expect("decode final")
    };
    assert!(
        st.accumulator_size >= 1,
        "accumulator should contain {:?} (size {})",
        claim.name,
        st.accumulator_size
    );

    // Next height after claim block: duplicate-name rescan still succeeds.
    let d3 = digest40(0xC3);
    let poke3 = build_scan_block_poke(&d2, H0 + 2, &d3, &page_tx_ids, &candidates);
    let mut k = state.kernel.lock().await;
    let fx = k
        .poke(nockapp::wire::SystemWire.to_wire(), poke3)
        .await
        .expect("poke height 3");
    assert!(
        first_scan_block_done(&fx).is_some(),
        "duplicate-name rescan should complete without trap"
    );
}
