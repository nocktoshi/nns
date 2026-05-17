# Nockchain sibling checkout (`../nockchain`)

`nns-vesl` uses **path dependencies** on a local **`../nockchain`** tree (`Cargo.toml`: `nockapp`, `nockapp-grpc`, `nockchain-types`, `zkvm-jetpack`, …). You maintain that checkout in your own repo; **this document only states the contract NNS needs** — not how to branch or merge your fork.

## Contract: what Path Y needs

1. **RPC / block explorer surfaces must return `note_data` on v1 outputs**  
   The follower (`src/chain_follower.rs`) only learns about claims from `output.note_data`. If `GetBlockDetails` (or whatever your build wires) omits it, `%scan-block` never sees `nns/v1/claim` payloads.

2. **`GetTransactionDetails` must expose tx signers on each input**  
   Path Y verifies `claim.owner` against **`TransactionInput.signer_pubkey_b58`** (Schnorr pubkey base58 from each v1 spend’s witness / legacy signature map), not against the spent note’s `note_name_b58`. If this field is missing on all inputs, the follower falls back to the legacy `note_name_b58 == owner` rule only when every `signer_pubkey_b58` list is empty (pre-upgrade nodes).

3. **A way to put that NoteData on-chain**  
   Wallets must be able to attach the keyed blobs NNS defines (`docs/claim-note-wallet-support.md`). Upstream MR: **[nockchain#116](https://github.com/nockchain/nockchain/pull/116)** (`create-tx --memo-data` — API still evolving in review).

## Hoon / `hoonc`

Point **`NOCK_HOME`** at the same tree `scripts/setup-hoon-tree.sh` symlinks from (`../nockchain` by default) so kernel builds see the same Nockchain Hoon as the Rust crates.

## Relation to this repo

NNS does **not** vendor nockchain sources beyond those path deps. Fork-specific notes, merge plans, and wallet patches live **only in your nockchain fork**; keep `../nockchain` on whatever branch satisfies the contract above for your environment.
