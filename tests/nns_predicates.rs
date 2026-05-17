//! Structured coverage for `hoon/lib/nns-predicates.hoon` behavior exposed through kernel
//! pokes/peeks (`%validate-claim`, `%verify-chain-link`, `%verify-tx-in-page`, fee peek).
//!
//! Shared boot helpers live in `tests/common/mod.rs`. Enable API response logs:
//! `RUST_LOG=nns_predicate_tests=debug` or `NNS_PREDICATE_TEST_LOG=1`.

mod common;

use common::{
    anchor_header, boot_kernel, digest, good_claim_bundle, log_predicate_api,
    poke_validate_claim, poke_verify_chain_link, poke_verify_tx_in_page, BOOT_PHASE3,
};
use nns::kernel::{ClaimBundle, ValidateClaimResult};
use nns::payment::fee_for_name;

const CASE_MATRIX: &str = "validate_claim_bundle_matrix";

/// Maps each [`validation-error`](../../hoon/lib/nns-predicates.hoon) tag to a representative
/// failing bundle and the expected [`ValidateClaimResult`].
fn validate_claim_failure_cases(
) -> Vec<(&'static str, Box<dyn Fn() -> ClaimBundle + Send>, &'static str)> {
    vec![
        (
            "invalid-name",
            Box::new(|| {
                let mut b = good_claim_bundle();
                b.name = "no-suffix".to_string();
                b
            }),
            "invalid-name",
        ),
        (
            "fee-below-schedule",
            Box::new(|| {
                let mut b = good_claim_bundle();
                b.fee = 327_679_999;
                b
            }),
            "fee-below-schedule",
        ),
        (
            "witness-tx-id-mismatch",
            Box::new(|| {
                let mut b = good_claim_bundle();
                b.witness.tx_id = vec![0xEE];
                b
            }),
            "witness-tx-id-mismatch",
        ),
        (
            "witness-sender-mismatch",
            Box::new(|| {
                let mut b = good_claim_bundle();
                b.witness.spender_pkh = b"nope".to_vec();
                b
            }),
            "witness-sender-mismatch",
        ),
        (
            "witness-underpaid",
            Box::new(|| {
                let mut b = good_claim_bundle();
                b.witness.treasury_amount = 1;
                b
            }),
            "witness-underpaid",
        ),
        (
            "witness-wrong-treasury",
            Box::new(|| {
                let mut b = good_claim_bundle();
                b.witness.output_lock_root =
                    "4uhcJHPZN6759D8ukUopNpVNPG3ho18pYjksyS81NLXo".to_string();
                b
            }),
            "witness-wrong-treasury",
        ),
        (
            "page-digest-mismatch",
            Box::new(|| {
                let mut b = good_claim_bundle();
                b.claim_block_digest = vec![0x99];
                b.page_digest = vec![0x42];
                b.anchored_tip = vec![0x99];
                b
            }),
            "page-digest-mismatch",
        ),
        (
            "tx-not-in-page",
            Box::new(|| {
                let mut b = good_claim_bundle();
                b.page_tx_ids = vec![vec![0x55]];
                b
            }),
            "tx-not-in-page",
        ),
        (
            "chain-broken",
            Box::new(|| {
                let mut b = good_claim_bundle();
                b.anchored_tip = vec![0x77];
                b
            }),
            "chain-broken",
        ),
    ]
}

#[tokio::test]
async fn validate_claim_bundle_matrix_failures_match_tags() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;

    for (tag, build, expect_tag) in validate_claim_failure_cases() {
        let bundle = build();
        let res = poke_validate_claim(&state, CASE_MATRIX, &bundle).await;
        let ValidateClaimResult::Error(msg) = &res else {
            panic!("{tag}: expected Error, got {res:?}");
        };
        assert_eq!(msg, expect_tag, "{tag}: predicate tag mismatch");
        log_predicate_api(
            CASE_MATRIX,
            &format!("matrix-row/{tag}"),
            &format!("ok reject tag={msg}"),
        );
    }
}

#[tokio::test]
async fn validate_claim_bundle_matrix_happy_and_short_circuit() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;

    let ok_bundle = good_claim_bundle();
    let r = poke_validate_claim(&state, CASE_MATRIX, &ok_bundle).await;
    assert_eq!(r, ValidateClaimResult::Ok);

    // Multiple violations — validator returns first failure (`invalid-name`).
    let mut bad = good_claim_bundle();
    bad.name = "BAD".into();
    bad.fee = 0;
    bad.anchored_tip = vec![0xFF];
    let r = poke_validate_claim(&state, CASE_MATRIX, &bad).await;
    assert_eq!(
        r,
        ValidateClaimResult::Error("invalid-name".into()),
        "short-circuit: invalid-name before fee-below-schedule"
    );
}

// --- Isolated Level A / B poke APIs (same Hoon arms as bundled validator) --------------

#[tokio::test]
async fn chain_links_predicate_api_accepts_and_rejects() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let id = "chain_links_predicate_api";

    assert!(
        poke_verify_chain_link(&state, id, &digest(7), &[], &digest(7))
            .await,
        "empty headers, claim digest equals tip"
    );
    assert!(
        !poke_verify_chain_link(&state, id, &digest(7), &[], &digest(9))
            .await,
        "empty headers wrong tip"
    );

    let headers = vec![
        anchor_header(2, 2, 1),
        anchor_header(3, 3, 2),
        anchor_header(4, 4, 3),
    ];
    assert!(poke_verify_chain_link(&state, id, &digest(1), &headers, &digest(4)).await);
}

#[tokio::test]
async fn has_tx_in_page_predicate_api_accepts_and_rejects() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let id = "has_tx_in_page_predicate_api";

    let tx = vec![0x2au8];
    assert!(
        poke_verify_tx_in_page(&state, id, &digest(1), std::slice::from_ref(&tx), &tx).await
    );
    assert!(
        !poke_verify_tx_in_page(
            &state,
            id,
            &digest(1),
            std::slice::from_ref(&tx),
            &[0x2b]
        )
        .await
    );
}

#[tokio::test]
async fn fee_for_name_matches_rust_payment_module() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let name = "xy.nock";
    let rust = fee_for_name(name);
    let hoon = common::fee_for_name_via_kernel(&state, name).await;
    assert_eq!(rust, hoon, "fee-for-name parity");
    log_predicate_api(
        "fee_schedule_smoke",
        "peek/fee-for-name",
        &format!("rust={rust} hoon={hoon}"),
    );
}
