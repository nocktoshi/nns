ℕℕ𝕊 — The Nockchain Name Service. 

<img width="1376" height="768" alt="image" src="https://github.com/user-attachments/assets/d3361e4a-f783-4135-89bd-57a243ee7c67" />

 <br /> <br />


ℕℕ𝕊 is an on-chain registrar for .nock names on Nockchain. It tracks who owns which names by scanning the chain and building a verifiable accumulator of registrations.

The hull is a **read-only chain scanner** — `GET /health`, `GET /status`, `GET /accumulator/:name` only. Users register `.nock` names by submitting **`nns/v1/claim`** notes to Nockchain; this HTTP service does **not** accept claim POSTs. Offline verification uses **`light_verify`** ([`docs/wallet-verification.md`](docs/wallet-verification.md): pinned checkpoint, headers, recursive STARK, accumulator snapshot — no live Nockchain RPC at verify time). **On-chain claims** need structured **NoteData** on outputs; see [`docs/claim-note-wallet-support.md`](docs/claim-note-wallet-support.md) and [nockchain#85](https://github.com/nockchain/nockchain/pull/85). Deeper architecture, proof model, and roadmap: [`ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Dependencies

```bash
# Nightly Rust
rustup toolchain install nightly
```

## Quick start

```bash
# clone nns repo
git clone https://github.com/nocktoshi/nns.git

# one-time install (installs `nns` into ~/.local/bin)
make install

# start service
nns

# You should now see the nns service running

#Available endpoints:
#  GET /health
#  GET /status
#  GET /accumulator/:name (?wallet_export=1)
#  GET /debug/kernel-state
#✅ ℕℕ𝕊 server listening on http://127.0.0.1:3000
```

`make install` also adds `export PATH="$HOME/.local/bin:$PATH"` to
`~/.zshrc` automatically so `nns` resolves to the installed CLI. You may need to open a new shell.

Check Service with cURL:

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
loads block data from the configured RPC, validates claim claims with
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
`nns-predicates` and are evaluated as claims enter the scan pipeline — see
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
  app/                      NNS kernel sources (in repo)
    app.hoon                v0 kernel: accumulator, %scan-block, Vesl graft, STARK causes
    nns-accumulator.hoon    name z-map + scan cursor (`last-proved-height`, digest, …)
    nns-predicates.hoon     dep-light claim predicates (G1 name rules, fee tiers, …)
    names-test.hoon         compile-time domain tests (G1 format, G2 Merkle fixtures)
    tx-witness.hoon         tx-shape helpers for witness / predicate paths
    recursive-build.hoon    Y3 recursive proof build helpers
    tracer.hoon             STARK trace tooling
    tracer-parity.hoon      trace parity checks
  common/ dat/ jams/ lib/   installed by `nockup` from pins in nockapp.toml
    lib/vesl-*.hoon         Vesl graft, merkle, prover, STARK verifier (subset)
    common/zeke.hoon        tip5 hash chain; wrapper.hoon state versioning; ztd/
  packages/                 nockup download cache (per-commit trees)
src/
  main.rs                   boot kernel, chain follower, HTTP server
  lib.rs                    modules + `nns.toml` / tracing defaults
  api.rs                    read-only HTTP (`/health`, `/status`, `/accumulator/...`)
  kernel.rs                 NounSlab peeks + read-only verify pokes
  state.rs                  AppState + follower telemetry
  chain.rs                  RPC helpers + genesis height
  chain_follower.rs         block-by-block `%scan-block` driver
  claim_note.rs             `nns/v1/claim` NoteData encode/decode
  packed_blob.rs            wallet `%blob` / `%memo` belt decoding
  payment.rs                fee-tier helpers
  wallet_y4.rs              lookup bundles + header-chain verify (`light_verify`)
  freshness.rs              wallet-side anchor freshness checks (stale-proof defense)
  formula_nock.rs           Y3 formula opcode guards for Vesl prover limits
  noun_access.rs            scoped noun accessors over nockvm handles
  types.rs                  JSON / wire types
  bin/light_verify.rs       offline verifier CLI (checkpoint + headers + STARK + snapshot)
docs/                       ARCHITECTURE.md, wallet-verification.md, claim-note guides, …
tests/
  handlers.rs               HTTP integration tests
  phase*.rs, prover.rs, …   anchor, predicates, light_verify, scan-order, grpc fixtures
  common/mod.rs             shared test helpers
  fixtures/                 wire blobs for grpcurl claim tests
.github/workflows/          ci.yml, prover-weekly.yml
Makefile                    `make install`, kernel compile (`hoonc`), `~/.local/bin` wrapper
nockapp.toml                nockup Hoon dependency pins (nockchain + vesl-core commits)
nockapp.lock                resolved nockup install manifest
nns.toml                    runtime config (settlement mode, chain RPC, `[tracing_env]`)
Cargo.toml                  Rust deps (`nocktoshi/nockchain` + `vesl-core`, `dev` branch)
nns.jam                     compiled kernel artifact (`make compile-kernel`)
```

