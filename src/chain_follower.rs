use std::collections::HashSet;
use std::time::Duration;

use nockapp::wire::{SystemWire, Wire};
use nockapp_grpc::pb::common::v2::NoteData as PbNoteData;
use nockapp_grpc::pb::public::v2::TransactionDetails;
use nockchain_client_rs::{NoteData, NoteDataEntry};
use tokio::task::JoinHandle;

use crate::chain::{
    base58_hash_to_atom_bytes, canonical_z_set_tx_order, fetch_current_tip_height,
    prefetch_scan_blocks_for_heights, scan_cursor_is_genesis_boot,
    sort_claims_by_z_set_tx_order, validate_scan_block_chain,
};
use crate::claim_note::ClaimNoteV1;
use crate::kernel::{
    build_prove_recursive_genesis_poke, build_prove_recursive_transition_poke,
    build_recursive_proof_peek, build_scan_block_poke, build_scan_state_peek,
    decode_recursive_proof, decode_scan_state, first_error_message,
    first_genesis_recursive_proof, first_recursive_transition_proof,
    first_scan_block_done, first_scan_block_error, format_effect_tags, has_effect,
    ClaimCandidate, ClaimWitness,
};
use crate::payment::{fee_for_name, sum_treasury_outputs_v1, TREASURY_LOCK_ROOT_B58};
use crate::state::SharedState;

/// Hex prefix for Tip5 atoms in logs (`RUST_LOG=nns_vesl::chain_follower=debug`).
fn atom_hex_preview(bytes: &[u8], prefix_len: usize) -> String {
    let n = prefix_len.min(bytes.len());
    let hex: String = bytes[..n].iter().map(|b| format!("{b:02x}")).collect();
    if bytes.len() > n {
        format!("{hex}…({}B)", bytes.len())
    } else {
        format!("{hex}({}B)", bytes.len())
    }
}

fn format_tx_id_previews(ids: &[Vec<u8>], max: usize) -> String {
    let mut out = Vec::new();
    for id in ids.iter().take(max) {
        out.push(atom_hex_preview(id, 8));
    }
    let suffix = if ids.len() > max {
        format!(" …(+{} more)", ids.len() - max)
    } else {
        String::new()
    };
    format!("[{}]{suffix}", out.join(", "))
}

fn format_claims_for_log(claims: &[ClaimCandidate]) -> String {
    const MAX: usize = 12;
    let mut parts = Vec::new();
    for c in claims.iter().take(MAX) {
        parts.push(format!(
            "(name={:?} owner={:?} fee={} tx_hash={} wit.tx={} wit.spender={} wit.amt={} wit.treas={:?})",
            c.name,
            c.owner,
            c.fee,
            atom_hex_preview(&c.tx_hash, 6),
            atom_hex_preview(&c.witness.tx_id, 6),
            atom_hex_preview(&c.witness.spender_pkh, 6),
            c.witness.treasury_amount,
            c.witness.output_lock_root,
        ));
    }
    if claims.len() > MAX {
        parts.push(format!("…(+{} more claims)", claims.len() - MAX));
    }
    parts.join(" ")
}

/// Sleep between ticks **only** when there is nothing to scan (caught up
/// within finality) or after an error — avoids gRPC busy-loops. While a
/// finalized backlog exists, consecutive `%scan-block` steps run back-to-back.
const FOLLOWER_POLL: Duration = Duration::from_secs(10);

/// When the block endpoint returns **429 / 5xx** (rate limit, bad gateway, etc.),
/// wait longer so we do not hammer a stressed proxy. The gRPC client often
/// reports a misleading *"invalid compression flag: 101"* in that case: the
/// HTTP body is usually HTML/JSON, not a valid gRPC frame; the first byte
/// (e.g. `e` from *error* / HTML) is misread as a compression byte.
const FOLLOWER_POLL_STRESSED: Duration = Duration::from_secs(60);

fn is_upstream_stressed(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("429")
        || e.contains("too many requests")
        || e.contains("502")
        || e.contains("bad gateway")
        || e.contains("503")
        || e.contains("service unavailable")
        || e.contains("504")
        || e.contains("gateway timeout")
}

/// How far behind the chain tip the follower waits before committing a
/// block to the kernel scan cursor. Keeps Path Y scans free of short
/// reorgs without waiting on economic finality.
pub const DEFAULT_FINALITY_DEPTH: u64 = 10;

/// Transitional compatibility for status/admin JSON while the API is renamed
/// from "anchor advance" to "block scan".
pub const DEFAULT_MAX_ADVANCE_BATCH: u64 = 1;

/// How many consecutive finalized blocks to prefetch and `%scan-block` apply
/// per idle tick when catching up. Override with `NNS_FOLLOWER_BATCH_BLOCKS`.
pub const DEFAULT_SCAN_BATCH_BLOCKS: u64 = 32;

fn follower_scan_batch_blocks() -> u64 {
    match std::env::var("NNS_FOLLOWER_BATCH_BLOCKS") {
        Ok(s) => match s.parse::<u64>() {
            Ok(n) if n >= 1 => n,
            _ => DEFAULT_SCAN_BATCH_BLOCKS,
        },
        Err(_) => DEFAULT_SCAN_BATCH_BLOCKS,
    }
}

/// Spawn the Path Y block scanner. It advances the kernel with `%scan-block`,
/// prefetching a batch of blocks in parallel when behind finality.
pub fn spawn(state: SharedState) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let (idle, stressed) = match scan_once(&state).await {
                Ok(Some(scanned)) => {
                    tracing::info!(
                        height_end = scanned.height,
                        blocks = scanned.blocks_applied,
                        phase = "scan_block",
                        "chain follower scanned blocks"
                    );
                    (false, false)
                }
                Ok(None) => {
                    tracing::trace!(phase = "scan_block", "scan tick no-op");
                    (true, false)
                }
                Err(err) => {
                    let ts = crate::state::AppState::now_epoch_ms();
                    let mut h = state.hull.lock().await;
                    h.follower.record_error("scan_block", err.clone(), ts);
                    drop(h);
                    let stressed = is_upstream_stressed(&err);
                    tracing::warn!(
                        err = %err,
                        phase = "scan_block",
                        stressed,
                        "chain follower scan tick failed"
                    );
                    (true, stressed)
                }
            };
            if idle {
                let wait = if stressed {
                    FOLLOWER_POLL_STRESSED
                } else {
                    FOLLOWER_POLL
                };
                tokio::time::sleep(wait).await;
            } else {
                tokio::task::yield_now().await;
            }
        }
    })
}

pub async fn process_once(state: &SharedState) -> Result<(), String> {
    scan_once(state).await.map(|_| ())
}

/// Outcome of one block-scan pass (last block in the batch).
#[derive(Debug, Clone)]
pub struct ScanBlockOutcome {
    pub height: u64,
    pub digest: Vec<u8>,
    pub accumulator_root: Vec<u8>,
    /// Finalized blocks applied this tick (`1` when only one `%scan-block` ran).
    pub blocks_applied: u64,
}

/// Backwards-compatible shape used by the existing admin handler.
#[derive(Debug, Clone)]
pub struct AnchorAdvanceOutcome {
    pub tip_height: u64,
    pub tip_digest: Vec<u8>,
    pub count: u64,
}


/// One pass of the Path Y block scanner. Returns `Ok(None)` if there is
/// no finalized block beyond the kernel's current `/scan-state`.
pub async fn scan_once(state: &SharedState) -> Result<Option<ScanBlockOutcome>, String> {
    let (is_local_mode, chain_endpoint) = {
        let h = state.hull.lock().await;
        (
            matches!(h.settlement.mode, vesl_core::SettlementMode::Local),
            h.settlement.chain_endpoint.clone(),
        )
    };
    if is_local_mode {
        return Ok(None);
    }
    let Some(endpoint) = chain_endpoint else {
        return Ok(None);
    };

    let scan_state = {
        let peek_result = {
            let mut k = state.kernel.lock().await;
            k.peek(build_scan_state_peek()).await
        };
        match peek_result {
            Ok(result) => {
                decode_scan_state(&result).map_err(|e| format!("scan-state decode failed: {e}"))?
            }
            Err(e) => {
                let msg = format!("scan-state peek failed: {e:?}");
                let ts = crate::state::AppState::now_epoch_ms();
                let mut h = state.hull.lock().await;
                h.follower.record_error("scan_peek", msg.clone(), ts);
                return Err(msg);
            }
        }
    };

    tracing::debug!(
        last_proved_height = scan_state.last_proved_height,
        last_proved_digest = %atom_hex_preview(&scan_state.last_proved_digest, 16),
        accumulator_root = %atom_hex_preview(&scan_state.accumulator_root, 16),
        accumulator_size = scan_state.accumulator_size,
        "chain_follower: /scan-state peek"
    );

    // Y3: If we are still at the genesis cursor, ensure we have the base-case
    // recursive proof before we start scanning blocks. This proof becomes
    // the root that all future incremental recursive proofs will chain from.
    if scan_cursor_is_genesis_boot(
        scan_state.last_proved_height,
        scan_state.last_proved_digest.as_slice(),
    ) {
        tracing::info!("chain_follower: genesis cursor detected — producing base-case recursive proof");
        let efx = {
            let mut k = state.kernel.lock().await;
            k.poke(
                SystemWire.to_wire(),
                build_prove_recursive_genesis_poke(),
            )
            .await
            .map_err(|e| format!("genesis recursive prove poke failed: {e:?}"))?
        };
        if let Some(proof) = first_genesis_recursive_proof(&efx) {
            tracing::info!(
                proof_bytes = proof.len(),
                "chain_follower: genesis recursive proof produced and stored in kernel state"
            );
        } else if let Some(_fail) = first_error_message(&efx) {
            // Non-fatal for now — we can still run in Y2 mode until the prover
            // is ready for the base case.
            tracing::warn!("chain_follower: genesis recursive prove did not succeed (prover may trap until upstream opcodes land)");
        }
    }

    let current_chain_tip = match fetch_current_tip_height(&endpoint).await {
        Ok(p) => p,
        Err(e) => {
            let ts = crate::state::AppState::now_epoch_ms();
            let mut h = state.hull.lock().await;
            h.follower.record_error("plan", e.clone(), ts);
            return Err(e);
        }
    };

    let now = crate::state::AppState::now_epoch_ms();
    {
        let mut h = state.hull.lock().await;
        h.follower.record_chain_tip(current_chain_tip, now);
    }

    if current_chain_tip <= DEFAULT_FINALITY_DEPTH {
        return Ok(None);
    }
    let finalized_height = current_chain_tip.saturating_sub(DEFAULT_FINALITY_DEPTH);
    let scan_first = {
        let h = state.hull.lock().await;
        h.nns_genesis_height
    };
    let next_height = if scan_cursor_is_genesis_boot(
        scan_state.last_proved_height,
        scan_state.last_proved_digest.as_slice(),
    ) {
        scan_first.max(scan_state.last_proved_height.saturating_add(1))
    } else {
        scan_state.last_proved_height.saturating_add(1)
    };
    if next_height > finalized_height {
        return Ok(None);
    }

    let batch_max = follower_scan_batch_blocks();
    let batch_end = next_height
        .saturating_add(batch_max.saturating_sub(1))
        .min(finalized_height);
    let heights: Vec<u64> = (next_height..=batch_end).collect();

    let prefetched = prefetch_scan_blocks_for_heights(&endpoint, &heights).await?;

    let batch_lo = prefetched
        .first()
        .map(|b| b.height)
        .unwrap_or(next_height);
    let batch_end = prefetched
        .last()
        .map(|b| b.height)
        .unwrap_or(next_height);

    apply_prefetched_scan_blocks_inner(
        state,
        prefetched,
        Some(FollowerScanTrace {
            chain_tip: current_chain_tip,
            finalized_height,
            batch_lo,
            batch_end,
        }),
        None,
    )
    .await
}

/// Trace context for follower logs (optional for integration tests).
pub(crate) struct FollowerScanTrace {
    pub chain_tip: u64,
    pub finalized_height: u64,
    pub batch_lo: u64,
    pub batch_end: u64,
}

/// Apply prefetched blocks in ascending height order with `%scan-block` pokes.
///
/// The first block's `parent` must match the kernel's current `last_proved_digest`
/// (unless still at genesis boot), and each subsequent block must link to the
/// previous block's page digest.
///
/// Used by [`scan_once`] and integration tests that replay RPC-backed
/// [`crate::chain::ScanBlockFetch`] batches without starting the follower loop.
pub async fn apply_prefetched_scan_blocks(
    state: &SharedState,
    prefetched: Vec<crate::chain::ScanBlockFetch>,
) -> Result<Option<ScanBlockOutcome>, String> {
    apply_prefetched_scan_blocks_inner(state, prefetched, None, None).await
}

/// Like [`apply_prefetched_scan_blocks`], but supplies explicit claim claims per block
/// (skips gRPC-shaped transaction extraction). Used by integration tests that construct
/// canonical `%scan-block` payloads directly (e.g. minimal `@ux` tx-id atoms).
pub async fn apply_prefetched_scan_blocks_with_claims(
    state: &SharedState,
    prefetched: Vec<crate::chain::ScanBlockFetch>,
    claims_per_block: Vec<Vec<ClaimCandidate>>,
) -> Result<Option<ScanBlockOutcome>, String> {
    if prefetched.len() != claims_per_block.len() {
        return Err(format!(
            "claims_per_block length {} does not match prefetched blocks {}",
            claims_per_block.len(),
            prefetched.len()
        ));
    }
    apply_prefetched_scan_blocks_inner(state, prefetched, None, Some(claims_per_block)).await
}

async fn apply_prefetched_scan_blocks_inner(
    state: &SharedState,
    prefetched: Vec<crate::chain::ScanBlockFetch>,
    trace: Option<FollowerScanTrace>,
    injected_claims: Option<Vec<Vec<ClaimCandidate>>>,
) -> Result<Option<ScanBlockOutcome>, String> {
    if prefetched.is_empty() {
        return Ok(None);
    }

    let scan_state = {
        let peek_result = {
            let mut k = state.kernel.lock().await;
            k.peek(build_scan_state_peek()).await
        };
        match peek_result {
            Ok(result) => {
                decode_scan_state(&result).map_err(|e| format!("scan-state decode failed: {e}"))?
            }
            Err(e) => {
                let msg = format!("scan-state peek failed: {e:?}");
                let ts = crate::state::AppState::now_epoch_ms();
                let mut h = state.hull.lock().await;
                h.follower.record_error("scan_peek", msg.clone(), ts);
                return Err(msg);
            }
        }
    };

    validate_scan_block_chain(
        scan_state.last_proved_height,
        scan_state.last_proved_digest.as_slice(),
        &prefetched,
    )?;

    let blocks_applied = prefetched.len() as u64;
    let mut last_done = None;

    for (idx, block) in prefetched.iter().enumerate() {
        let page_tx_for_poke = canonical_z_set_tx_order(block.page_tx_ids.clone())
            .map_err(|e| format!("canonical tx-id order: {e}"))?;

        let claims = if let Some(ref inj) = injected_claims {
            sort_claims_by_z_set_tx_order(
                &page_tx_for_poke,
                inj.get(idx)
                    .cloned()
                    .ok_or_else(|| format!("injected claims missing for block index {idx}"))?,
            )
            .map_err(|e| format!("sort injected scan claims: {e}"))?
        } else {
            extract_claims(&block.tx_details)?
        };

        if let Some(t) = trace.as_ref() {
            tracing::debug!(
                chain_tip = t.chain_tip,
                finalized_height = t.finalized_height,
                batch_lo = t.batch_lo,
                batch_end = t.batch_end,
                height = block.height,
                parent = %atom_hex_preview(&block.parent, 16),
                page_digest = %atom_hex_preview(&block.page_digest, 16),
                page_tx_count = page_tx_for_poke.len(),
                page_tx_ids_preview = %format_tx_id_previews(&page_tx_for_poke, 8),
                claims_count = claims.len(),
                claims = %format_claims_for_log(&claims),
                "chain_follower: %scan-block poke payload (Tip5 atoms are LE 40B; compare with kernel last_proved_digest)"
            );
        } else {
            tracing::debug!(
                height = block.height,
                parent = %atom_hex_preview(&block.parent, 16),
                page_digest = %atom_hex_preview(&block.page_digest, 16),
                claims_count = claims.len(),
                "chain_follower (test harness): %scan-block poke"
            );
        }

        let poke_result = {
            let mut k = state.kernel.lock().await;
            k.poke(
                SystemWire.to_wire(),
                build_scan_block_poke(
                    &block.parent,
                    block.height,
                    &block.page_digest,
                    &page_tx_for_poke,
                    &claims,
                ),
            )
            .await
        };

        let effects = poke_result.map_err(|e| {
            let msg = format!("scan-block poke failed: {e:?}");
            let ts = crate::state::AppState::now_epoch_ms();
            if let Ok(mut h) = state.hull.try_lock() {
                h.follower.record_error("scan_poke", msg.clone(), ts);
            }
            msg
        })?;

        if let Some(err) = first_scan_block_error(&effects) {
            let msg = format!("kernel rejected %scan-block: {err}");
            let ts = crate::state::AppState::now_epoch_ms();
            let mut h = state.hull.lock().await;
            h.follower.record_error("scan_poke", msg.clone(), ts);
            return Err(msg);
        }
        if let Some(err) = first_error_message(&effects) {
            let msg = format!("kernel rejected %scan-block: {err}");
            let ts = crate::state::AppState::now_epoch_ms();
            let mut h = state.hull.lock().await;
            h.follower.record_error("scan_poke", msg.clone(), ts);
            return Err(msg);
        }
        if let Some(done) = first_scan_block_done(&effects) {
            // === Success path for this block ===
            // Y3: attempt recursive transition for the block we just successfully scanned
            try_y3_recursive_transition_after_block(state, &block, &claims).await;

            state.maybe_persist_after_follower_scan().await;

            tracing::debug!(
                height = done.height,
                block_digest = %atom_hex_preview(&done.digest, 16),
                accumulator_root = %atom_hex_preview(&done.accumulator_root, 16),
                "chain_follower: %scan-block-done"
            );

            last_done = Some(done);
        } else {
            let tags = format_effect_tags(&effects);
            let msg = if has_effect(&effects, "invalid-cause") {
                "%invalid-cause from kernel — `(soft cause)` failed (mold mismatch). \
                 Rebuild nns.jam from current hoon/app/app.hoon so `+$cause` includes `%scan-block`, \
                 point NNS_KERNEL_JAM at it, redeploy."
                    .to_string()
            } else if effects.is_empty() {
                "kernel did not emit %scan-block-done (empty effects — wrapper/nockapp returned no effects; \
                 if stderr shows `nns: invalid cause`, rebuild nns.jam; otherwise check nockapp poke wiring)"
                    .to_string()
            } else {
                format!(
                    "kernel did not emit %scan-block-done (effect tags: {tags}; check kernel JAM vs hull or scan-block-done noun shape)"
                )
            };
            let ts = crate::state::AppState::now_epoch_ms();
            let mut h = state.hull.lock().await;
            h.follower.record_error("scan_poke", msg.clone(), ts);
            return Err(msg);
        }
    }

    let Some(done) = last_done else {
        return Err("prefetch produced no blocks (internal error)".into());
    };

    let now = crate::state::AppState::now_epoch_ms();
    if trace.is_some() {
        let mut h = state.hull.lock().await;
        h.follower.record_advance(done.height, blocks_applied, now);
    }

    Ok(Some(ScanBlockOutcome {
        height: done.height,
        digest: done.digest,
        accumulator_root: done.accumulator_root,
        blocks_applied,
    }))
}

/// Extract NNS claim claims from one prefetched block (for tests / tooling).
///
/// **`ScanBlockFetch` from [`crate::chain::fetch_scan_block_inputs`]** already has
/// tx rows in z-set canonical order. For hand-built stubs, call
/// [`crate::chain::sort_claims_by_z_set_tx_order`] if needed.
pub fn claims_from_fetch(block: &crate::chain::ScanBlockFetch) -> Result<Vec<crate::kernel::ClaimCandidate>, String> {
    extract_claims(&block.tx_details)
}

fn extract_claims(details: &[TransactionDetails]) -> Result<Vec<ClaimCandidate>, String> {
    let mut claims = Vec::new();
    for tx in details {
        claims.extend(extract_claims_from_transaction(tx)?);
    }
    Ok(claims)
}

fn extract_claims_from_transaction(
    details: &TransactionDetails,
) -> Result<Vec<ClaimCandidate>, String> {
    let mut claims = Vec::new();
    for output in &details.outputs {
        let Some(note_data) = output.note_data.as_ref() else {
            continue;
        };
        let note_data = note_data_from_proto(note_data);
        let Ok(note) = ClaimNoteV1::from_note_data(&note_data) else {
            continue;
        };
        let tx_id_expected = details.tx_id.trim();
        let note_tx = note.tx_hash.trim();
        let effective_tx_id = if note_tx.is_empty() {
            tx_id_expected
        } else {
            note_tx
        };
        if effective_tx_id != tx_id_expected {
            tracing::warn!(
                tx_id = %tx_id_expected,
                tx_hash_in_blob = %note_tx,
                name = %note.name,
                "nns: claim blob tx_hash does not match this transaction id; \
                 skipping (kernel matches-tx-id)"
            );
            continue;
        }
        let tx_hash = base58_hash_to_atom_bytes(effective_tx_id)?;
        let actual_tx_hash = base58_hash_to_atom_bytes(tx_id_expected)?;
        let Some(owner) = infer_claim_owner_for_claim(details, &note) else {
            tracing::warn!(
                tx_id = %tx_id_expected,
                name = %note.name,
                owner_in_blob = %note.owner,
                input_count = details.inputs.len(),
                "nns: cannot infer claim owner (path-style blob needs a unique signer pubkey or \
                 legacy spent-note name); skipping (kernel sender-is-owner)"
            );
            continue;
        };
        let Some(witness) = claim_witness_from_transaction(details, &actual_tx_hash, &owner) else {
            tracing::warn!(
                tx_id = %tx_id_expected,
                name = %note.name,
                owner_in_blob = %owner,
                input_count = details.inputs.len(),
                "nns: claim owner is not a tx signer pubkey (GetTransactionDetails \
                 `signer_pubkey_b58`) nor a legacy spent-note name match; skipping (kernel sender-is-owner)"
            );
            continue;
        };
        let min_fee = fee_for_name(&note.name);
        if witness.treasury_amount < min_fee {
            tracing::warn!(
                tx_id = %details.tx_id.trim(),
                name = %note.name,
                treasury_nicks = witness.treasury_amount,
                min_fee_nicks = min_fee,
                "nns: claim claim treasury sum below fee schedule (lock decode / \
                 output filter may not match this tx; kernel would reject witness-underpaid)"
            );
        }
        claims.push(ClaimCandidate {
            fee: min_fee,
            name: note.name,
            owner,
            tx_hash,
            witness,
        });
    }
    Ok(claims)
}

/// When the on-chain `blob` is a **path** (`nns/v1/claim/<name>.nock`), `owner` is empty in
/// [`ClaimNoteV1`]. Resolve it from `signer_pubkey_b58` (unique across inputs), or legacy
/// distinct `note_name_b58` when signer lists are empty.
fn infer_claim_owner_for_claim(details: &TransactionDetails, note: &ClaimNoteV1) -> Option<String> {
    let o = note.owner.trim();
    if !o.is_empty() {
        return Some(o.to_string());
    }
    let signers: HashSet<&str> = details
        .inputs
        .iter()
        .flat_map(|i| i.signer_pubkey_b58.iter().map(|s| s.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if signers.len() == 1 {
        return Some(signers.into_iter().next()?.to_string());
    }
    if !signers.is_empty() {
        return None;
    }
    let names: HashSet<&str> = details
        .inputs
        .iter()
        .map(|i| i.note_name_b58.as_str().trim())
        .filter(|s| !s.is_empty())
        .collect();
    if names.len() == 1 {
        return Some(names.into_iter().next()?.to_string());
    }
    None
}

/// Build Level C-A witness fields from RPC `TransactionDetails`.
///
/// `claim.owner` must identify the **signer**: it is checked against every input's
/// `signer_pubkey_b58` (Schnorr pubkey base58 from the v1 spend witness, populated by
/// nockchain `GetTransactionDetails`). That matches Nockblocks-style `pkhSignature.pubkey`.
///
/// If the chain node has not been upgraded and all `signer_pubkey_b58` lists are empty,
/// we fall back to the legacy rule: `owner` must equal some input's `note_name_b58`
/// (spent-note name), which older NNS docs assumed.
///
/// `spender_pkh` in the witness is always the UTF-8 bytes of `owner`, which is what the
/// kernel's `sender-is-owner` predicate compares to `claim.owner` as `@`.
fn claim_witness_from_transaction(
    details: &TransactionDetails,
    tx_hash: &[u8],
    owner: &str,
) -> Option<ClaimWitness> {
    let owner = owner.trim();
    if owner.is_empty() {
        return None;
    }

    let signer_set: HashSet<&str> = details
        .inputs
        .iter()
        .flat_map(|i| i.signer_pubkey_b58.iter().map(|s| s.as_str()))
        .collect();

    let owner_ok = if signer_set.is_empty() {
        details
            .inputs
            .iter()
            .any(|i| i.note_name_b58.trim() == owner)
    } else {
        signer_set.contains(owner)
    };

    if !owner_ok {
        return None;
    }

    Some(ClaimWitness {
        tx_id: tx_hash.to_vec(),
        spender_pkh: owner.as_bytes().to_vec(),
        treasury_amount: sum_treasury_outputs_v1(details),
        output_lock_root: TREASURY_LOCK_ROOT_B58.to_string(),
    })
}

fn note_data_from_proto(data: &PbNoteData) -> NoteData {
    NoteData::new(
        data.entries
            .iter()
            .map(|entry| NoteDataEntry::new(entry.key.clone(), entry.blob.clone().into()))
            .collect(),
    )
}

/// Y3 helper: after a successful %scan-block for `block` with the given `claims`,
/// attempt to produce the recursive transition proof.
///
/// This is called from the success path inside `apply_prefetched_scan_blocks_inner`.
/// It is best-effort (the transition may fail while the formula is still being completed).
async fn try_y3_recursive_transition_after_block(
    state: &SharedState,
    block: &crate::chain::ScanBlockFetch,
    claims: &[ClaimCandidate],
) {
    // Peek the recursive proof that existed *before* this scan block
    let prev_triple = {
        let mut k = state.kernel.lock().await;
        k.peek(build_recursive_proof_peek())
            .await
            .ok()
            .and_then(|r| decode_recursive_proof(&r).ok().flatten())
    };

    let Some((prev_proof, prev_subj, prev_form)) = prev_triple else {
        return; // No previous recursive proof yet (still at genesis or first block)
    };

    let tx_ids: Vec<Vec<u8>> = block.page_tx_ids.iter().map(|id| id.to_vec()).collect();

    let transition_poke = build_prove_recursive_transition_poke(
        &prev_proof,
        &prev_subj,
        &prev_form,
        &block.page_digest,
        &tx_ids,
        claims,
        &[], // TODO: pass the real Nockchain block sp proof for verify:sp-verifier
    );

    if let Ok(efx) = {
        let mut k = state.kernel.lock().await;
        k.poke(SystemWire.to_wire(), transition_poke).await
    } {
        if first_recursive_transition_proof(&efx).is_some() {
            tracing::info!(height = block.height, "Y3: recursive transition proof produced");
        } else if let Some(msg) = first_error_message(&efx) {
            tracing::debug!(
                height = block.height,
                error = %msg,
                "Y3 transition prove (expected while formula is incomplete)"
            );
        }
    }
}
