# nns-vesl

<img width="1416" height="540" alt="NNS - Nockchain Name Service" src="https://github.com/user-attachments/assets/7793ed6f-766a-4e93-a2b4-834c04843c37" /> <br />

NNS — the Nockchain Name Service. On-chain `.nock` name registrar,
ported from the centralized Cloudflare Worker at `api.nocknames.com`
to a Vesl-grafted NockApp.

The hull is a **read-only chain scanner** — `GET /health`, `GET /status`, `GET /accumulator/:name` only. Users register `.nock` names by submitting **`nns/v1/claim`** notes to Nockchain; this HTTP service does **not** accept claim POSTs. Offline verification uses **`light_verify`** ([`docs/wallet-verification.md`](docs/wallet-verification.md): pinned checkpoint, headers, recursive STARK, accumulator snapshot — no live Nockchain RPC at verify time). **On-chain claims** need structured **NoteData** on outputs; see [`docs/claim-note-wallet-support.md`](docs/claim-note-wallet-support.md) and [nockchain#85](https://github.com/nockchain/nockchain/pull/85). Deeper architecture, proof model, and roadmap: [`ARCHITECTURE.md`](ARCHITECTURE.md).

## Dependencies

```bash
# Nightly Rust
rustup toolchain install nightly

# hoonc + nockup (Hoon dependency installer)
cargo +nightly install --git https://github.com/nockchain/nockchain.git hoonc
cargo +nightly install --path ~/nockchain/crates/nockup --locked   # or clone nockchain first
```

`make install-kernel` (via `make install`) runs `nockup package install` from
[`nockapp.toml`](nockapp.toml): Nockchain `hoon/` @ `ff6dd2d…` and Vesl libs from
[nocktoshi/vesl-core `phase1-verifier-debug-and-type-fix`](https://github.com/nocktoshi/vesl-core/tree/phase1-verifier-debug-and-type-fix)
(`protocol/lib` @ [`dc9382cd`](https://github.com/nocktoshi/vesl-core/commit/dc9382cd2110cb39561bf4abdc47d7d7f88029b5)).

 `make sync-hoon-from-nockup` copies from
`hoon/packages/` into `hoon/common/`, `hoon/dat/`, and `hoon/lib/` (real files, no symlinks);
`hoon/jams/` is vendored in-repo.
No sibling clones required.

## Quick start

```bash
# clone nns repo
git clone https://github.com/nocktoshi/nns-vesl.git

# one-time install (installs `nns` into ~/.local/bin)
make install

# run
nns
```

`make install` also adds `export PATH="$HOME/.local/bin:$PATH"` to
`~/.zshrc` automatically so `nns` resolves to the installed CLI. You may need to open a new shell.

Once started:

```bash
curl -s http://127.0.0.1:3000/status | jq .
curl -s http://127.0.0.1:3000/accumulator/nns.nock | jq .
# Full `jam(accumulator)` hex for `light_verify` (rebuilt kernel JAM — see docs)
curl -s 'http://127.0.0.1:3000/accumulator/nns.nock?wallet_export=true' | jq .
```

## HTTP API

| Method | Path | Purpose |
| ------ | ---- | ------- |
| GET | `/health` | `{"status":"ok"}` |
| GET | `/status` | Settlement mode, chain endpoint, **`scan_state`** (`last_proved_height`, `last_proved_digest`, `accumulator_root`, `accumulator_size`), **`follower`** telemetry (`anchor_lag_blocks` = chain tip minus scan height, etc.) |
| GET | `/accumulator/:name` | Lookup row + proof-axis material + scan fields. Query **`wallet_export=true`** adds **`accumulator_snapshot_hex`** (`jam(accumulator)`) for [`light_verify`](docs/wallet-verification.md) |

CORS is open (`*`). Naming validation for `:name` matches the kernel’s `.nock` rules.

## Kernel and follower

The kernel (`hoon/app/app.hoon`) uses **`v0-state`**: an **`nns-accumulator`**
(z-map of registered names), **`last-proved-height`** / **`last-proved-digest`**
(scan cursor), Vesl graft fragment, and optional STARK plumbing (`last-proved`,
`%prove-arbitrary`, `%verify-stark`, …). The [**chain follower**](src/chain_follower.rs)
loads block data from the configured RPC, validates claim candidates with
[**`nns-predicates`**](hoon/lib/nns-predicates.hoon), and pokes **`%scan-block`**
so each valid on-chain **`nns/v1/claim`** note is applied in canonical
**(block height, tx index)** order.

Claim **NoteData** is decoded in [`src/claim_note.rs`](src/claim_note.rs)
(entry key **`blob`**, value UTF-8 JAM of `[name owner tx-hash]`). Fee tiers
and validation tags live in `nns-predicates` and [`src/payment.rs`](src/payment.rs).

For proof wallets, recursive STARK research, and Vesl integration details, see
[`ARCHITECTURE.md`](ARCHITECTURE.md).

## Consensus architecture (why chain ordering helps)

The consensus problem is not whether one node can reject duplicate names (it
can), but whether **every** honest node applies the **same** ordered stream of
on-chain claims.

Without a shared order source, two nodes could each accept conflicting first
claims for the same name. **Nockchain** provides a single global order. The
follower walks blocks in height order and applies **`%scan-block`** so the
accumulator fold matches what every other honest follower sees.

Predicate rules (format, fee, name uniqueness, payment replay, etc.) live in
`nns-predicates` and are evaluated as candidates enter the scan pipeline — see
[`ARCHITECTURE.md`](ARCHITECTURE.md) for the full proof and wallet story.

## Security/Trust model

### What a malicious app can do

- Submit conflicting or spammy claim transactions (including duplicate
attempts for the same name).
- Lie in indexer responses (`/accumulator/...`, `/status`) to clients that skip **`light_verify`** / checkpoint checks.
- Censor or delay forwarding user requests through its own frontend/API.
- Run a modified follower locally and produce a non-canonical private view.

### What a malicious app cannot do (assuming honest chain + honest followers)

- Force canonical state to accept two owners for one name: kernel C3 rejects
later conflicting claims when replayed in canonical chain order.
- Reuse one payment for multiple successful claims: kernel C4 rejects reused
`tx-hash` values.
- Rewrite chain ordering for honest nodes: canonical `(block_height, tx_index_in_block)` comes from Nockchain data.
- Make honest nodes converge to different final states if they replay the same
chain data with the same kernel rules.

### Remaining work for fully trustless verification

- **Reorg-safe replay:** followers should handle chain reorganizations so the
  scan cursor and accumulator stay consistent with the canonical tip (see
  [`ARCHITECTURE.md`](ARCHITECTURE.md)).
- **Transition-proof completeness:** `nns-gate` currently proves inclusion
properties (G1/G2). Full trustless light-client security needs provable claim
transitions (C1-C4 over ordered events), not only inclusion in a committed
root.
- **Payment attestation depth:** current payment path is chain-acceptance-aware
but not full semantic attestation (`sender`, `recipient`, `amount >= fee`)
in-proof.
- **Client verification defaults:** wallets/UIs should verify proof bundles and
chain anchors by default instead of trusting any single app server response.

## Architecture

Three layers: HTTP clients talk to the **Rust hull** (advisory); the hull
holds the **NockApp** and drives **`%scan-block`** from chain RPC; the **Hoon
kernel** owns **`v0-state`** (accumulator + scan cursor + Vesl graft fragment).
The graft is embedded kernel-side, not a separate process.

```
  HTTP client
       |  GET /status, /accumulator/...
       v
  Rust hull (api.rs, chain_follower.rs)
       |  poke %scan-block + peeks (/accumulator/..., /scan-state, ...)
       v
  Hoon kernel (app.hoon) — v0-state: accumulator, last-proved-*, vesl
       |
       v
  $NNS_DATA_DIR/.nns-data/checkpoints/*.chkjam
```

**Trust boundary:** hull ↔ kernel (the hull is rebuildable from chain replay +
checkpoints). **Vesl / STARK proof scope** (gates, recursive steps, wallet
bundle) is spelled out in [`ARCHITECTURE.md`](ARCHITECTURE.md).


Fee tiers (ported from `nock-names-worker/src/utils/constants.ts`):


| Stem length | API `price` (NOCK) | On-chain / kernel (nicks) |
| ----------- | ------------------ | --------------------------- |
| 1-4 chars   | 5000               | 327680000                   |
| 5-9 chars   | 500                | 32768000                    |
| 10+ chars   | 100                | 6553600                     |


## Configuration

Three layers, in precedence order (highest wins): CLI flags, env vars,
`nns.toml`. The hull honors:


| Env var                 | Purpose                                           | Default                                                  |
| ----------------------- | ------------------------------------------------- | -------------------------------------------------------- |
| `API_PORT`              | HTTP port                                         | `3000`                                                   |
| `BIND_ADDR`             | HTTP bind address                                 | `127.0.0.1`                                              |
| `NNS_DATA_DIR`          | Root dir for kernel checkpoints + mirror snapshot | `.`                                                      |
| `NNS_KERNEL_JAM`        | Path to the compiled kernel                       | `nns.jam`                                                |
| `NNS_PAYMENT_ADDRESS`   | Base58 NNS treasury address (Phase 2)             | `8s29XUK8Do7QWt2MHfPdd1gDSta6db4c3bQrxP1YdJNfXpL3WPzTT5` |
| `NNS_CONFIG`             | Path to settlement config                         | `nns.toml`                                              |
| `RUST_LOG`              | Tracing filter (passed to `tracing_subscriber`)   | unset                                                    |

`NNS_PAYMENT_ADDRESS` is also readable from `nns.toml` as
`payment_address = "..."` for treasury binding used in predicate checks.

Vesl settlement config in `nns.toml`:

```toml
# v1: kernel verifies, no chain interaction.
settlement_mode = "local"

# uncomment and flip to "fakenet" or "dumbnet" for real chain:
# chain_endpoint       = "http://localhost:9090"
# tx_fee               = 256
# accept_timeout_secs  = 300

# Treasury address for predicate checks (see claim-note docs)
# payment_address = "8s29XUK8Do7QWt2MHfPdd1gDSta6db4c3bQrxP1YdJNfXpL3WPzTT5"
```

## Data layout

Under `$NNS_DATA_DIR` (default CWD):

```
.nns-data/
  checkpoints/                   # kernel jammed-state snapshots (NockApp)
  pma/                           # NockApp persistent memory arena
```

Kernel checkpoints hold authoritative **`v0-state`**. The hull keeps
settlement config and follower telemetry in memory; see [`src/state.rs`](src/state.rs).

## Testing

```bash
# Hoon compile-time domain tests
hoonc --new --arbitrary hoon/tests/names.hoon hoon/

# Rust unit + handler tests (boots the real kernel per test)
cargo +nightly test

# light_verify offline verifier (JSON bundle on stdin — see docs/wallet-verification.md)
cargo +nightly run --bin light_verify -- --help
```

## Settlement, proofs, and open work

Vesl graft state (`registered`, `settled`) lives inside the kernel fragment.
**HTTP does not drive settlement** on this hull — recursive STARK goals,
`nns-gate` scope, settlement posting to Nockchain, and upstream prover
limitations are documented in [`ARCHITECTURE.md`](ARCHITECTURE.md) (§3–§11,
§14).

## Project layout

```
hoon/
  app/app.hoon              v0 kernel: accumulator + %scan-block + Vesl + STARK causes
  lib/vesl-graft.hoon       graft state + dispatcher (copied from vesl)
  lib/vesl-merkle.hoon      merkle primitives (hash-leaf, hash-pair, verify-chunk)
  common/wrapper.hoon       state versioning
  common/zeke.hoon          tip5 hash chain
  common/ztd/               tip5 math tables
  tests/names.hoon          compile-time domain invariant tests
                            (G1 format + G2 Merkle inclusion across tree sizes)
src/
  main.rs                   entrypoint: boot kernel, load config, serve HTTP
  lib.rs                    module wiring
  api.rs                    read-only HTTP (`/status`, `/accumulator/...`)
  kernel.rs                 NounSlab builders for peeks + read-only verify pokes
  state.rs                  AppState + follower telemetry
  payment.rs                fee tiers (shared helpers)
  chain.rs                  chain RPC helpers for the follower
  chain_follower.rs         block-by-block `%scan-block` driver
  claim_note.rs             canonical claim-note NoteData schema helpers
  wallet_y4.rs              lookup bundle types + header-chain verify (`light_verify`)
  types.rs                  JSON / wire types (accumulator responses, etc.)
  bin/light_verify.rs       offline verifier (checkpoint + headers + STARK + snapshot)
scripts/
  parity.py                 legacy vs new API diff tool
tests/
  handlers.rs               full HTTP integration tests
nns.toml                   settlement config
Cargo.toml                  local path deps (../nockchain + ../vesl)
nns.jam                     compiled kernel (built by hoonc)
```

