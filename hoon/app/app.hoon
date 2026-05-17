::  nns — .nock name registrar kernel.
::
::  v0 kernel (`+$v0-state`): `nns-accumulator` + chain-scan cursor
::  (`last-proved-height`, `last-proved-digest`) + `vesl-state` + optional
::  `last-proved` for STARK prove/verify arms. The follower pokes
::  `%scan-block` to merge on-chain `nns/v1/claim` notes in canonical
::  block/tx order; `nns-predicates` / claim-scanner arms enforce name
::  format, fee tiers, uniqueness, payment replay, and chain linkage.
::
::  Vesl graft (`registered`, `settled`) and `nns-gate` back settlement /
::  proof bundles where wired. Prover/verifier causes include
::  `%prove-arbitrary`, `%prove-claim-in-stark`, `%verify-stark`, … — see
::  `+$cause` below.
::
::  Compile: hoonc --new hoon/app/app.hoon hoon/
::
/+  *vesl-graft
/+  *vesl-merkle
/+  vp=vesl-prover
/+  vv=vesl-verifier
/=  np  /app/nns-predicates
/=  na  /app/nns-accumulator
/=  tw  /app/tx-witness
/=  tracer  /app/tracer
/=  trcp  /app/tracer-parity
/=  rb  /app/recursive-build
/=  nv  /common/nock-verifier
/=  four  /common/ztd/four
/=  *  /common/zoon
/=  *  /common/wrapper
::  nockup:imports
::
=>
|%
::
::  First Nockchain block height the hull may `%scan-block` after a
::  fresh kernel (cursor height/digest both zero). Before this height
::  NNS did not exist on-chain.
::
++  nns-genesis-height  63.000
::
::  +$anchor-header: minimal header triple sufficient for parent-chain
::  verification. The full Nockchain page header carries a `proof:vp`,
::  tx-ids z-set, coinbase split, etc. — none of which the kernel needs
::  at Phase 2. We only need enough to walk parent pointers and commit
::  to a specific block-id at a specific height for Phase 3's STARK.
::
+$  anchor-header
  $:  digest=@ux     :: Tip5 hash of this header
      height=@ud    :: page-number
      parent=@ux    :: Tip5 hash of parent header (anchor-tip of genesis is 0)
  ==
::
+$  transition-claim
  $:  key=nns-name-key:na
      cand=nns-claim:np
  ==
::
::  +$anchored-chain: kernel's view of the Nockchain header chain,
::  trimmed to the minimum a zkRollup-style design needs.
::
::  We store ONLY the current follower-anchored tip. Per-claim chain
::  linkage for proofs is wallet-side from a pinned checkpoint (Path Y4),
::  not extra note-data on claims — the hull re-fetches txs/blocks from RPC.
::  The kernel is not a Nockchain-replica and does not cache the full chain.
::
::  Analog: Optimism stores a state root on L1, not L1's headers. The
::  wallet independently trusts Nockchain (for UTXOs anyway); all we
::  need to commit to is "this is the Nockchain tip NNS claims anchor
::  to", and the STARK attests to parent-chain linkage up to it.
::
+$  anchored-chain
  $:  tip-digest=@ux    :: follower-advanced canonical tip (0 = uninitialised)
      tip-height=@ud    :: page-number of tip
  ==
::
::  +$v0-state: Path Y prerelease kernel — z-map accumulator + chain-scan
::  cursor.  Tag `%v0` is the on-disk / jam identity for this shape; it
::  does not refer to the old HTTP-era names map (that stack is gone).
::
+$  v0-state
  $:  %v0
      accumulator=nns-accumulator:na
      last-proved-height=@ud
      last-proved-digest=@ux
      vesl=vesl-state
      ::  Cached (subject, formula) for the most recent successful
      ::  `prove-computation` (%prove-arbitrary, %prove-claim-in-stark,
      ::  %prove-recursive-step). `~` until first prove.
      ::
      last-proved=(unit [subject=* formula=*])
      ::  Path Y3: latest recursive rollup STARK. `~` until a successful
      ::  `%prove-recursive-transition` (or the Y3 genesis bootstrap).
      ::  The `subject` / `formula` are the traced pair for *this* proof;
      ::  `subject` embeds the prior recursive proof for the verify(prev)
      ::  step. Used for wallet bundles and for chaining the next block.
      ::
      recursive-proof=(unit [proof=* subject=* formula=*])
  ==
::
::
+$  effect  *
::
+$  cause
  $%  [%prove-recursive-genesis ~]
      $:  %prove-recursive-transition
          prev-proof-jam=@
          prev-subject-jam=@
          prev-formula-jam=@
          page-digest=@ux
          page-tx-ids=(list @ux)
          claims=(list nns-claim:np)
          block-proof=*
      ==
      ::
      ::  Phase 1-redo: cue JAM and run verify:nv  same jets as
      ::  on-chain block PoW STARK verification . Read-only; for
      ::  benchmarking recursion cost — verify is not inside the
      ::  fink-traced prove-computation subject.
      ::
      ::  Use *  not @  so soft accepts large JAM atoms; cast
      ::  before cue.
      ::
      [%verify-stark blob=*]
      ::  Path Y4 / wallet offline: cue proof plus caller-supplied
      ::  subject-jam and formula-jam atoms  raw JAM bytes of the
      ::  traced nouns , then verify:vesl-stark-verifier — same math as
      ::   pct verify-stark but does not read last-proved.state. Read-only.
      ::
      [%verify-stark-explicit blob=* subject-jam=* formula-jam=*]
      ::  Path Y4: offline z-map membership. Cues acc-jam into an
      ::  nns-accumulator, checks root-atom matches expected-root,
      ::  and that  get acc name  is exactly entry. Read-only.
      ::
      $:  %verify-accumulator-snapshot
          expected-root=@
          acc-jam=@
          name=@t
          owner=@t
          tx-hash=@ux
          claim-height=@ud
          block-digest=@ux
      ==
      ::  Phase 1-redo sanity: prove the identity subject/formula with
      ::  vesl-prover, then verify it with vesl-stark-verifier. Uses
      ::  the exact same shape as vesl/protocol/tests/prove-verify.hoon
      ::  so we can confirm prover<->verifier compatibility independent
      ::  of our batch-specific subject/formula.
      ::
      [%prove-identity ~]
      ::  Path Y2: ingest one Nockchain block worth of nns/v1/claim
      ::  claims. Verifies parent links to last-proved-digest,
      ::  height is the successor of last-proved-height, except on
      ::  genesis boot where it must be at least nns-genesis-height
      ::   NNS shipped long after Nockchain genesis; blocks below that have
      ::  no claim notes . Then folds valid claims into the accumulator via
      ::  claim-scanner:np. On success advances the scan cursor to
      ::  this block’s digest and emits [ pct scan-block-done ...].
      ::
      $:  %scan-block
          parent=@ux
          height=@ud
          page-digest=@ux
          page-tx-ids=(list @ux)
          claims=(list nns-claim:np)
      ==
      ::  Phase 3 Level A: exercise chain-links-to:nns-predicates
      ::  without going through  pct claim. Read-only — the cause does not
      ::  mutate state, it just runs the predicate and emits the
      ::  result. Used by tests + ops tooling to verify a claim’s
      ::  header chain resolves to the kernel’s anchored tip before
      ::  issuing an expensive  pct claim poke.
      ::
      [%verify-chain-link claim-digest=@ux headers=(list anchor-header) anchored-tip=@ux]
      ::  Phase 3 Level B: drive has-tx-in-page:nns-predicates.
      ::  Read-only; emits [ pct tx-in-page-result ok=?] iff
      ::  claimed-tx-id appears in the flat tx-ids list  linear
      ::  scan — no z-silt . The page summary is hull-provided
      ::   Phase 2c fetch_page_for_tx ; Level C will recompute the
      ::  block-commitment from the full page noun.
      ::
      [%verify-tx-in-page digest=@ux tx-ids=(list @ux) claimed-tx-id=@ux]
      ::  Phase 3c: compose all Level A + Level B + G1/C2 predicates
      ::  into one bundled validation call. Read-only — the cause does
      ::  not mutate state. Emits validate-claim-ok on success or
      ::  validate-claim-error plus tag on the first failing
      ::  predicate. The hull uses this pre- pct claim to give users an
      ::  early rejection + structured error tag before committing a
      ::  claim that would only be rejected during chain replay.
      ::
      ::  page-tx-ids is a flat list; inclusion is a list walk in
      ::  has-tx-in-page:np  same as  pct verify-tx-in-page .
      ::
      $:  %validate-claim
          name=@t
          owner=@t
          fee=@ud
          tx-hash=@ux
          claim-block-digest=@ux
          anchor-headers=(list anchor-header)
          page-digest=@ux
          page-tx-ids=(list @ux)
          anchored-tip=@ux
          anchored-tip-height=@ud
          witness-tx-id=@ux
          witness-spender-pkh=@
          witness-treasury-amount=@ud
          witness-output-lock-root=@t    :: v1 output lock root b58 note_name
      ==
      ::  prove-claim-in-stark: same bundle as validate-claim; proves in STARK.
      ::
      ::  Wallet verification then runs vesl-verifier verify on the
      ::  emitted proof against the same subject and formula pair.
      ::  The wallet cross-checks that the subject/formula matches the
      ::  intended property; that closes the trust loop. For NNS that means
      ::  matching the Nock of validate-claim-bundle-linear on the bundle,
      ::  once a canonical encoding is published  see docs on recursive
      ::  payment proof, step 3 Nock-formula encoding .
      ::
      ::  subject-jam and formula-jam are the JAM bytes of the two nouns.
      ::  The kernel cues them before handing to prove-computation, keeping
      ::  the Rust poke-builder side simple  bytes in, bytes out  and the
      ::  kernel in charge of Nock-noun shape.
      ::
      [%prove-arbitrary subject-jam=@ formula-jam=@]
      ::  Phase 3c step 3 completion: proves a claim bundle by
      ::  tracing validate-claim-bundle-linear bundle  INSIDE the
      ::  STARK. Uses the subject-bundled-core encoding from
      ::  build-validator-trace-inputs:np.
      ::
      ::  Emits [%claim-in-stark-proof product proof] on success.
      ::  The product is  each ~ validation-error :np head-tagged:
      ::  [%& ~] iff validation passed, [%| err] on rejection.
      ::  Wallet reads product and proof — no re-running the
      ::  validator. Single-artifact trust.
      ::
      ::  name is the UTF-8 cord from the claim bundle. The Path Y z-map
      ::  does not key rows by raw name; see name-key in nns-accumulator
      ::   Tip5 5-limb digest, same based limb layout as v1 tx-id .
      ::
      $:  %prove-claim-in-stark
          name=@t
          owner=@t
          fee=@ud
          tx-hash=@ux
          claim-block-digest=@ux
          anchor-headers=(list anchor-header)
          page-digest=@ux
          page-tx-ids=(list @ux)
          anchored-tip=@ux
          anchored-tip-height=@ud  ::  Phase 7
      ==
      ::  Y0 recursive-composition spike  legacy notes; vesl-cause follows .
      ::
      ::  nockup:cause
      ::  graft-inject would add vesl-cause here on a fresh kernel.
      ::  Already present below; marker is idempotent.
      ::
      vesl-cause
  ==
::
::  --- Y3 genesis bootstrap helpers ---
::
::  Reserved TLD row: stem `nock` (registry key is the cord `nock`).
::
++  nns-genesis-tld-name  'nock'
::
++  genesis-tld-entry
  ^-  nns-accumulator-entry:na
  :*  nns-genesis-tld-name
      'nock'
      0x1
      nns-genesis-height
      0x0
  ==
::
++  ensure-genesis-tld
  |=  acc=nns-accumulator:na
  ^-  nns-accumulator:na
  ?:  (has:na acc nns-genesis-tld-name)
    acc
  (insert:na acc nns-genesis-tld-name genesis-tld-entry)
::
::  Trace formulas: /app/tracer.hoon. Spec + build: /app/recursive-build.hoon.
::  Parity oracles: /app/tracer-parity.hoon.
::
::  Local arm that performs the subject-bundled verify of a previous
::  recursive proof (the same technique from the old Y0 spike, now
::  part of the real transition).
::
::  NOTE: The version above uses a gate call (Nock 9) and will trap in the
::  current prover.  For producing real transition proofs without Nock 9/10/11
::  support we use the pure 0-6 version below.
::
++  verify-previous-recursive-proof
  |=  [prev-proof=* prev-subj=* prev-form=*]
  ^-  ?
  =/  prf=proof:vp  ;;(proof:vp prev-proof)
  (verify:vv prf ~ 0 prev-subj prev-form)
::
::  Pure Nock 0-6 version of the above (no gate calls).
::  Accepts any non-empty previous proof with a positive height.
::  This lets us produce a real %recursive-transition-proof even before
::  Nock 9/10/11 support lands.
::
:: ++  verify-previous-recursive-proof-pure
::   |=  [prev-proof=* prev-subj=* prev-form=* prev-height=@ud]
::   ^-  ?
::   ?&  (gth (met 3 prev-proof) 0)
::       (gth prev-height 0)
::   ==

++  verify-previous-recursive-proof-axis
  ^~
  =/  probe  !=(verify-previous-recursive-proof)
  =/  inner=*
    ?.  ?=([%11 * *] probe)  probe
    +>.probe
  ?>  ?=([@ @ *] inner)
  +<.inner
::
::  Axis for the main transition formula (must be defined before the
::  build function that uses it).
::
++  transition-spec-axis
  ^~
  =/  probe  !=(transition-spec:rb)
  =/  inner=*
    ?.  ?=([%11 * *] probe)  probe
    +>.probe
  ?>  ?=([@ @ *] inner)
  +<.inner
::
::  Build the subject+formula for the real per-block transition prove.
::
::  +nns-gate: verification gate for %vesl-settle / %vesl-verify.
::
::    data:          (list [name=@t owner=@t tx-hash=@t proof=(list proof-node)])
::    expected-root: Merkle root that every `proof` is claimed to cover
::
::  G1: every leaf's name has valid format.
::  G2: for every leaf, `jam [name owner tx-hash]` hashed as a leaf
::      and walked through `proof` equals `expected-root`.
::  G3: no duplicate `name` within this transition batch.
::  G4: no duplicate `tx-hash` within this transition batch.
::
::  The graft supplies `expected-root` from the registered hull
::  root, so a verified `nns-gate` invocation proves: "these
::  (name, owner, tx-hash) triples were all registry rows at the
::  commitment `expected-root`."  An empty leaves list is rejected
::  at the %settle-batch layer before it ever reaches the gate, but
::  the gate itself accepts the vacuous case (nothing to disprove)
::  so a direct %vesl-verify on an empty batch is a no-op success.
::  No payment checking here — that's on the hot path and payment
::  attestation is a separate concern (see README TODO).
::
++  nns-gate
  |=  [data=* expected-root=@]
  ^-  ?
  =/  leaves
    ;;((list [name=@t owner=@t tx-hash=@t proof=(list [hash=@ side=?])]) data)
  =|  seen-names=(set @t)
  =|  seen-tx-hashes=(set @t)
  |-  ^-  ?
  ?~  leaves  %.y
  =/  chunk=@  (jam [name.i.leaves owner.i.leaves tx-hash.i.leaves])
  ?&  (is-valid-name:np name.i.leaves)
      !(~(has in seen-names) name.i.leaves)
      !(~(has in seen-tx-hashes) tx-hash.i.leaves)
      (verify-chunk chunk proof.i.leaves expected-root)
      %=  $
        leaves  t.leaves
        seen-names  (~(put in seen-names) name.i.leaves)
        seen-tx-hashes  (~(put in seen-tx-hashes) tx-hash.i.leaves)
      ==
  ==
::
++  stark-bind
  |=  state=v0-state
  ^-  [@ @]
  :*  (root-atom:na accumulator.state)
      last-proved-height.state
  ==
++  moat  (keep v0-state)
::
++  inner
  |_  state=v0-state
  ::
  ++  load
    |=  old-state=v0-state
    ^-  _state
    old-state
  ::
  ::  +peek: Path Y accumulator + graft state
  ::
  ::    /accumulator/<name>  -> (unit nns-accumulator-entry)
  ::    /accumulator-root    -> @ (lossy atom of Tip5 z-map tip)
  ::    /accumulator-jam     -> @ (jam of full nns-accumulator noun)
  ::    /scan-state          -> [height=@ud digest=@ux root=@ size=@ud]
  ::    /fee-for-name/<n>    -> @ud
  ::    /kernel-debug        -> fixed tuple for HTTP debug (see Rust decode)
  ::    [anything else]      -> vesl-peek
  ::
  ++  peek
    |=  =path
    ^-  (unit (unit *))
    =/  parity=(unit (unit *))
      (peek-tracer-parity:trcp path (ensure-genesis-tld accumulator.state) last-proved-height.state last-proved-digest.state nns-genesis-height)
    ?^  parity  parity
    ?+  path  (vesl-peek vesl.state path)
        [%kernel-debug ~]
      =/  acc=(list [name=@t nns-accumulator-entry:na])
        (to-list:na accumulator.state)
      =/  acc-out=(list [name=@t owner=@t tx=@ux claim-height=@ud block-digest=@ux])
        %+  turn  acc
        |=  [name=@t en=nns-accumulator-entry:na]
        [name owner.en tx-hash.en claim-height.en block-digest.en]
      =/  acc-sorted
        %+  sort  acc-out
        |=  [[a=@t *] [b=@t *]]
        (lth a b)
      =/  reg-list=(list [@ @])
        %+  sort  ~(tap by registered.vesl.state)
        |=  [a=[h=@ *] b=[h=@ *]]
        (lth h.a h.b)
      =/  settled-list=(list @)
        %+  sort  ~(tap in settled.vesl.state)
        |=  [a=@ b=@]
        (lth a b)
      =/  lp-out=(unit [@ @])
        ?~  last-proved.state
          ~
        [~ [(jam subject.u.last-proved.state) (jam formula.u.last-proved.state)]]
      ::  [ver=@ud h=@ud digest=@ux root=@ size=@ud acc reg settled lp]
      ``[0 last-proved-height.state last-proved-digest.state (root-atom:na accumulator.state) (size:na accumulator.state) acc-sorted reg-list settled-list lp-out]
        ::
        [%accumulator name=@t ~]
      =/  key=@t  +<.path
      ``(get:na [accumulator.state key])
        ::
        [%accumulator-proof name=@t ~]
      =/  key=@t  +<.path
      ``(proof-axis:na [accumulator.state key])
        ::
        [%accumulator-root ~]
      ``(root-atom:na accumulator.state)
        ::
        [%accumulator-jam ~]
      ``(jam accumulator.state)
        ::
        [%scan-state ~]
      ``[ last-proved-height.state
            last-proved-digest.state
            (root-atom:na accumulator.state)
            (size:na accumulator.state)
        ]
        ::
        [%recursive-proof ~]
      ?~  recursive-proof.state
        ``~
      =/  rp  u.recursive-proof.state
      ``[(jam proof.rp) (jam subject.rp) (jam formula.rp)]
        ::
        [%fee-for-name name=@t ~]
      =/  key=@t  +<.path
      ``(fee-for-name:np key)
    ==
  ::
  ++  poke
    |=  =ovum:moat
    ^-  [(list effect) _state]
    =/  act  ((soft cause) cause.input.ovum)
    ?~  act
      ~>  %slog.[3 'nns: invalid cause']
      :_  state
      ~[[%invalid-cause ~]]
    ?-  -.u.act
        ::
        ::  Path Y2: %scan-block — parent link + height monotonicity,
        ::  then `+claim-scanner:np` over the supplied claims.
        ::
        %scan-block
      =/  c  u.act
      =/  boot=?
        &(=(0 last-proved-height.state) =(0 last-proved-digest.state))
      ?.  ?|(boot =(parent.c last-proved-digest.state))
        :_  state
        ~[[%scan-block-error 'parent-mismatch']]
      =.  accumulator.state
        ?:  boot
          (ensure-genesis-tld accumulator.state)
        accumulator.state
      =/  want-height=@ud
        ?:  boot
          (max +(last-proved-height.state) nns-genesis-height)
        +(last-proved-height.state)
      ?.  =(height.c want-height)
        :_  state
        ~[[%scan-block-error 'height-not-successor']]

      ::  Claim-scanner then accumulator Tip5 root (split `mule`s so a
      ::  trap in either phase surfaces a distinct `%scan-block-error`
      ::  tag — see `claim-scanner-trap` vs `accumulator-root-trap`).
      ::  Page summary uses a flat tx-id list (see `has-tx-in-page:np`)
      ::  — no `z-silt` / `gor-tip` path (jet edge case with 3+ 40-byte
      ::  atoms in z-sets).
      ::
      =/  pag=nns-page-summary:np  [page-digest.c page-tx-ids.c]
      =/  acc-run
        %-  mule
        |.
        (claim-scanner:np accumulator.state pag height.c claims.c)
      ?.  ?=(%& -.acc-run)
        ~>  %slog.[2 'nns: %scan-block claim-scanner trapped']
        :_  state
        ~[[%scan-block-error 'claim-scanner-trap']]
      =/  new-acc=nns-accumulator:na  p.acc-run
      =/  root-run
        %-  mule
        |.
        (root-atom:na new-acc)
      ?.  ?=(%& -.root-run)
        ~>  %slog.[2 'nns: %scan-block accumulator root-atom trapped']
        :_  state
        ~[[%scan-block-error 'accumulator-root-trap']]
      =/  acc-root=@  p.root-run
      =.  accumulator.state  new-acc
      =.  last-proved-height.state  height.c
      =.  last-proved-digest.state  page-digest.c
      :_  state
      ~[[%scan-block-done height.c page-digest.c acc-root]]
      ::
        ::  Sanity-check arm: prove `[42 [0 1]]` then verify. Emits
        ::  [%prove-identity-result ok=?] so the test can confirm the
        ::  prover/verifier round-trip works at all.
        ::
        %prove-identity
      =/  subj=*  42
      =/  form=*  [0 1]
      =/  res
        %-  mule  |.
        (prove-computation:vp subj form 1 1)
      ?.  ?=(%& -.res)
        :_  state
        ~[[%prove-identity-result %.n]]
      =/  pr  p.res
      ?.  ?=(%& -.pr)
        :_  state
        ~[[%prove-identity-result %.n]]
      =/  prf=proof:vp  p.pr
      ::  NB: Phase 1-redo finding — vesl-prover bypasses puzzle-nock
      ::  and standard `verify:nv` derives `[s f]` from puzzle-nock,
      ::  so this round-trip currently fails composition eval. The
      ::  matched verifier is `verify:vv` from vendored vesl-verifier,
      ::  but making it accept our proof requires further investigation
      ::  of stark-config injection. Tracked in the research memo.
      ::
      =/  ok=?  (verify:vv prf ~ 0 subj form)
      :_  state
      ~[[%prove-identity-result ok]]
      ::
        %verify-stark
      ?.  ?=(@ blob.u.act)
        :_  state
        ~[[%verify-stark-error 'blob-not-atom']]
      =/  jammy=@  blob.u.act
      =/  cue-res  (mule |.((cue jammy)))
      ?.  -.cue-res
        :_  state
        ~[[%verify-stark-error 'bad-jam']]
      =/  proof=proof:four  ;;(proof:four +.cue-res)
      ::  Replay the exact [s f] the prover traced. vesl-stark-verifier
      ::  takes them externally (bypasses puzzle-nock). We cache them
      ::  in last-proved on every successful prove poke.
      ::
      ?~  last-proved.state
        :_  state
        ~[[%verify-stark-error 'no-cached-sf']]
      =/  subject=*  subject.u.last-proved.state
      =/  formula=*  formula.u.last-proved.state
      =/  ok=?  (verify:vv proof ~ 0 subject formula)
      :_  state
      ~[[%verify-stark-result ok]]
      ::
        %verify-stark-explicit
      ?.  ?=(@ blob.u.act)
        :_  state
        ~[[%verify-stark-error 'blob-not-atom']]
      ?.  ?=(@ subject-jam.u.act)
        :_  state
        ~[[%verify-stark-error 'subject-jam-not-atom']]
      ?.  ?=(@ formula-jam.u.act)
        :_  state
        ~[[%verify-stark-error 'formula-jam-not-atom']]
      =/  jammy=@  blob.u.act
      =/  cue-res  (mule |.((cue jammy)))
      ?.  -.cue-res
        :_  state
        ~[[%verify-stark-error 'bad-jam']]
      =/  proof=proof:four  ;;(proof:four +.cue-res)
      =/  subject-cue  (mule |.((cue subject-jam.u.act)))
      ?.  -.subject-cue
        :_  state
        ~[[%verify-stark-error 'bad-subject-jam']]
      =/  formula-cue  (mule |.((cue formula-jam.u.act)))
      ?.  -.formula-cue
        :_  state
        ~[[%verify-stark-error 'bad-formula-jam']]
      =/  subject=*  p.subject-cue
      =/  formula=*  p.formula-cue
      =/  ok=?  (verify:vv proof ~ 0 subject formula)
      :_  state
      ~[[%verify-stark-result ok]]
      ::
        %verify-accumulator-snapshot
      ::  expected-root / acc-jam are already `@` on the $cause mold; do not
      ::  test `?=(@ ...)` here — mint-vain (dead branch) under current Hoon.
      ::
      =/  acc-cue  (mule |.((cue acc-jam.u.act)))
      ?.  -.acc-cue
        :_  state
        ~[[%accumulator-snapshot-verify-error 'bad-acc-jam']]
      =/  acc=nns-accumulator:na  ;;(nns-accumulator:na +.acc-cue)
      ?.  =((root-atom:na acc) expected-root.u.act)
        :_  state
        ~[[%accumulator-snapshot-verify-result %.n]]
      =/  entry=nns-accumulator-entry:na
        :*  name=name.u.act
            owner=owner.u.act
            tx-hash=tx-hash.u.act
            claim-height=claim-height.u.act
            block-digest=block-digest.u.act
        ==
      =/  got=(unit nns-accumulator-entry:na)  (get:na [acc name.u.act])
      =/  ok=?  =(got [~ entry])
      :_  state
      ~[[%accumulator-snapshot-verify-result ok]]
      ::
        ::  %verify-chain-link: read-only Phase 3 Level A predicate
        ::  smoke test. Returns `[%chain-link-result ok=?]` without
        ::  mutating state.
        ::
        %verify-chain-link
      =/  ok=?
        %-  chain-links-to:np
        :*  claim-digest.u.act
            headers.u.act
            anchored-tip.u.act
        ==
      :_  state
      ^-  (list effect)
      ~[[%chain-link-result ok]]
      ::
        ::  %verify-tx-in-page: read-only Phase 3 Level B predicate
        ::  smoke test. Runs `has-tx-in-page:np` on `[digest tx-ids]`.
        ::  Returns `[%tx-in-page-result ok=?]` without mutating state.
        ::
        %verify-tx-in-page
      =/  pag=nns-page-summary:np  [digest.u.act tx-ids.u.act]
      =/  ok=?  (has-tx-in-page:np pag claimed-tx-id.u.act)
      :_  state
      ^-  (list effect)
      ~[[%tx-in-page-result ok]]
      ::
        ::  %validate-claim: Phase 3c gate validator. Composes Level A
        ::  + Level B + G1/C2 predicates on the full claim bundle.
        ::  Read-only; emits `[%validate-claim-ok]` on success or
        ::  `[%validate-claim-error <tag>]` where <tag> names the
        ::  first predicate that rejected. State is not mutated.
        ::
        %validate-claim
      =/  pag=nns-page-summary:np  [page-digest.u.act page-tx-ids.u.act]
      =/  wit=nns-raw-tx-witness:np
        :*  witness-tx-id.u.act
            witness-spender-pkh.u.act
            witness-treasury-amount.u.act
            witness-output-lock-root.u.act
        ==
      =/  bundle=claim-bundle:np
        :*  name.u.act
            owner.u.act
            fee.u.act
            tx-hash.u.act
            claim-block-digest.u.act
            anchor-headers.u.act
            pag
            anchored-tip.u.act
            anchored-tip-height.u.act
            wit
        ==
      =/  res=(each ~ validation-error:np)
        (validate-claim-bundle:np bundle)
      ?-  -.res
          %&
        :_  state
        ^-  (list effect)
        ~[[%validate-claim-ok ~]]
      ::
          %|
        :_  state
        ^-  (list effect)
        ~[[%validate-claim-error p.res]]
      ==
      ::
        ::  %prove-arbitrary: trace an arbitrary [subject formula] via
        ::  `prove-computation:vp` and emit a proof bound to
        ::  `+stark-bind` (accumulator root + scan height). No validation
        ::  — caller is responsible for
        ::  constructing the pair.
        ::
        ::  Emits `[%arbitrary-proof product proof]` on prover
        ::  success (product is what the formula evaluated to on the
        ::  subject) or `[%prove-failed trace]` on crash. Caches
        ::  `(subject, formula)` in `last-proved` so subsequent
        ::  `%verify-stark` pokes find the right replay inputs.
        ::
        ::  This is the Phase 3c step 3 primitive — see `docs/PROOF_STORAGE.md`
        ::  §"What the current proof attests to".
        ::
        %prove-arbitrary
      =/  subject-cue  (mule |.((cue subject-jam.u.act)))
      ?.  ?=(%& -.subject-cue)
        :_  state
        ~[[%prove-failed (jam p.subject-cue)]]
      =/  formula-cue  (mule |.((cue formula-jam.u.act)))
      ?.  ?=(%& -.formula-cue)
        :_  state
        ~[[%prove-failed (jam p.formula-cue)]]
      =/  subj=*  p.subject-cue
      =/  form=*  p.formula-cue
      =/  [br=@ bh=@]  (stark-bind state)
      =/  attempt
        %-  mule  |.
        (prove-computation:vp subj form br bh)
      ?.  ?=(%& -.attempt)
        :_  state
        ^-  (list effect)
        ~[[%prove-failed (jam p.attempt)]]
      =/  pr  p.attempt
      ?.  ?=(%& -.pr)
        :_  state
        ^-  (list effect)
        ~[[%prove-failed (jam p.pr)]]
      =/  the-proof=proof:vp  p.pr
      ::  Run the formula directly to capture the evaluated product
      ::  for inclusion in the emitted effect. Same semantics as the
      ::  STARK's trace — `.*` and `fink:fock` agree on products,
      ::  they only differ in whether the execution is traced.
      ::
      =/  product=*  .*(subj form)
      =.  last-proved.state  `[subj form]
      :_  state
      ^-  (list effect)
      ~[[%arbitrary-proof product the-proof]]
      ::
        ::  %prove-claim-in-stark: Phase 3c step 3 completion.
        ::  Builds the subject+formula pair via the nns-predicates
        ::  library, runs prove-computation, emits the trace's
        ::  committed product (the validator's return value) alongside
        ::  the STARK. Wallet verifies proof, reads product — no
        ::  validator re-run required.
        ::
        %prove-claim-in-stark
      =/  bundle=claim-bundle-linear:np
        :*  name.u.act
            owner.u.act
            fee.u.act
            tx-hash.u.act
            claim-block-digest.u.act
            anchor-headers.u.act
            page-digest.u.act
            page-tx-ids.u.act
            anchored-tip.u.act
            anchored-tip-height.u.act
        ==
      =/  [subj=* form=*]  (build-validator-trace-inputs:np bundle)
      ::
      ::  Dry-run outside the STARK to catch validator-level bugs
      ::  before paying for a prover run. `.*` on the raw nockvm
      ::  supports the full Nock opcode set, unlike `fink:fock`
      ::  (which is restricted to opcodes 0-8 for STARK-tractability).
      ::  The validator body uses Nock 9 (slam) and Nock 10 (edit)
      ::  via the subject-bundled-core encoding — those ops are
      ::  currently `!!` in `common/ztd/eight.hoon` under Vesl's
      ::  prover, so the `prove-computation` call below will trap
      ::  until upstream Vesl extends `interpret`.
      ::
      =/  dry-run
        %-  mule  |.  .*(subj form)
      ?.  ?=(%& -.dry-run)
        :_  state
        ^-  (list effect)
        ~[[%prove-failed (jam p.dry-run)]]
      =/  [br2=@ bh2=@]  (stark-bind state)
      =/  attempt
        %-  mule  |.
        (prove-computation:vp subj form br2 bh2)
      ?.  ?=(%& -.attempt)
        :_  state
        ^-  (list effect)
        ~[[%prove-failed (jam p.attempt)]]
      =/  pr  p.attempt
      ?.  ?=(%& -.pr)
        :_  state
        ^-  (list effect)
        ~[[%prove-failed (jam p.pr)]]
      =/  the-proof=proof:vp  p.pr
      =/  product=*  .*(subj form)
      =.  last-proved.state  `[subj form]
      :_  state
      ^-  (list effect)
      ~[[%claim-in-stark-proof product the-proof]]
      ::
        ::  Y3 genesis bootstrap. Prove the base-case formula that
        ::  attests the empty starting state. On success we store the
        ::  proof in `recursive-proof.state` so the first real
        ::  `%scan-block` can chain from it.
        ::
        %prove-recursive-genesis
      ::  Real Y3 base case. Seed the reserved TLD, then prove genesis
      ::  height/digest with that accumulator. No prior proof is verified.
      =.  accumulator.state  (ensure-genesis-tld accumulator.state)
      =/  [subj=* form=*]
        (build-genesis-recursive-inputs:rb accumulator.state nns-genesis-height 0x0)
      =/  dry-ok=?
        =/  dry-run
          %-  mule  |.  .*(subj form)
        ?.  ?=(%& -.dry-run)  %.n
        (trace-succeeded:tracer p.dry-run)
      =/  [br3=@ bh3=@]  (stark-bind state)
      =/  attempt
        %-  mule  |.
        (prove-computation:vp subj form br3 bh3)
      ?.  ?=(%& -.attempt)
        :_  state
        ^-  (list effect)
        :~  [%genesis-recursive-dry-run-ok dry-ok]
            [%prove-failed (jam p.attempt)]
        ==
      =/  pr  p.attempt
      ?.  ?=(%& -.pr)
        :_  state
        ^-  (list effect)
        :~  [%genesis-recursive-dry-run-ok dry-ok]
            [%prove-failed (jam p.pr)]
        ==
      =/  the-proof=proof:vp  p.pr
      =.  last-proved.state  `[subj form]
      =.  recursive-proof.state  `[the-proof subj form]
      ::  Do not advance the scan cursor here. `last-proved-height` /
      ::  `last-proved-digest` stay at genesis boot (0 / 0x0) until the
      ::  first `%scan-block` links to Nockchain; otherwise the follower
      ::  prefetches height 63001 with parent checked against 0x0 and fails.
      :_  state
      ^-  (list effect)
      :~  [%genesis-recursive-dry-run-ok dry-ok]
          [%genesis-recursive-proof the-proof]
      ==
      ::
        ::  Y3: %prove-recursive-transition — the real per-block recursive step.
        ::  Cues the previous proof triple, builds the transition subject using
        ::  the current accumulator + the new page/claims/block-proof,
        ::  runs prove-computation on the 0–8 trace formula from
        ::  ++build-recursive-transition-inputs (spec: ++transition-spec), and on
        ::  success commits the new proof into recursive-proof.state.
        ::
        %prove-recursive-transition
      =/  p  u.act
      =/  claims=(list nns-claim:np)
        ;;((list nns-claim:np) claims.p)
      =/  pag=nns-page-summary:np
        [page-digest.p ;;((list @ux) page-tx-ids.p)]
      =/  prev-proof=*
        ?:  =(0 prev-proof-jam.p)
          ?~  recursive-proof.state
            ~|(%no-recursive-proof-to-chain !!)
          proof.u.recursive-proof.state
        =/  cue-res  (mule |.((cue prev-proof-jam.p)))
        ?.  ?=(%& -.cue-res)
          ~|(%prev-proof-cue-failed !!)
        p.cue-res
      =/  prev-subj=*
        ?:  =(0 prev-subject-jam.p)
          ?~  recursive-proof.state
            ~|(%no-recursive-proof-to-chain !!)
          subject.u.recursive-proof.state
        =/  cue-res  (mule |.((cue prev-subject-jam.p)))
        ?.  ?=(%& -.cue-res)
          ~|(%prev-subject-cue-failed !!)
        p.cue-res
      =/  prev-form=*
        ?:  =(0 prev-formula-jam.p)
          ?~  recursive-proof.state
            ~|(%no-recursive-proof-to-chain !!)
          formula.u.recursive-proof.state
        =/  cue-res  (mule |.((cue prev-formula-jam.p)))
        ?.  ?=(%& -.cue-res)
          ~|(%prev-formula-cue-failed !!)
        p.cue-res
      =/  prev-h=@ud
        ?:  (gth last-proved-height.state 0)
          last-proved-height.state
        nns-genesis-height
      =/  [subj=* form=*]
        %-  build-recursive-transition-inputs:rb
        :*  prev-proof
            prev-subj
            prev-form
            prev-h
            accumulator.state
            pag
            claims
            block-proof.p
            digest.pag
        ==
      ~&  ['chained' 'claims' (lent claims) 'subj' (met 3 (jam subj)) 'form' (met 3 (jam form))]
      =/  dry-ok=?
        =/  dry-run
          %-  mule  |.  .*(subj form)
        ?.  ?=(%& -.dry-run)
          ~&  ['chained' 'dry-run-failed']
          %.n
        (trace-succeeded:tracer p.dry-run)
      =/  [br3=@ bh3=@]  (stark-bind state)
      =/  attempt
        %-  mule  |.
        (prove-computation:vp subj form br3 bh3)
      ?.  ?=(%& -.attempt)
        :_  state
        ^-  (list effect)
        :~  [%recursive-transition-dry-run-ok dry-ok]
            [%prove-failed (jam p.attempt)]
        ==
      =/  pr  p.attempt
      ?.  ?=(%& -.pr)
        :_  state
        ^-  (list effect)
        :~  [%recursive-transition-dry-run-ok dry-ok]
            [%prove-failed (jam p.pr)]
        ==
      =/  the-proof=proof:vp  p.pr
      =.  last-proved.state  `[subj form]
      =.  recursive-proof.state  `[the-proof subj form]
      :_  state
      ^-  (list effect)
      :~  [%recursive-transition-dry-run-ok dry-ok]
          [%recursive-transition-proof the-proof]
      ==
      ::
        ::  vesl-cause tags — delegate to the graft with nns-gate.
        ::  %vesl-register is normally driven by %claim above; a
        ::  direct poke is kept for tests / manual re-registration
        ::  of historical roots.
        ::
        %vesl-register
      =^  efx=(list vesl-effect)  vesl.state
        (vesl-poke vesl.state u.act nns-gate)
      :_  state
      ^-  (list effect)
      efx
      ::
        %vesl-verify
      =^  efx=(list vesl-effect)  vesl.state
        (vesl-poke vesl.state u.act nns-gate)
      :_  state
      ^-  (list effect)
      efx
      ::
        %vesl-settle
      =^  efx=(list vesl-effect)  vesl.state
        (vesl-poke vesl.state u.act nns-gate)
      :_  state
      ^-  (list effect)
      efx
      ::
      ::  nockup:poke
      ::  graft-inject would add the three `%vesl-register` /
      ::  `%vesl-verify` / `%vesl-settle` arms here on a fresh
      ::  kernel. Already present above; marker is idempotent.
      ::
    ==
  --
--
((moat |) inner)
