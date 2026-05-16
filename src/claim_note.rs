//! NNS claim note decoding for **NoteData on v1 outputs** (Path Y scanner).
//! See `docs/claim-note-wallet-support.md` and [nockchain#116](https://github.com/nockchain/nockchain/pull/116).
//!
//! Claims use the canonical **`blob`** note-data key. The on-wire value is **wallet-packed**
//! (JAM of belt list → inner bytes); see [`crate::packed_blob`]. The inner payload is either
//! JAM of `[name owner tx_hash]` (cord triple) or a UTF-8 **claim path** `nns/v1/claim/<name>.nock`
//! (owner / tx id inferred by the follower from the enclosing tx and signers).
//!
//! **No optional “chain bundle” in note-data:** the hull does not trust extra attachments for
//! raw-tx, page, proofs, or headers — it **re-fetches** the paying tx and block context from
//! Nockchain RPC and runs predicates (`chain_follower`, kernel) on that canonical view.
use nock_noun_rs::{cue_from_bytes, jam_to_bytes, make_cord, new_stack, T};
use nockchain_client_rs::{find_entry, find_opaque_bytes_entry, NoteData};
use serde::{Deserialize, Serialize};

use crate::noun_access::ScopedNoun;

/// Programmatic claim payload (`wallet-tx-builder` / gRPC `NoteDataEntry.key`).
pub const CLAIM_NOTE_BLOB_ENTRY_KEY: &str = "blob";

/// On-chain `nns/v1/claim` txs may use a **path-shaped** inner payload (UTF-8) instead of a JAM
/// triple; the hull fills `tx_hash` from the enclosing tx id and `owner` from spenders.
pub const CLAIM_NOTE_PATH_PREFIX: &str = "nns/v1/claim/";

/// If `bytes` is a UTF-8 claim path (`nns/v1/claim/<stem>.nock`), returns the `<stem>.nock` name.
pub fn claim_name_from_path_inner(bytes: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(bytes).ok()?.trim_end_matches('\0');
    if !s.starts_with(CLAIM_NOTE_PATH_PREFIX) || !s.ends_with(".nock") {
        return None;
    }
    let name = s[CLAIM_NOTE_PATH_PREFIX.len()..].to_string();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimNoteV1 {
    pub name: String,
    pub owner: String,
    pub tx_hash: String,
}

impl ClaimNoteV1 {
    pub fn new(name: String, owner: String, tx_hash: String) -> Self {
        Self {
            name,
            owner,
            tx_hash,
        }
    }

    /// Canonical jam payload for the claim triple `[name owner tx_hash]`.
    pub fn jam_tuple(&self) -> Vec<u8> {
        let mut stack = new_stack();
        let n = make_cord(&mut stack, &self.name);
        let o = make_cord(&mut stack, &self.owner);
        let tx = make_cord(&mut stack, &self.tx_hash);
        let noun = T(&mut stack, &[n, o, tx]);
        jam_to_bytes(noun, &stack.noun_space())
    }

    /// Decode **`blob`** only. Chain evidence is **not** read from note-data; the follower
    /// loads `TransactionDetails` and block metadata from RPC and validates in the kernel.
    ///
    /// Tries the jammed-opaque encoding first (`jam_opaque_bytes_entry` round-trip). If that
    /// fails, falls back to **raw wallet-packed wire** in `NoteDataEntry.blob` (as returned by
    /// some `GetTransactionDetails` builds / grpcurl samples).
    pub fn from_note_data(note_data: &NoteData) -> Result<Self, String> {
        let wire = match find_opaque_bytes_entry(note_data, CLAIM_NOTE_BLOB_ENTRY_KEY) {
            Ok(w) => w,
            Err(_) => find_entry(note_data, CLAIM_NOTE_BLOB_ENTRY_KEY)
                .map_err(|e| format!("missing {CLAIM_NOTE_BLOB_ENTRY_KEY} note-data entry: {e}"))?
                .blob
                .to_vec(),
        };
        Self::from_wallet_packed_blob_wire(&wire)
    }

    /// Decode a **wallet-packed** claim `blob` wire (inner path after `GetTransactionDetails`
    /// exposes raw `bytes` for the entry, i.e. **not** jam-of-atom wrapped). Used by tests and
    /// tooling that mirror gRPC `NoteDataEntry.blob` verbatim.
    pub fn from_wallet_packed_blob_wire(wire: &[u8]) -> Result<Self, String> {
        let tuple_jam = match crate::packed_blob::unpack_wallet_blob_jam(wire) {
            Ok(inner) => inner,
            Err(_) => wire.to_vec(),
        };

        if let Some(name) = claim_name_from_path_inner(&tuple_jam) {
            return Ok(Self {
                name,
                owner: String::new(),
                tx_hash: String::new(),
            });
        }

        let mut stack = new_stack();
        let tuple = cue_from_bytes(&mut stack, &tuple_jam)
            .ok_or_else(|| "failed to decode claim tuple".to_string())?;
        let root = ScopedNoun::from_stack(&stack, tuple);
        let (name_sn, rest) = root
            .uncons()
            .map_err(|_| "claim tuple malformed (slot 1)".to_string())?;
        let (owner_sn, tx_hash_sn) = rest
            .uncons()
            .map_err(|_| "claim tuple malformed (slot 2)".to_string())?;

        Ok(Self {
            name: cord_field(&name_sn)?,
            owner: cord_field(&owner_sn)?,
            tx_hash: cord_field(&tx_hash_sn)?,
        })
    }
}

/// Cord (`@t`) fields are atoms; some encoders terminate the last field as `[atom ~]`.
fn cord_field(sn: &ScopedNoun<'_>) -> Result<String, String> {
    let leaf = if sn.is_atom() {
        sn.clone()
    } else {
        let (head, tail) = sn.uncons()?;
        let zero_tail = tail.as_u64_opt() == Some(0);
        if !zero_tail {
            return Err("claim tuple field is not an atom".to_string());
        }
        head
    };
    leaf.as_cord()
        .map_err(|e| format!("claim tuple field is not utf8: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn sample_note() -> ClaimNoteV1 {
        ClaimNoteV1 {
            name: "foo.nock".to_string(),
            owner: "owner-xyz".to_string(),
            tx_hash: "tx-abc".to_string(),
        }
    }

    fn note_data_fixture(note: &ClaimNoteV1) -> NoteData {
        let wire = crate::packed_blob::pack_wallet_blob_jam(&note.jam_tuple());
        use nockchain_client_rs::jam_opaque_bytes_entry;
        NoteData::new(vec![jam_opaque_bytes_entry(
            CLAIM_NOTE_BLOB_ENTRY_KEY,
            &wire,
        )])
    }

    #[test]
    fn note_data_roundtrip_preserves_fields() {
        let note = sample_note();
        let decoded = ClaimNoteV1::from_note_data(&note_data_fixture(&note)).expect("decode");
        assert_eq!(decoded, note);
    }

    /// grpcurl sample `blob` base64 (`wXZA...`) — wallet-packed UTF-8 path, not a JAM triple.
    #[test]
    fn grpcurl_fixture_blob_unpacks_to_claim_path() {
        let wire = base64::engine::general_purpose::STANDARD
            .decode("wXZAd3ObewO+XczLOOCzhaW1A/6L29s44K+NoYUDfpqbizvgvY2tBQ==")
            .unwrap();
        let inner = crate::packed_blob::unpack_wallet_blob_jam(&wire).expect("unpack");
        assert_eq!(
            std::str::from_utf8(&inner).unwrap(),
            "nns/v1/claim/nockchain.nock"
        );
        let note = ClaimNoteV1::from_wallet_packed_blob_wire(&wire).expect("decode");
        assert_eq!(note.name, "nockchain.nock");
        assert!(note.owner.is_empty() && note.tx_hash.is_empty());
    }
}
