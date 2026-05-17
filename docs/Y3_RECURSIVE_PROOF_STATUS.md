# Y3 Recursive STARK Proof — Current Status (Path Y, stateless NNS)

**Branch**: `stateless`  
**Last major focus**: Getting a real, non-empty `%recursive-transition-proof` effect that survives `prove-computation:vp` in the current Vesl prover (which only supports Nock opcodes 0-8).

This document is the hand-off point. A new engineer (or future session) should be able to resume exactly here without re-reading the full chat.

---

## 1. Goal of Y3

Produce a **recursive STARK** that proves:

```
verify(prev_recursive_proof)
∧ claim-scanner(old_acc, page, txs) == new_acc
∧ height and digest are monotonic
```

The output is a `%recursive-transition-proof` effect containing a `proof:sp` that a `light_verify` client can check with `verify_stark_explicit_offline`.

Two sub-goals:
- **Genesis** (empty accumulator at `nns-genesis-height`, digest `0x0`) — already working and verifiable.
- **Transition** (real first-writer-wins over a non-empty list of `nns-claim`s) — the part that still panics in the prover.

Because the upstream Vesl prover (`fock:fink:interpret` in `common/ztd/eight.hoon`) does **not** yet support Nock `%9` (call), `%10` (edit), or `%11` (hint), the entire traced formula must be a pure 0-6 Nock tree (nested `[6 [5 …] …]` conditionals, inlined loops, no gate calls).

---

## 2. Key Architectural Decisions (Path Y)

| Decision | Rationale | Status |
|----------|-----------|--------|
| Cons-list + linear first-writer-wins instead of z-map / BST | `~(has z-by …)` and `~(put z-by …)` pull in Nock 9 via the `z` core. Linear scan + hand-written `list-has` / `list-insert-first-wins` stay in 0-8. | Done |
| All list walking, `valid-claim`, and `name-key` inlined into the formula | Eliminates every `%9` opcode from the trace. | Done (see the big inlined block inside `++recursive-transition-formula`) |
| Noun packing of previous proof: `[proof [subj form]]` | Avoids feeding a ~70 KiB raw atom through deep axis navigation. The prover chokes on large flat atoms. | Done in `build-recursive-transition-inputs` |
| "Based" noun representation (the topic of the last several turns) | Nockchain note data uses a canonical shape so the prover's axis/edit operations never see an atom where a cell is expected. Raw `@ux` tx-hashes and plain `@t` strings produce "Invalid axis for edit: NotCell". | Partially done (see §5) |
| Empty-claims fast path (tiny hand-written 0-6 formula) | Proves "no claims in this block → acc unchanged" without the full list machinery. Useful for the first real blocks. | Done |
| `light_verify` verification path via `verify_stark_explicit_offline` + accumulator snapshot + header chain | Y4 wallet story (no live RPC, pinned checkpoint). | Working for genesis; transition proofs not yet produced |

---

## 3. Where the Code Lives (as of the last successful kernel build)

### Hoon — `hoon/app/app.hoon`

- **Types** (around line 478):
  ```hoon
  +$  tx-id        [@ux @ux @ux @ux @ux]   :: 5-limb based form
  +$  based-cord   @t
  +$  transition-claim
    $:  key=nns-name-key:na
        name=based-cord
        owner=based-cord
        tx-id=tx-id
        fee=@ud
    ==
  ```

- **`++based`** (the canonical deep-map encoding from nockchain):
  ```hoon
  ++  based
    |=  a=*
    ^-  *
    ?@  a  a
    [$(a -.a) $(a +.a)]
  ```

- **`++atom-to-tx-id`**:
  ```hoon
  ++  atom-to-tx-id
    |=  a=@ux
    (flop (rip 8 a))   :: 40-byte LE atom → 5-limb cell
  ```

- **`++prekey-claims`** (off-trace, Rust → Hoon conversion):
  ```hoon
  =/  keyed=transition-claim
    :*  (name-key:na name.c)
        (based name.c)
        (based owner.c)
        (based (atom-to-tx-id tx-hash.c))
        fee.c
    ==
  ```

- **`++recursive-transition-formula`** (the giant gate that gets traced):
  - Has an **empty-claims fast path** (tiny 0-6 formula).
  - Has a **non-empty path** that:
    - Receives `packed-prev`, `prev-height`, `acc-list`, `claims`, `pag`.
    - Inlines `list-has`, `list-insert-first-wins`, and a drastically simplified predicate check (the "TEMPORARY SIMPLIFICATION FOR DEBUGGING" block).
    - Builds new cons-list accumulator.
    - Does the height/digest monotonicity checks.
  - The non-empty path is what still crashes.

- **`%prove-recursive-transition` handler** (the poke arm):
  - Calls `build-recursive-transition-inputs`.
  - Packs the previous proof.
  - Calls `prove-computation:vp`.
  - On success commits `recursive-proof.state` and emits the effect.

- Many `~&` debug prints were added at the very start of the handler, after building subject/form, at the top of the formula, and at the start of the non-empty `claims` arm. **None of them have ever fired on a non-empty transition** — the crash is before Hoon code runs.

### Rust — `src/kernel.rs`

- `build_prove_recursive_transition_poke`
- `RecursiveTransitionProof` + `first_recursive_transition_proof`
- `build_recursive_proof_peek` + `decode_recursive_proof`
- The genesis equivalents (`build_prove_recursive_genesis_poke`, etc.)

### Rust — `src/chain_follower.rs`

- Skeleton hook inside `apply_prefetched_scan_blocks_inner` after `%scan-block-done`:
  - Peek `/recursive-proof`
  - Build transition poke
  - Send it
  - Only advance cursor on success
  (Currently mostly logging; not yet driving real blocks.)

### Tests — `tests/prover.rs`

- `y3_genesis_recursive_proof`
- `y3_genesis_proof_verifiable_by_light_verify_path`
- `y3_transition_proof_verifiable_by_light_verify_path`
- `y3_strict_transition_proof_effect` ← the one that asserts a real `%recursive-transition-proof` effect (the one that keeps failing)
- `y3_follower_attempts_recursive_transition_after_scan_block`

---

## 4. The Nock 9/10/11 Blocker (why we are doing all this painful inlining)

The Vesl prover used by `prove-computation:vp` only models opcodes 0-8. Any gate call (`%9`), core edit (`%10`), or hint (`%11`) causes an immediate trap ("Invalid opcode") or a later "Invalid axis for edit: NotCell" when the interpreter tries to continue.

This is why:
- We removed every `++` arm call from the traced path.
- We replaced the z-map with a plain cons-list.
- We inlined `list-has`, `list-insert-first-wins`, and the predicate checks.
- We are forced to keep the transition formula extremely simple until upstream lands Nock 9/10/11 support.

---

## 5. The "Based" Noun Work (the last several iterations)

**User directive** (repeated):
> "you need to base the nouns like: ++ based"
> "you need to bse the following types: name=@t owner=@t tx-id=tx-id fee=@ud"

**What was done**:
- Introduced `+$ tx-id [@ux @ux @ux @ux @ux]` and `+$ based-cord @t` in app.hoon.
- Added the generic `++based` deep-map (exactly the pattern used in nockchain for making nouns prover-friendly).
- Updated `prekey-claims` to wrap every field:
  - `(based name.c)`
  - `(based owner.c)`
  - `(based (atom-to-tx-id tx-hash.c))`
- `atom-to-tx-id` uses the canonical `(flop (rip 8 a))` 5-limb conversion.

**Current state of `nns-accumulator-entry`** (in `hoon/lib/nns-accumulator.hoon`):
Still the old raw form:
```hoon
+$  nns-accumulator-entry
  $:  name=@t
      owner=@t
      tx-hash=@ux
      ...
  ==
```
Inside the formula we still construct entries with raw `name.c owner.c tx-id.c …`.

**Why it hasn't fixed the crash yet**:
The crash ("Invalid axis for edit: NotCell" in `nockvm/src/interpreter.rs:1620`) happens **before any `~&` print executes**. This means the prover fails while constructing the initial trace from the subject noun we hand it — the shape of the sample itself is invalid for the formula's axis expectations.

The most likely remaining source is **Rust-side noun construction**:
- When `build_prove_recursive_transition_poke` (or the test helper) builds the `claims` list and the 5-limb `tx-id` cells, it may be producing left-nested cells or flat atoms instead of the exact right-nested cell tree that Hoon `[@ux @ux @ux @ux @ux]` and the `based` map expect.
- The previous proof packing `[proof [subj form]]` may also have a shape mismatch on the Rust side.

---

## 6. Exact Symptom We Are Debugging

Test: `cargo +nightly test --test prover y3_strict_transition_proof_effect -- --nocapture --ignored`

Typical failure:
```
thread 'y3_strict_transition_proof_effect' panicked at .../interpreter.rs:1620:
Invalid axis for edit: NotCell
```

- Kernel rebuild (`make kernel-jam`) succeeds.
- All `~&` statements inside the `%prove-recursive-transition` handler and inside the formula are **never reached**.
- The empty-claims fast path works (tiny subject).
- Any non-empty `claims` list (even with the "temporary simplification" that removes all predicate checks) produces the crash immediately when `prove-computation:vp` is called.

This strongly indicates the problem is in the **subject noun** that Rust builds for the prover, not in the Hoon logic that runs after the subject is accepted.

---

## 7. Immediate Next Step (user's last explicit request)

> "temporarily simplify the non-empty path to see if we can at least get past the crash"

Combined with the basing work, the concrete next actions are:

1. **Further simplify the non-empty branch** (even more than the current "TEMPORARY SIMPLIFICATION"):
   - Remove the previous-proof verification entirely from the non-empty subject (or make it a no-op stub like the genesis case).
   - Make the subject for a non-empty transition as small as possible — just the `acc-list`, the `claims` list (already pre-keyed and based), and the current page.
   - This mirrors exactly what was done for the empty-claims fast path.

2. **Audit the Rust noun builder** for the transition poke:
   - Look at how `build_prove_recursive_transition_poke` (and the test helper that creates `ClaimCandidate`s) constructs:
     - The 5-limb `tx-id` cells (`[@ux @ux @ux @ux @ux]`).
     - The list of `transition-claim` (the `claims` field).
     - The packed previous proof `[proof [subj form]]`.
   - Ensure every cell is built with the same right-nested structure that Hoon produces (use `cell!` or the equivalent noun builder helpers in the right order).

3. **Update `nns-accumulator-entry` and the entry construction inside the formula** to use the based forms (so that when we later store real entries, they match what the prover expects).

4. Once a non-empty transition proof can be produced (even a trivial one that just inserts one name), re-enable the richer predicate checks one piece at a time.

---

## 8. Longer-Term Roadmap (after we get past the crash)

- When upstream Vesl lands Nock 9/10/11 support, flip the transition formula back to the full `claim-scanner:np` + `verify:sp-verifier` + rich predicates version.
- Y5: Reorg/fork handling — periodic checkpoints of `(acc + recursive-proof + height + digest)`, ability to rewind to last good checkpoint.
- Y6 (optional): Hybrid Rust + STARK — move expensive checks (fee tiers, ownership, format) out of the STARK for throughput; keep only the first-writer-wins uniqueness and the height/digest binding inside the proof.
- Full wallet export path (`/accumulator-jam`, `recursive_*_jam_hex`, header chain) for `light_verify` with real recursive proofs.

---

## 9. How to Reproduce the Current State

```bash
# 1. Rebuild kernel after any Hoon change
make kernel-jam

# 2. Run the strict transition test (the one that demonstrates the crash)
cargo +nightly test --test prover y3_strict_transition_proof_effect -- --nocapture --ignored

# 3. (Optional) run the whole Y3 suite
cargo +nightly test --test prover -- --ignored y3_
```

Useful debug prints are already wired in `app.hoon` inside the `%prove-recursive-transition` arm and at the top of `++recursive-transition-formula`. If any of them appear, the crash has moved into Hoon-level logic.

---

## 10. Files You Will Touch Most Often

- `hoon/app/app.hoon` — the entire Y3 formula and handler live here.
- `hoon/lib/nns-accumulator.hoon` — `nns-accumulator-entry` type (needs basing).
- `src/kernel.rs` — Rust builders for the pokes and the peek decoder.
- `src/chain_follower.rs` — the hook that will eventually drive real transitions after every block.
- `tests/prover.rs` — the four Y3 ignored tests.

---

This document + the plan file (`stateless_nns_tx_primitive_3db35132.plan.md`) + the ARCHITECTURE.md section on Path Y should be sufficient for anyone to continue the work.

**Current blocking question**: "Why does the prover reject the subject noun for any non-empty `claims` list before a single Hoon instruction executes?"

Answer that (most likely by auditing the Rust noun construction of 5-limb cells and the packed previous proof), and the Y3 recursive transition will finally produce a real, verifiable proof.