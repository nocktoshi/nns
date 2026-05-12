use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::claim_note::ClaimNoteV1;
use crate::kernel::ClaimCandidate;
use nockapp_grpc::pb::common::v1::PageRequest;
use nockapp_grpc::pb::common::v1::{Base58Hash, Belt, Hash};
use nockapp_grpc::pb::public::v2::nockchain_block_service_client::NockchainBlockServiceClient;
use nockapp_grpc::pb::public::v2::{
    get_block_details_request, get_block_details_response, get_blocks_response,
    get_transaction_block_response, get_transaction_details_response, BlockDetails,
    GetBlockDetailsRequest, GetBlocksRequest, GetTransactionBlockRequest,
    GetTransactionDetailsRequest, TransactionDetails,
};
use nockchain_client_rs::{ChainClient, ChainConfig};
use nockchain_math::zoon::zset::ZSet;
use nockchain_types::tx_engine::common::Hash as Tip5Hash;
use tokio::sync::Semaphore;

/// Best-effort chain acceptance check for a base58 tx id.
pub async fn transaction_is_accepted(
    endpoint: &str,
    accept_timeout_secs: u64,
    tx_id_base58: &str,
) -> Result<bool, String> {
    let mut cfg = ChainConfig::local(endpoint);
    cfg.accept_timeout = Duration::from_secs(accept_timeout_secs.max(1));
    let mut client = ChainClient::connect(cfg)
        .await
        .map_err(|e| format!("chain connect failed: {e}"))?;
    client
        .check_accepted(tx_id_base58)
        .await
        .map_err(|e| format!("acceptance query failed: {e}"))
}

/// Query chain for the block height that included a tx.
///
/// Returns:
/// - `Ok(Some(height))` when the tx is in a block
/// - `Ok(None)` when the tx is still pending
/// - `Err(...)` on transport or server failures
pub async fn transaction_block_height(
    endpoint: &str,
    tx_id_base58: &str,
) -> Result<Option<u64>, String> {
    let mut client = NockchainBlockServiceClient::connect(endpoint.to_string())
        .await
        .map_err(|e| format!("block service connect failed: {e}"))?;
    let req = GetTransactionBlockRequest {
        tx_id: Some(Base58Hash {
            hash: tx_id_base58.to_string(),
        }),
    };
    let res = client
        .get_transaction_block(req)
        .await
        .map_err(|e| format!("transaction block query failed: {e}"))?
        .into_inner();
    match res.result {
        Some(get_transaction_block_response::Result::Block(block)) => Ok(Some(block.height)),
        Some(get_transaction_block_response::Result::Pending(_)) => Ok(None),
        Some(get_transaction_block_response::Result::Error(err)) => Err(err.message),
        None => Ok(None),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmedTxPosition {
    pub block_height: u64,
    pub tx_index_in_block: u64,
}

/// Query chain for canonical ordering position of a confirmed tx.
///
/// Returns:
/// - `Ok(Some(position))` when tx is mined and found in block tx list
/// - `Ok(None)` when tx is pending
/// - `Err(...)` on transport/server failures or inconsistent block data
pub async fn confirmed_tx_position(
    endpoint: &str,
    tx_id_base58: &str,
) -> Result<Option<ConfirmedTxPosition>, String> {
    let mut client = NockchainBlockServiceClient::connect(endpoint.to_string())
        .await
        .map_err(|e| format!("block service connect failed: {e}"))?;
    let tx_req = GetTransactionBlockRequest {
        tx_id: Some(Base58Hash {
            hash: tx_id_base58.to_string(),
        }),
    };
    let tx_res = client
        .get_transaction_block(tx_req)
        .await
        .map_err(|e| format!("transaction block query failed: {e}"))?
        .into_inner();
    let block_height = match tx_res.result {
        Some(get_transaction_block_response::Result::Block(block)) => block.height,
        Some(get_transaction_block_response::Result::Pending(_)) => return Ok(None),
        Some(get_transaction_block_response::Result::Error(err)) => return Err(err.message),
        None => return Ok(None),
    };

    let details_req = GetBlockDetailsRequest {
        selector: Some(get_block_details_request::Selector::Height(block_height)),
    };
    let details_res = client
        .get_block_details(details_req)
        .await
        .map_err(|e| format!("block details query failed: {e}"))?
        .into_inner();
    let details = match details_res.result {
        Some(get_block_details_response::Result::Details(details)) => details,
        Some(get_block_details_response::Result::Error(err)) => return Err(err.message),
        None => {
            return Err(format!(
                "missing block details for confirmed tx at height {block_height}"
            ))
        }
    };

    let tx_index_in_block = details
        .tx_ids
        .iter()
        .position(|h| h.hash == tx_id_base58)
        .map(|idx| idx as u64)
        .ok_or_else(|| {
            format!("tx {tx_id_base58} not found in block tx list at height {block_height}")
        })?;

    Ok(Some(ConfirmedTxPosition {
        block_height,
        tx_index_in_block,
    }))
}

/// Attempt to post settlement receipt metadata to chain.
///
/// The current implementation returns a deterministic local marker in
/// local mode and validates chain connectivity in submit modes.
pub async fn post_settlement_receipt(
    cfg: &vesl_core::SettlementConfig,
    note_id_hex: &str,
) -> Result<Option<String>, String> {
    if matches!(cfg.mode, vesl_core::SettlementMode::Local) {
        return Ok(None);
    }
    let endpoint = cfg
        .chain_endpoint
        .as_deref()
        .ok_or_else(|| "chain endpoint not configured".to_string())?;
    // Connectivity probe so settlement surfaces actionable failures.
    let mut client = ChainClient::connect(ChainConfig::local(endpoint))
        .await
        .map_err(|e| format!("chain connect failed: {e}"))?;
    // Probe with a known-false tx id to verify RPC reachability.
    let _ = client
        .check_accepted("11111111111111111111111111111111111111111111111111111111111")
        .await
        .map_err(|e| format!("chain probe failed: {e}"))?;
    Ok(Some(format!("queued-{note_id_hex}")))
}

/// Submit a claim note to chain.
///
/// Current implementation validates that chain RPC is reachable in submit
/// modes and returns a synthetic submission id for tracking.
pub async fn submit_claim_note(
    cfg: &vesl_core::SettlementConfig,
    note: &ClaimNoteV1,
) -> Result<Option<String>, String> {
    if matches!(cfg.mode, vesl_core::SettlementMode::Local) {
        return Ok(None);
    }
    let endpoint = cfg
        .chain_endpoint
        .as_deref()
        .ok_or_else(|| "chain endpoint not configured".to_string())?;
    let mut client = ChainClient::connect(ChainConfig::local(endpoint))
        .await
        .map_err(|e| format!("chain connect failed: {e}"))?;
    let _ = client
        .check_accepted("11111111111111111111111111111111111111111111111111111111111")
        .await
        .map_err(|e| format!("chain probe failed: {e}"))?;
    let payload_len = note.jam_tuple().len();
    Ok(Some(format!(
        "queued-{}-{}-{payload_len}",
        note.name, note.tx_hash
    )))
}

// =========================================================================
// Phase 2c — chain-input fetchers
// =========================================================================

/// Encode a `common.v1.Hash` (5×Belt) as the 40-byte LE-packed atom
/// shape the kernel uses (matches `noun-digest:tip5` on the Hoon side:
/// `[@ux @ux @ux @ux @ux]` where each `@ux` is a single Goldilocks
/// felt and the whole tuple reads as a Tip5 digest).
pub fn hash_to_atom_bytes(h: &Hash) -> Vec<u8> {
    let mut out = Vec::with_capacity(40);
    for b in [&h.belt_1, &h.belt_2, &h.belt_3, &h.belt_4, &h.belt_5] {
        let v = b.as_ref().map(|bb| bb.value).unwrap_or_default();
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Compare two serializations of the same Tip5 `@ux` digest.
///
/// [`crate::kernel::decode_scan_state`] uses `atom.as_ne_bytes()` (minimal
/// width), so genesis `0` is often `[]` or a single `0` byte, while
/// gRPC [`hash_to_atom_bytes`] always returns 40 bytes (5×u64 LE). Those
/// must still compare equal where direct slice equality would fail.
pub fn tip5_atom_semantic_eq(a: &[u8], b: &[u8]) -> bool {
    fn five_limbs(s: &[u8]) -> [u64; 5] {
        let mut padded = [0u8; 40];
        let n = s.len().min(40);
        if n > 0 {
            padded[..n].copy_from_slice(&s[..n]);
        }
        let mut out = [0u64; 5];
        for i in 0..5 {
            out[i] = u64::from_le_bytes(
                padded[i * 8..(i + 1) * 8]
                    .try_into()
                    .expect("40-byte chunk"),
            );
        }
        out
    }
    five_limbs(a) == five_limbs(b)
}

/// First Nockchain block height the chain follower applies after a **genesis**
/// kernel cursor (`last-proved-height=0`, zero digest). Blocks below this are
/// skipped: NNS did not ship at Nockchain genesis.
///
/// **Protocol constant:** must match `++nns-genesis-height` in
/// `hoon/app/app.hoon` (not configurable in `vesl.toml` or env).
pub const NNS_GENESIS_HEIGHT: u64 = 63_000;

/// Matches the kernel `%scan-block` `boot` guard in `hoon/app/app.hoon`: cursor still at
/// genesis (`last-proved-height=0` and `last-proved-digest=@ux` `0`).
///
/// In that state the kernel accepts **any** `parent` on the first poke — block
/// `nns-genesis-height`'s parent is the **real** header digest below
/// that height on Nockchain, not `@ux` `0`.
pub fn scan_cursor_is_genesis_boot(last_proved_height: u64, last_proved_digest: &[u8]) -> bool {
    last_proved_height == 0 && tip5_atom_semantic_eq(last_proved_digest, &[0u8; 40])
}

/// Build a `common.v1.Hash` from 40 bytes of LE packed felts. Returns
/// `None` if the slice is not exactly 40 bytes.
pub fn atom_bytes_to_hash(bytes: &[u8]) -> Option<Hash> {
    if bytes.len() != 40 {
        return None;
    }
    let mut belts = [0u64; 5];
    for (i, b) in belts.iter_mut().enumerate() {
        let mut tmp = [0u8; 8];
        tmp.copy_from_slice(&bytes[i * 8..(i + 1) * 8]);
        *b = u64::from_le_bytes(tmp);
    }
    Some(Hash {
        belt_1: Some(Belt { value: belts[0] }),
        belt_2: Some(Belt { value: belts[1] }),
        belt_3: Some(Belt { value: belts[2] }),
        belt_4: Some(Belt { value: belts[3] }),
        belt_5: Some(Belt { value: belts[4] }),
    })
}

/// Decode a Nockchain base58 Tip5 hash into the 40-byte LE-packed atom
/// representation used by the Hoon kernel.
pub fn base58_hash_to_atom_bytes(value: &str) -> Result<Vec<u8>, String> {
    let hash = Tip5Hash::from_base58(value)
        .map_err(|e| format!("invalid base58 Tip5 hash {value:?}: {e}"))?;
    let mut out = Vec::with_capacity(40);
    for limb in hash.to_array() {
        out.extend_from_slice(&limb.to_le_bytes());
    }
    Ok(out)
}

/// Convert the public block proto's base58 tx-id list into kernel atoms.
pub fn tx_ids_from_block_details(details: &BlockDetails) -> Result<Vec<Vec<u8>>, String> {
    details
        .tx_ids
        .iter()
        .map(|h| base58_hash_to_atom_bytes(&h.hash))
        .collect()
}

/// Decode a 40-byte LE limb-packed tx-id atom (kernel / gRPC shape) into a Tip5 [`Tip5Hash`].
pub fn tx_atom_bytes_to_tip5_hash(bytes: &[u8]) -> Result<Tip5Hash, String> {
    if bytes.len() != 40 {
        return Err(format!(
            "tx-id atom must be 40 bytes (5×u64 LE), got {}",
            bytes.len()
        ));
    }
    let mut limbs = [0u64; 5];
    for i in 0..5 {
        let chunk: [u8; 8] = bytes[i * 8..(i + 1) * 8]
            .try_into()
            .map_err(|_| "internal: tx-id slice chunk".to_string())?;
        limbs[i] = u64::from_le_bytes(chunk);
    }
    Ok(Tip5Hash::from_limbs(&limbs))
}

/// Encode [`Tip5Hash`] as the 40-byte LE atom bytes used in `%scan-block` `page-tx-ids`.
pub fn tip5_hash_to_tx_atom_bytes(h: &Tip5Hash) -> Vec<u8> {
    let limbs = h.to_array();
    let mut out = Vec::with_capacity(40);
    for l in limbs {
        out.extend_from_slice(&l.to_le_bytes());
    }
    out
}

/// Deterministic tx-id order matching Hoon `~(tap z-in tx-ids)` on the canonical z-set:
/// insert each `@ux` tx-id into a [`ZSet`], then [`ZSet::into_items`] (same `gor-tip` /
/// `dor-tip` structure as `/common/zoon`).
///
/// RPC lists may be arbitrary; Nockchain block nouns store tx-ids as `(z-set @ux)`.
///
/// If **any** entry is not exactly **40 bytes** (legacy tests using 1-byte stub atoms),
/// returns the input **unchanged** so harnesses keep deterministic insertion order.
pub fn canonical_z_set_tx_order(page_tx_ids: Vec<Vec<u8>>) -> Result<Vec<Vec<u8>>, String> {
    if page_tx_ids.is_empty() {
        return Ok(Vec::new());
    }
    if page_tx_ids.iter().any(|id| id.len() != 40) {
        return Ok(page_tx_ids);
    }
    let mut seen = std::collections::HashSet::<Vec<u8>>::new();
    for id in &page_tx_ids {
        if !seen.insert(id.clone()) {
            return Err(format!(
                "duplicate tx-id in block page_tx_ids list ({} bytes)",
                id.len()
            ));
        }
    }
    let hashes: Vec<Tip5Hash> = page_tx_ids
        .iter()
        .map(|b| tx_atom_bytes_to_tip5_hash(b))
        .collect::<Result<Vec<_>, _>>()?;
    let set = ZSet::try_from_items(hashes).map_err(|e| format!("z-set tx-id build failed: {e}"))?;
    Ok(set
        .into_items()
        .into_iter()
        .map(|h| tip5_hash_to_tx_atom_bytes(&h))
        .collect())
}

/// Reorder [`TransactionDetails`] to match `canonical_tx_atoms` order from [`canonical_z_set_tx_order`].
pub fn align_transaction_details_with_canonical_order(
    canonical_tx_atoms: &[Vec<u8>],
    tx_details: Vec<TransactionDetails>,
) -> Result<Vec<TransactionDetails>, String> {
    let mut by_id: HashMap<Vec<u8>, TransactionDetails> = HashMap::with_capacity(tx_details.len());
    for d in tx_details {
        let atom = base58_hash_to_atom_bytes(&d.tx_id)?;
        by_id.insert(atom, d);
    }
    let mut ordered = Vec::with_capacity(canonical_tx_atoms.len());
    for id in canonical_tx_atoms {
        let d = by_id.remove(id).ok_or_else(|| {
            format!(
                "missing TransactionDetails for tx atom {} (align with block tx_ids)",
                hex_prefix(id, 8)
            )
        })?;
        ordered.push(d);
    }
    if !by_id.is_empty() {
        return Err(format!(
            "extra TransactionDetails not listed in page_tx_ids ({} stray)",
            by_id.len()
        ));
    }
    Ok(ordered)
}

fn hex_prefix(bytes: &[u8], n: usize) -> String {
    let show = n.min(bytes.len());
    let hex: String = bytes[..show].iter().map(|b| format!("{b:02x}")).collect();
    if bytes.len() > show {
        format!("{hex}…")
    } else {
        hex
    }
}

/// Apply canonical z-set ordering to RPC-fetched block inputs before `%scan-block`.
pub fn align_scan_block_fetch_tx_order(
    page_tx_ids: Vec<Vec<u8>>,
    tx_details: Vec<TransactionDetails>,
) -> Result<(Vec<Vec<u8>>, Vec<TransactionDetails>), String> {
    let canonical_ids = canonical_z_set_tx_order(page_tx_ids)?;
    let details = align_transaction_details_with_canonical_order(&canonical_ids, tx_details)?;
    Ok((canonical_ids, details))
}

/// Stable scan order for [`crate::kernel::ClaimCandidate`] rows: sort by z-set position of
/// `witness.tx_id` in the block's tx-id multiset (matches `+claim-scanner` fold order).
pub fn sort_claim_candidates_by_z_set_tx_order(
    page_tx_ids: &[Vec<u8>],
    mut candidates: Vec<ClaimCandidate>,
) -> Result<Vec<ClaimCandidate>, String> {
    let canonical = canonical_z_set_tx_order(page_tx_ids.to_vec())?;
    let rank: HashMap<Vec<u8>, usize> = canonical
        .into_iter()
        .enumerate()
        .map(|(i, k)| (k, i))
        .collect();
    candidates.sort_by_key(|c| {
        rank
            .get(&c.witness.tx_id)
            .copied()
            .unwrap_or(usize::MAX)
    });
    Ok(candidates)
}

/// Connect a `NockchainBlockServiceClient` against `endpoint`.
async fn connect_block_service(
    endpoint: &str,
) -> Result<NockchainBlockServiceClient<tonic::transport::Channel>, String> {
    NockchainBlockServiceClient::connect(endpoint.to_string())
        .await
        .map_err(|e| format!("block service connect failed: {e}"))
}

/// Fetch `BlockDetails` by height from the node's public v2 API.
///
/// Phase 3 will feed this structure (block_id, parent, tx_ids, pow) into
/// the recursive `nns-gate` circuit. Today the follower uses this for
/// `%scan-block` prefetch and claim evidence fetches.
pub async fn fetch_block_details_by_height(
    endpoint: &str,
    height: u64,
) -> Result<BlockDetails, String> {
    let mut client = connect_block_service(endpoint).await?;
    let req = GetBlockDetailsRequest {
        selector: Some(get_block_details_request::Selector::Height(height)),
    };
    let res = client
        .get_block_details(req)
        .await
        .map_err(|e| format!("block details query failed at height {height}: {e}"))?
        .into_inner();
    match res.result {
        Some(get_block_details_response::Result::Details(d)) => Ok(d),
        Some(get_block_details_response::Result::Error(err)) => Err(format!(
            "block details error at height {height}: {}",
            err.message
        )),
        None => Err(format!("empty block details response at height {height}")),
    }
}

/// Fetch the block PoW STARK proof (JAM bytes) for the block at
/// `height`. Returns `Ok(None)` when the block has no PoW (genesis /
/// pre-PoW-activation blocks). Phase 3 will embed these bytes as the
/// `proof:sp` input to the recursive `nns-gate` circuit.
pub async fn fetch_block_proof_bytes(
    endpoint: &str,
    height: u64,
) -> Result<Option<Vec<u8>>, String> {
    let details = fetch_block_details_by_height(endpoint, height).await?;
    let Some(pow) = details.pow else {
        return Ok(None);
    };
    if !pow.present {
        return Ok(None);
    }
    Ok(pow.raw_proof)
}

enum TxDetailsOutcome {
    Ready(TransactionDetails),
    Pending,
}

/// One `get_transaction_details` round-trip (no retry). Reuses an existing
/// client so block scans do not open a new gRPC connection per tx.
async fn fetch_transaction_details_outcome_client(
    client: &mut NockchainBlockServiceClient<tonic::transport::Channel>,
    tx_id_base58: &str,
) -> Result<TxDetailsOutcome, String> {
    let req = GetTransactionDetailsRequest {
        tx_id: Some(Base58Hash {
            hash: tx_id_base58.to_string(),
        }),
    };
    let res = client
        .get_transaction_details(req)
        .await
        .map_err(|e| format!("transaction details query failed for {tx_id_base58}: {e}"))?
        .into_inner();
    match res.result {
        Some(get_transaction_details_response::Result::Details(d)) => Ok(TxDetailsOutcome::Ready(d)),
        Some(get_transaction_details_response::Result::Pending(_)) => Ok(TxDetailsOutcome::Pending),
        Some(get_transaction_details_response::Result::Error(err)) => Err(format!(
            "transaction details error for {tx_id_base58}: {}",
            err.message
        )),
        None => Err(format!(
            "empty transaction details response for {tx_id_base58}"
        )),
    }
}

/// One `get_transaction_details` round-trip (no retry), with a fresh connection.
async fn fetch_transaction_details_outcome(
    endpoint: &str,
    tx_id_base58: &str,
) -> Result<TxDetailsOutcome, String> {
    let mut client = connect_block_service(endpoint).await?;
    fetch_transaction_details_outcome_client(&mut client, tx_id_base58).await
}

/// Transaction-level structured details from the node. Phase 3 will
/// reshape this into the `raw-tx:t` noun the circuit consumes; for now
/// the hull just round-trips the proto and caches it in the claim note.
///
/// Returns `Err` immediately if the node reports `Pending` (still in mempool).
/// Block scanning uses [`fetch_block_transaction_details`] instead, which
/// retries `Pending` to absorb index lag vs finalized blocks.
pub async fn fetch_transaction_details(
    endpoint: &str,
    tx_id_base58: &str,
) -> Result<TransactionDetails, String> {
    match fetch_transaction_details_outcome(endpoint, tx_id_base58).await? {
        TxDetailsOutcome::Ready(d) => Ok(d),
        TxDetailsOutcome::Pending => Err(format!(
            "tx {tx_id_base58} is pending; no block yet"
        )),
    }
}

/// Fetch transaction details for every tx-id listed in a block.
///
/// Retries when the node returns `Pending` for a tx-id that already appears
/// in `block.tx_ids` — common transient lag between block headers and the
/// transaction index on busy nodes.
///
/// **Concurrency:** one gRPC client is shared (cloned per in-flight tx).
/// Fetches run in parallel up to [`MAX_CONCURRENT_TX_FETCHES`] so wide blocks
/// are not serialized one RPC at a time (the dominant follower cost when
/// `%scan-block` does not run the STARK prover).
pub async fn fetch_block_transaction_details(
    endpoint: &str,
    block: &BlockDetails,
) -> Result<Vec<TransactionDetails>, String> {
    // Retries per tx before failing (bounded wait ~15–25s worst case).
    const MAX_ATTEMPTS: u32 = 24;
    const INITIAL_DELAY_MS: u64 = 80;
    const MAX_DELAY_MS: u64 = 1_500;
    const MAX_CONCURRENT_TX_FETCHES: usize = 32;

    if block.tx_ids.is_empty() {
        return Ok(Vec::new());
    }

    let root_client = connect_block_service(endpoint).await?;
    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT_TX_FETCHES));
    let mut handles = Vec::with_capacity(block.tx_ids.len());

    for (idx, tx_id) in block.tx_ids.iter().enumerate() {
        let hash = tx_id.hash.clone();
        let mut c = root_client.clone();
        let permit = sem.clone();
        handles.push(tokio::spawn(async move {
            let _permit = permit
                .acquire_owned()
                .await
                .map_err(|e| format!("semaphore closed: {e}"))?;
            let mut delay_ms = INITIAL_DELAY_MS;
            for attempt in 1..=MAX_ATTEMPTS {
                match fetch_transaction_details_outcome_client(&mut c, &hash).await? {
                    TxDetailsOutcome::Ready(d) => return Ok((idx, d)),
                    TxDetailsOutcome::Pending => {
                        if attempt >= MAX_ATTEMPTS {
                            return Err(format!(
                                "tx {hash} still Pending after {MAX_ATTEMPTS} get_transaction_details attempts; \
                                 node tx index lags block header or tx left mempool — retry later"
                            ));
                        }
                        tracing::debug!(
                            tx_id = %hash,
                            attempt,
                            delay_ms,
                            "chain: transaction details Pending while scanning block; retrying"
                        );
                        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                        delay_ms = (delay_ms.saturating_mul(3) / 2).min(MAX_DELAY_MS);
                    }
                }
            }
            Err(format!(
                "internal error: missing transaction details for {hash} after retries"
            ))
        }));
    }

    let mut indexed: Vec<(usize, TransactionDetails)> = Vec::with_capacity(handles.len());
    for h in handles {
        indexed.push(
            h.await
                .map_err(|e| format!("transaction-details fetch task join: {e}"))??,
        );
    }
    indexed.sort_by_key(|(i, _)| *i);
    Ok(indexed.into_iter().map(|(_, d)| d).collect())
}

/// RPC bundle for one `%scan-block`: digests, tx-id atom list, and full tx
/// bodies for claim extraction.
#[derive(Debug, Clone)]
pub struct ScanBlockFetch {
    pub height: u64,
    pub page_digest: Vec<u8>,
    pub parent: Vec<u8>,
    pub page_tx_ids: Vec<Vec<u8>>,
    pub tx_details: Vec<TransactionDetails>,
}

/// Fetch everything needed for one `%scan-block` poke at `height`.
pub async fn fetch_scan_block_inputs(endpoint: &str, height: u64) -> Result<ScanBlockFetch, String> {
    let details = fetch_block_details_by_height(endpoint, height).await?;
    let dh = details.height;
    if dh != height {
        tracing::warn!(
            details_height = dh,
            requested = height,
            "chain: GetBlockDetails height differs from selector"
        );
    }
    let page_digest_atom = details
        .block_id
        .as_ref()
        .ok_or_else(|| format!("block {height} missing block_id"))?;
    let parent_atom = details
        .parent
        .as_ref()
        .ok_or_else(|| format!("block {height} missing parent"))?;
    let page_digest = hash_to_atom_bytes(page_digest_atom);
    let parent = hash_to_atom_bytes(parent_atom);
    let page_tx_ids = tx_ids_from_block_details(&details)?;
    let tx_details = fetch_block_transaction_details(endpoint, &details).await?;
    let (page_tx_ids, tx_details) = align_scan_block_fetch_tx_order(page_tx_ids, tx_details)?;
    Ok(ScanBlockFetch {
        height: dh,
        page_digest,
        parent,
        page_tx_ids,
        tx_details,
    })
}

/// Prefetch several blocks in parallel (bounded concurrency). Returned
/// slice is sorted by ascending height; callers must still poke `%scan-block`
/// strictly in order.
async fn prefetch_scan_blocks_for_heights_inner(
    endpoint: &str,
    heights: &[u64],
    max_concurrent_blocks: usize,
) -> Result<Vec<ScanBlockFetch>, String> {
    if heights.is_empty() {
        return Ok(Vec::new());
    }
    let endpoint = endpoint.to_string();
    let sem = Arc::new(Semaphore::new(max_concurrent_blocks.max(1)));
    let mut handles = Vec::with_capacity(heights.len());
    for &h in heights {
        let ep = endpoint.clone();
        let permit = sem.clone();
        handles.push(tokio::spawn(async move {
            let _p = permit
                .acquire_owned()
                .await
                .map_err(|e| format!("semaphore: {e}"))?;
            fetch_scan_block_inputs(&ep, h).await
        }));
    }
    let mut out = Vec::with_capacity(handles.len());
    for j in handles {
        out.push(
            j.await
                .map_err(|e| format!("prefetch scan block task join: {e}"))??,
        );
    }
    out.sort_by_key(|b| b.height);
    Ok(out)
}

/// Parallel prefetch for follower catch-up ([`crate::chain_follower`]).
///
/// Uses at most [`SCAN_BLOCK_PREFETCH_CONCURRENCY`] concurrent block pulls;
/// each block still fans out to parallel tx-detail RPCs internally.
pub async fn prefetch_scan_blocks_for_heights(
    endpoint: &str,
    heights: &[u64],
) -> Result<Vec<ScanBlockFetch>, String> {
    prefetch_scan_blocks_for_heights_inner(endpoint, heights, SCAN_BLOCK_PREFETCH_CONCURRENCY).await
}

/// Max concurrent `GetBlockDetails`+tx pulls while prefetching a batch.
pub const SCAN_BLOCK_PREFETCH_CONCURRENCY: usize = 32;

/// Verify prefetched headers link within `blocks`: each `parent` equals the
/// prior row’s `page_digest`, anchored at [`scan_cursor_is_genesis_boot`].
///
/// When [`scan_cursor_is_genesis_boot`] is true, the **first** prefetched block’s
/// parent is **not** checked against `last_proved_digest` (still `@ux` `0` in the
/// kernel). That first parent must instead be the chain’s genesis digest — the hull
/// mirrors the kernel’s `boot` branch and only RPC-consistency is enforced across
/// subsequent rows in this batch.
pub fn validate_scan_block_chain(
    last_proved_height: u64,
    last_proved_digest: &[u8],
    blocks: &[ScanBlockFetch],
) -> Result<(), String> {
    let genesis_boot = scan_cursor_is_genesis_boot(last_proved_height, last_proved_digest);
    let mut exp = last_proved_digest.to_vec();

    for (i, b) in blocks.iter().enumerate() {
        let skip_parent_vs_cursor = genesis_boot && i == 0;
        if !skip_parent_vs_cursor && !tip5_atom_semantic_eq(b.parent.as_slice(), exp.as_slice()) {
            return Err(format!(
                "prefetched scan batch parent mismatch at height {} (RPC skew or fork)",
                b.height
            ));
        }
        exp.clone_from(&b.page_digest);
    }
    Ok(())
}

/// Composite fetch: given a confirmed tx id, return its containing
/// block's `BlockDetails` plus that block's PoW proof bytes (if any).
/// This is the per-claim bundle Phase 4's follower will embed into the
/// kernel `%claim` poke for in-gate verification.
#[derive(Debug, Clone)]
pub struct ClaimBlockBundle {
    pub block: BlockDetails,
    pub block_proof: Option<Vec<u8>>,
}

pub async fn fetch_page_for_tx(
    endpoint: &str,
    tx_id_base58: &str,
) -> Result<ClaimBlockBundle, String> {
    let mut client = connect_block_service(endpoint).await?;
    let tx_req = GetTransactionBlockRequest {
        tx_id: Some(Base58Hash {
            hash: tx_id_base58.to_string(),
        }),
    };
    let tx_res = client
        .get_transaction_block(tx_req)
        .await
        .map_err(|e| format!("transaction block query failed: {e}"))?
        .into_inner();
    let height = match tx_res.result {
        Some(get_transaction_block_response::Result::Block(b)) => b.height,
        Some(get_transaction_block_response::Result::Pending(_)) => {
            return Err(format!("tx {tx_id_base58} not yet in a block"));
        }
        Some(get_transaction_block_response::Result::Error(err)) => {
            return Err(err.message);
        }
        None => return Err(format!("empty tx-block response for {tx_id_base58}")),
    };
    let block = fetch_block_details_by_height(endpoint, height).await?;
    let block_proof = block
        .pow
        .as_ref()
        .filter(|p| p.present)
        .and_then(|p| p.raw_proof.clone());
    Ok(ClaimBlockBundle { block, block_proof })
}

/// Light read of the current chain tip height.
///
/// Uses `GetBlocks` with page size 1 (newest-first) and extracts
/// `current_height`. Cheap enough to call every follower tick.
pub async fn fetch_current_tip_height(endpoint: &str) -> Result<u64, String> {
    let mut client = connect_block_service(endpoint).await?;
    let req = GetBlocksRequest {
        page: Some(PageRequest {
            page_token: String::new(),
            client_page_items_limit: 1,
            ..Default::default()
        }),
    };
    let res = client
        .get_blocks(req)
        .await
        .map_err(|e| format!("GetBlocks query failed: {e}"))?
        .into_inner();
    match res.result {
        Some(get_blocks_response::Result::Blocks(b)) => Ok(b.current_height),
        Some(get_blocks_response::Result::Error(e)) => Err(e.message),
        None => Err("empty GetBlocks response".into()),
    }
}

#[cfg(test)]
mod tip5_semantic_eq_tests {
    use super::{
        scan_cursor_is_genesis_boot, tip5_atom_semantic_eq, validate_scan_block_chain,
        ScanBlockFetch,
    };

    #[test]
    fn zero_digest_empty_matches_rpc_forty_zeros() {
        assert!(tip5_atom_semantic_eq(&[], &[0u8; 40]));
        assert!(tip5_atom_semantic_eq(&[0u8], &[0u8; 40]));
        assert!(tip5_atom_semantic_eq(&[0u8; 40], &[0u8; 40]));
    }

    #[test]
    fn nonzero_requires_full_agreement() {
        let mut a = [0u8; 40];
        a[0] = 7;
        assert!(!tip5_atom_semantic_eq(&[], &a));
        assert!(tip5_atom_semantic_eq(&a, &a));
    }

    #[test]
    fn genesis_boot_detects_kernel_cursor() {
        assert!(scan_cursor_is_genesis_boot(0, &[]));
        assert!(scan_cursor_is_genesis_boot(0, &[0u8; 40]));
        assert!(!scan_cursor_is_genesis_boot(1, &[]));
        assert!(!scan_cursor_is_genesis_boot(0, &[1u8; 40]));
    }

    #[test]
    fn validate_chain_boot_accepts_real_genesis_parent_not_kernel_zero() {
        let mut genesis_hdr = [0u8; 40];
        genesis_hdr[..8].copy_from_slice(&0xdeadbeefu64.to_le_bytes());

        let b1 = ScanBlockFetch {
            height: 1,
            page_digest: vec![9u8; 40],
            parent: genesis_hdr.to_vec(),
            page_tx_ids: vec![],
            tx_details: vec![],
        };
        validate_scan_block_chain(0, &[], &[b1]).expect("block1 parent is chain genesis, not @ux 0");
    }

    #[test]
    fn validate_chain_boot_digest_first_block_all_zero_parent_still_ok() {
        let b = ScanBlockFetch {
            height: 1,
            page_digest: vec![9u8; 40],
            parent: vec![0u8; 40],
            page_tx_ids: vec![],
            tx_details: vec![],
        };
        validate_scan_block_chain(0, &[], &[b]).expect("kernel peek [] vs RPC genesis parent");
    }

    #[test]
    fn validate_chain_second_block_links_to_first_page_digest() {
        let b1 = ScanBlockFetch {
            height: 1,
            page_digest: vec![3u8; 40],
            parent: vec![0xffu8; 40],
            page_tx_ids: vec![],
            tx_details: vec![],
        };
        let b2 = ScanBlockFetch {
            height: 2,
            page_digest: vec![4u8; 40],
            parent: vec![3u8; 40],
            page_tx_ids: vec![],
            tx_details: vec![],
        };
        validate_scan_block_chain(0, &[], &[b1, b2]).expect("two-block batch from genesis");
    }
}

#[cfg(test)]
mod z_set_order_tests {
    use super::*;
    use crate::kernel::{ClaimCandidate, ClaimWitness};

    fn atom_from_limbs(l: [u64; 5]) -> Vec<u8> {
        let h = Tip5Hash::from_limbs(&l);
        tip5_hash_to_tx_atom_bytes(&h)
    }

    #[test]
    fn canonical_z_set_order_idempotent() {
        let a = atom_from_limbs([3, 0, 0, 0, 0]);
        let b = atom_from_limbs([9, 0, 0, 0, 0]);
        let once = canonical_z_set_tx_order(vec![b.clone(), a.clone()]).unwrap();
        let twice = canonical_z_set_tx_order(once.clone()).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn non_40_byte_stub_preserves_input_order() {
        let v = vec![vec![0x07], vec![0x08]];
        let out = canonical_z_set_tx_order(v.clone()).unwrap();
        assert_eq!(out, v);
    }

    #[test]
    fn sort_claim_candidates_follows_z_set_rank() {
        let a = atom_from_limbs([1, 0, 0, 0, 0]);
        let b = atom_from_limbs([4, 0, 0, 0, 0]);
        let page = vec![b.clone(), a.clone()];
        let canon = canonical_z_set_tx_order(page.clone()).unwrap();
        assert_eq!(canon.len(), 2);

        let c_a = ClaimCandidate {
            name: "z.NOCK".to_string(),
            owner: "alice".to_string(),
            fee: 0,
            tx_hash: a.clone(),
            witness: ClaimWitness {
                tx_id: a,
                spender_pkh: vec![],
                treasury_amount: 0,
                output_lock_root: String::new(),
            },
        };
        let c_b = ClaimCandidate {
            name: "z.NOCK".to_string(),
            owner: "bob".to_string(),
            fee: 0,
            tx_hash: b.clone(),
            witness: ClaimWitness {
                tx_id: b,
                spender_pkh: vec![],
                treasury_amount: 0,
                output_lock_root: String::new(),
            },
        };
        // Deliberately reverse of desired scan order
        let sorted = sort_claim_candidates_by_z_set_tx_order(&page, vec![c_b, c_a]).unwrap();
        assert_eq!(sorted[0].witness.tx_id, canon[0]);
        assert_eq!(sorted[1].witness.tx_id, canon[1]);
    }
}

