//! Phase 3 Level A predicate tests.
//!
//! Pins two invariants that Phase 3c's recursive gate will depend on:
//!
//!   - `fee-for-name:nns-predicates` in Hoon matches
//!     `payment::fee_for_name` in Rust, bit-for-bit, across the full
//!     fee-tier boundary surface. If either side drifts, Phase 3's
//!     C2 check inside the gate will start rejecting claims the hull
//!     pre-approved (or accepting ones it should have rejected).
//!
//!   - `chain-links-to:nns-predicates` accepts a valid
//!     `AnchorHeader` chain from a claim digest to the follower's
//!     anchored tip, and rejects every failure mode (parent break,
//!     height gap, wrong tip, empty chain that doesn't terminate at
//!     the tip).
//!
//! These are pure-kernel peeks driven from Rust — no STARK prove, no
//! tx-engine dependency, so they run in seconds on every
//! `cargo test`.
//!
//! Shared boot/helpers: `tests/common/mod.rs`.

mod common;

use common::{
    anchor_header, boot_kernel, digest, fee_for_name_via_kernel, good_claim_bundle,
    poke_validate_claim, poke_verify_chain_link, poke_verify_tx_in_page, BOOT_PHASE3,
};
use nns::kernel::{AnchorHeader, ValidateClaimResult};
use nns::payment::fee_for_name;

/// Cross-repo parity: every Rust `payment::fee_for_name` input must
/// return the same u64 from the kernel's `fee-for-name:nns-predicates`.
///
/// Covers each tier plus boundaries, names with and without the
/// `.nock` suffix, and the "too-long" bucket.
#[tokio::test]
async fn fee_for_name_parity_hoon_rust() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;

    let cases = [
        // tier: 1..=4 chars -> 327_680_000 nicks (5000 NOCK)
        ("a.nock", 327_680_000),
        ("ab.nock", 327_680_000),
        ("abc.nock", 327_680_000),
        ("abcd.nock", 327_680_000),
        // tier: 5..=9 chars -> 32_768_000 nicks (500 NOCK)
        ("abcde.nock", 32_768_000),
        ("abcdef.nock", 32_768_000),
        ("abcdefgh.nock", 32_768_000),
        ("abcdefghi.nock", 32_768_000),
        // tier: 10+ chars -> 6_553_600 nicks (100 NOCK)
        ("abcdefghij.nock", 6_553_600),
        ("zzzzzzzzzzzzzzzzzzzz.nock", 6_553_600),
        // empty stem (G1 would reject before fee; exercises the zero path)
        (".nock", 0),
        ("", 0),
        // lookups without the .nock suffix still derive a sensible fee,
        // matching Rust's `strip_suffix(".nock").unwrap_or(name)` fallback.
        ("abcd", 327_680_000),
        ("abcde", 32_768_000),
        ("abcdefghij", 6_553_600),
    ];

    for (name, expected) in cases {
        let rust = fee_for_name(name);
        let hoon = fee_for_name_via_kernel(&state, name).await;
        assert_eq!(
            rust, expected,
            "Rust fee_for_name({name:?}) = {rust}, expected {expected}",
        );
        assert_eq!(
            hoon, expected,
            "Hoon /fee-for-name/{name:?} = {hoon}, expected {expected}",
        );
        assert_eq!(
            rust, hoon,
            "Hoon/Rust fee drift on {name:?}: Rust={rust} Hoon={hoon}",
        );
    }
}

/// Sanity: long names (100+ bytes) don't blow up the peek path.
#[tokio::test]
async fn fee_for_name_accepts_long_names() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let long_name = format!("{}.nock", "z".repeat(200));
    let rust = fee_for_name(&long_name);
    let hoon = fee_for_name_via_kernel(&state, &long_name).await;
    assert_eq!(rust, 6_553_600);
    assert_eq!(hoon, 6_553_600);
}

// =========================================================================
// chain-links-to — Phase 3 Level A header-chain walker
// =========================================================================

/// Synthetic 40-byte digest built from a seed; lets us reason about
/// parent/digest relationships in tests without a real Tip5 hash.
/// Empty header list: claim's own block IS the anchored tip.
#[tokio::test]
async fn chain_link_accepts_claim_is_tip() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let ok = poke_verify_chain_link(&state, "phase3", &digest(7), &[], &digest(7)).await;
    assert!(
        ok,
        "claim-digest == anchored-tip with empty headers should pass"
    );
}

/// Empty headers + different tip must fail.
#[tokio::test]
async fn chain_link_rejects_empty_chain_wrong_tip() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let ok = poke_verify_chain_link(&state, "phase3", &digest(7), &[], &digest(9)).await;
    assert!(!ok, "empty chain from claim=7 to tip=9 must be rejected");
}

/// Happy path: claim at digest=1, chain [2<-1, 3<-2, 4<-3], tip=4.
#[tokio::test]
async fn chain_link_accepts_three_header_chain() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let headers = vec![anchor_header(2, 2, 1), anchor_header(3, 3, 2), anchor_header(4, 4, 3)];
    let ok = poke_verify_chain_link(&state, "phase3", &digest(1), &headers, &digest(4)).await;
    assert!(ok);
}

/// First header's parent doesn't match claim-digest.
#[tokio::test]
async fn chain_link_rejects_first_parent_mismatch() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let headers = vec![anchor_header(2, 2, 99)]; // parent = 99, not claim-digest 1
    let ok = poke_verify_chain_link(&state, "phase3", &digest(1), &headers, &digest(2)).await;
    assert!(!ok);
}

/// Internal link break: [2<-1, 3<-99] — second header doesn't chain
/// to first.
#[tokio::test]
async fn chain_link_rejects_internal_break() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let headers = vec![anchor_header(2, 2, 1), anchor_header(3, 3, 99)];
    let ok = poke_verify_chain_link(&state, "phase3", &digest(1), &headers, &digest(3)).await;
    assert!(!ok);
}

/// Height gap: [2<-1, 5<-2] — height jumps from 2 to 5 instead of 3.
#[tokio::test]
async fn chain_link_rejects_height_gap() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let headers = vec![anchor_header(2, 2, 1), anchor_header(5, 3, 2)];
    let ok = poke_verify_chain_link(&state, "phase3", &digest(1), &headers, &digest(3)).await;
    assert!(!ok);
}

/// Final digest != anchored tip.
#[tokio::test]
async fn chain_link_rejects_wrong_final_tip() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let headers = vec![anchor_header(2, 2, 1), anchor_header(3, 3, 2)];
    let ok = poke_verify_chain_link(&state, "phase3", &digest(1), &headers, &digest(99)).await;
    assert!(!ok);
}

// =========================================================================
// has-tx-in-page — Phase 3 Level B tx-inclusion predicate (via zoon z-in)
// =========================================================================

// Level B's `has-tx-in-page` is a flat list scan (`has-tx-in-list` in
// Hoon) — no `z-silt` / `gor-tip` / Tip5 on the membership path. (An
// older z-set build hit a jet edge case with 3+ 40-byte atoms; see
// commit history / `docs/ROADMAP.md` if you need the archaeology.)

/// Minimal probe: single tx-id, direct-atom-sized values.
#[tokio::test]
async fn tx_in_page_accepts_small_atom_id() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let small: Vec<u8> = vec![0x2a];
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &[small.clone()], &small).await);
    assert!(!poke_verify_tx_in_page(&state, "phase3", &digest(1), &[small.clone()], &vec![0x2b]).await);
}

/// Two small atoms — verifies `z-silt` can build a 2-element tree
/// and membership works for each.
#[tokio::test]
async fn tx_in_page_two_small_atoms() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let a: Vec<u8> = vec![0x01];
    let b: Vec<u8> = vec![0x02];
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &[a.clone(), b.clone()], &a).await);
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &[a.clone(), b.clone()], &b).await);
}

/// 8-byte atoms × 3 — one full Goldilocks felt each, canonical felt
/// encoding. Exercises multiple `put`+`gor-tip` rounds through
/// jetted Tip5.
#[tokio::test]
async fn tx_in_page_eight_byte_atoms() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mk = |n: u64| -> Vec<u8> { n.to_le_bytes().to_vec() };
    let ids = vec![mk(1), mk(2), mk(3)];
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &ids, &mk(2)).await);
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &ids, &mk(1)).await);
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &ids, &mk(3)).await);
}

/// 8-byte atoms × 3 — rejection path.
#[tokio::test]
async fn tx_in_page_eight_byte_rejects_absent() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mk = |n: u64| -> Vec<u8> { n.to_le_bytes().to_vec() };
    let ids = vec![mk(1), mk(2), mk(3)];
    assert!(!poke_verify_tx_in_page(&state, "phase3", &digest(1), &ids, &mk(99)).await);
    assert!(!poke_verify_tx_in_page(&state, "phase3", &digest(1), &ids, &mk(0)).await);
}

/// Empty set — `z-silt ~` returns `~`, and `has:z-in ~` is `%.n`
/// for any key. Confirms the trivial path runs without crashing.
#[tokio::test]
async fn tx_in_page_rejects_empty_set() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let page = digest(42);
    assert!(!poke_verify_tx_in_page(&state, "phase3", &page, &[], &vec![0x01]).await);
}

/// Single 40-byte atom — verifies pure-Hoon or jetted Tip5 handles
/// atoms at the realistic tx-id size when only one insertion happens.
#[tokio::test]
async fn tx_in_page_forty_byte_single() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mk = |seed: u8| -> Vec<u8> {
        let mut v = vec![0u8; 40];
        v[0] = seed;
        v
    };
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &[mk(1)], &mk(1)).await);
    assert!(!poke_verify_tx_in_page(&state, "phase3", &digest(1), &[mk(1)], &mk(2)).await);
}

/// Three 40-byte atoms — membership with realistic tx-id size (the
/// case that used to crash when the kernel built a z-set via `z-silt`).
#[tokio::test]
async fn tx_in_page_forty_byte_three() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mk = |seed: u8| -> Vec<u8> {
        let mut v = vec![0u8; 40];
        v[0] = seed;
        v
    };
    let ids = vec![mk(1), mk(2), mk(3)];
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &ids, &mk(2)).await);
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &ids, &mk(1)).await);
    assert!(!poke_verify_tx_in_page(&state, "phase3", &digest(1), &ids, &mk(99)).await);
}

/// Two 40-byte atoms — regression guard for multi-insert Tip5 paths.
#[tokio::test]
async fn tx_in_page_forty_byte_two() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mk = |seed: u8| -> Vec<u8> {
        let mut v = vec![0u8; 40];
        v[0] = seed;
        v
    };
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &[mk(1), mk(2)], &mk(1)).await);
    assert!(poke_verify_tx_in_page(&state, "phase3", &digest(1), &[mk(1), mk(2)], &mk(2)).await);
    assert!(!poke_verify_tx_in_page(&state, "phase3", &digest(1), &[mk(1), mk(2)], &mk(3)).await);
}

// =========================================================================
// validate-claim-bundle — Phase 3c gate validator
// =========================================================================

#[tokio::test]
async fn validate_claim_happy_path() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let bundle = good_claim_bundle();
    assert_eq!(poke_validate_claim(&state, "phase3", &bundle).await, ValidateClaimResult::Ok);
}

#[tokio::test]
async fn validate_claim_rejects_invalid_name() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;

    // Missing `.nock` suffix.
    let mut b = good_claim_bundle();
    b.name = "plain-name".to_string();
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("invalid-name".into())
    );

    // Uppercase — invalid-char.
    let mut b = good_claim_bundle();
    b.name = "Ab.nock".to_string();
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("invalid-name".into())
    );

    // Empty stem.
    let mut b = good_claim_bundle();
    b.name = ".nock".to_string();
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("invalid-name".into())
    );
}

#[tokio::test]
async fn validate_claim_rejects_fee_below_schedule() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // 2-char stem requires 327_680_000; send 327_679_999.
    b.fee = 327_679_999;
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("fee-below-schedule".into())
    );
}

#[tokio::test]
async fn validate_claim_accepts_fee_above_schedule() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // Overpaying is fine (the gate only checks fee >= fee-for-name for ab.nock:
    // 327_680_000 nicks). Witness amount must follow (Level C underpaid check).
    b.fee = 400_000_000;
    b.witness.treasury_amount = 400_000_000;
    assert_eq!(poke_validate_claim(&state, "phase3", &b).await, ValidateClaimResult::Ok);
}

#[tokio::test]
async fn validate_claim_rejects_page_digest_mismatch() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // claim-block-digest says block=0x42 but page.digest says block=0x99.
    b.claim_block_digest = vec![0x99];
    b.anchored_tip = vec![0x99];
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("page-digest-mismatch".into())
    );
}

#[tokio::test]
async fn validate_claim_rejects_tx_not_in_page() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // Claim references tx_hash=0x07, but page.tx-ids doesn't contain it.
    b.page_tx_ids = vec![vec![0x08], vec![0x09]];
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("tx-not-in-page".into())
    );
}

#[tokio::test]
async fn validate_claim_rejects_chain_broken() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // Claim-block-digest (0x42) doesn't match anchored-tip (0x99) and
    // no headers link them.
    b.anchored_tip = vec![0x99];
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("chain-broken".into())
    );
}

#[tokio::test]
async fn validate_claim_accepts_chain_with_headers() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // Claim block 0x42 at some past height. Anchor tip is 0x44.
    // Chain: claim (0x42, height=10) <- 0x43 (height=11) <- 0x44 (height=12).
    b.claim_block_digest = vec![0x42];
    b.page_digest = vec![0x42];
    b.anchor_headers = vec![
        AnchorHeader {
            digest: vec![0x43],
            height: 11,
            parent: vec![0x42],
        },
        AnchorHeader {
            digest: vec![0x44],
            height: 12,
            parent: vec![0x43],
        },
    ];
    b.anchored_tip = vec![0x44];
    assert_eq!(poke_validate_claim(&state, "phase3", &b).await, ValidateClaimResult::Ok);
}

#[tokio::test]
async fn validate_claim_rejects_broken_header_chain() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // Headers don't chain — second header's parent should be 0x43
    // but is 0x99.
    b.claim_block_digest = vec![0x42];
    b.page_digest = vec![0x42];
    b.anchor_headers = vec![
        AnchorHeader {
            digest: vec![0x43],
            height: 11,
            parent: vec![0x42],
        },
        AnchorHeader {
            digest: vec![0x44],
            height: 12,
            parent: vec![0x99],
        },
    ];
    b.anchored_tip = vec![0x44];
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("chain-broken".into())
    );
}

#[tokio::test]
async fn validate_claim_short_circuits_on_first_error() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    // Deliberately construct a bundle that fails MULTIPLE predicates.
    // The validator should report the first one (invalid-name) and
    // not continue.
    let mut b = good_claim_bundle();
    b.name = "BAD".to_string(); // G1 fails
    b.fee = 0; // C2 would also fail
    b.claim_block_digest = vec![0x99]; // page-digest-mismatch would also fail
    b.page_tx_ids = vec![]; // tx-not-in-page would also fail
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("invalid-name".into())
    );
}

// Level C-A: payment-semantic witness predicates
// ----------------------------------------------------------------------
//
// Four bundle-internal predicates are asserted by validate-claim-bundle:
//   - matches-tx-id        → witness.tx-id == claim.tx-hash
//   - pays-sender          → witness.spender-pkh == claim.owner
//   - pays-amount          → witness.treasury-amount >= fee-for-name
// The fourth (matches-treasury) compares witness.output_lock_root to the
// v1 lock-root b58 derived from the kernel p2pkh (not the p2pkh string
// itself) — see `validate_claim_rejects_wrong_treasury`.

#[tokio::test]
async fn validate_claim_rejects_witness_tx_id_mismatch() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // Hull claims payment tx 0x07 but witness says 0x99 — smells like
    // the hull swapped one user's tx-id for another's. Reject.
    b.witness.tx_id = vec![0x99];
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("witness-tx-id-mismatch".into())
    );
}

#[tokio::test]
async fn validate_claim_rejects_witness_sender_mismatch() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // Witness says a different pkh paid than the claim's owner field.
    // Catches hostile hull redirecting someone else's payment to a
    // fresh owner string.
    b.witness.spender_pkh = b"not-owner".to_vec();
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("witness-sender-mismatch".into())
    );
}

#[tokio::test]
async fn validate_claim_rejects_witness_underpaid() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // Hull declared fee=327_680_000 (passes C2) but the actual on-chain
    // treasury-amount was only 4999. The witness-underpaid check is
    // stricter than C2: C2 trusts the hull's claim.fee, this trusts
    // what actually moved on chain (as reported by the hull, then
    // verified by the wallet externally).
    b.witness.treasury_amount = 4_999;
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("witness-underpaid".into())
    );
}

#[tokio::test]
async fn validate_claim_rejects_witness_underpaid_even_when_claim_fee_matches() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // Hull lies: claim.fee=327_680_000 passes C2, but actual treasury received 0.
    // Witness-underpaid is the belt-and-suspenders check that closes
    // the "hostile hull lies about fee" gap.
    b.fee = 327_680_000;
    b.witness.treasury_amount = 0;
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("witness-underpaid".into())
    );
}

// --- matches-treasury (canonical lock root) via %validate-claim ------

#[tokio::test]
async fn validate_claim_rejects_wrong_treasury() {
    let (_tmp, state) = boot_kernel(BOOT_PHASE3).await;
    let mut b = good_claim_bundle();
    // Bundle claims payment was sent to a *different* lock root than the
    // canonical NNS treasury.
    b.witness.output_lock_root = "4uhcJHPZN6759D8ukUopNpVNPG3ho18pYjksyS81NLXo".to_string();
    assert_eq!(
        poke_validate_claim(&state, "phase3", &b).await,
        ValidateClaimResult::Error("witness-wrong-treasury".into())
    );
}
