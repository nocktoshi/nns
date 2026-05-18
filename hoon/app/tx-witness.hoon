::  lib/tx-witness.hoon — narrow Nockchain page / tx-set helpers for NNS.
::
::  Duplicates the *hashing* shape of `page:tx-engine-1` / `z-set` walks
::  from `nockchain/hoon/common/tx-engine-{0,1}.hoon` without importing the
::  full tx-engine cone (see `nns-predicates.hoon` and `scripts/setup-hoon-tree.sh`).
::
::  Type layout matches tx-engine-1 exactly:
::    ++  hash      hash:v0       ::  [@ux @ux @ux @ux @ux]
::    ++  block-id  block-id:v0   ::  alias of hash (page digest)
::
::  Included today:
::    - `++block-commitment` — same field order as `+hashable-block-commitment`
::      on a v1 page body (parent, tx-ids, coinbase, timestamp, epoch-counter,
::      target, accumulated-work, height, msg). `target` / `accumulated-work`
::      are opaque nouns (`*`) so we stay aligned with whatever bignum shape
::      the chain jams.
::    - `++has-tx-in-ids` — `~(has z-in …)` over a `(z-set tx-id)`.
::
/=  *  /common/zeke
/=  *  /common/zoon
|%
+|  %digest-atom
++  pad-digest-atom-40
  |=  d=@
  ^-  @
  ?:  (gte (met 3 d) 40)
    d
  (rap 3 (weld (rip 3 d) (reap (sub 40 (met 3 d)) 0)))
::
+|  %hash-types
::  Mirrors `/common/tx-engine-0` `++hash` (re-exported as `++hash` / `++block-id`
::  in `/common/tx-engine-1`). Five Goldilocks belts — STARK-safe on deep axis picks.
::  Flattened into trace subjects in `/app/tracer.hoon` (`++empty-transition-subject`,
::  `++full-transition-subject`; see `++based:hash:tw` and `++compile-trace`).
::
++  hash
  =<  form
  |%
  ++  form
    $+  noun-digest
    [@ux @ux @ux @ux @ux]
  ::
  ++  based
    |=  has=form
    ^-  ?
    =+  [a=@ b=@ c=@ d=@ e=@]=has
    ?&  (^based a)
        (^based b)
        (^based c)
        (^based d)
        (^based e)
    ==
  ::
  ++  to-list
    |=  bid=form
    ^-  (list @ux)
    =+  [a=@ux b=@ux c=@ux d=@ux e=@ux]=bid
    ~[a b c d e]
  ::
  ::  Coerce a hull `@ux` page digest into `+$hash`. Accepts an existing five-tuple
  ::  or a raw atom (40-byte LE from gRPC / `hash_to_atom_bytes`, or minimal genesis
  ::  `0` padded to 40 bytes — same semantics as Rust `five_limbs`).
  ++  from-hull-atom
    |=  d=*
    ^-  form
    ?^  d
      ?>  ?=([@ux @ux @ux @ux @ux] d)
      d
    ^-  form
    =/  buf=@  (pad-digest-atom-40 d)
    :*  `@ux`(cut 3 [0 8] buf)
        `@ux`(cut 3 [8 8] buf)
        `@ux`(cut 3 [16 8] buf)
        `@ux`(cut 3 [24 8] buf)
        `@ux`(cut 3 [32 8] buf)
    ==
  --
::
::  Page / block digest (tx-engine-1: `digest=block-id` on `+$page`).
::
++  block-id
  =<  form
  |%
  ++  form  hash
  ++  based  based:hash
  ++  from-hull-atom  from-hull-atom:hash
  --
::
++  tx-id
  =<  form
  |%
  ++  form  hash
  ++  from-hull-atom  from-hull-atom:hash
  --
::
+$  coins  @ud
+$  page-number  @ud
::
::  v1 coinbase split: lock-hash -> coins (matches `coinbase-split:page:t`).
::
+$  coinbase-split-v1  (z-map hash coins)
::
::  Minimal v1 page *tail* used for block commitment (everything after pow).
::
+$  page-commit-tail
  $:  parent=hash
      tx-ids=(z-set tx-id)
      coinbase=coinbase-split-v1
      timestamp=@
      epoch-counter=@ud
      target=*
      accumulated-work=*
      height=page-number
      msg=*
  ==
::
++  hashable-tx-ids
  |=  tx-ids=(z-set tx-id)
  ^-  hashable:tip5:z
  ?~  tx-ids  leaf+tx-ids
  :+  hash+n.tx-ids
    $(tx-ids l.tx-ids)
  $(tx-ids r.tx-ids)
::
++  hashable-coinbase-split-v1
  |=  form=coinbase-split-v1
  ^-  hashable:tip5:z
  ?~  form  leaf+form
  :+  [hash+p.n.form leaf+q.n.form]
    $(form l.form)
  $(form r.form)
::
++  hashable-block-commitment
  |=  =page-commit-tail
  ^-  hashable:tip5:z
  :*  hash+parent.page-commit-tail
      hash+(hash-hashable:tip5:z (hashable-tx-ids tx-ids.page-commit-tail))
      hash+(hash-hashable:tip5:z (hashable-coinbase-split-v1 coinbase.page-commit-tail))
      leaf+timestamp.page-commit-tail
      leaf+epoch-counter.page-commit-tail
      leaf+target.page-commit-tail
      leaf+accumulated-work.page-commit-tail
      leaf+height.page-commit-tail
      leaf+msg.page-commit-tail
  ==
::
::  +block-commitment: Tip5 digest of the block body commitment (no pow).
::
++  block-commitment
  |=  =page-commit-tail
  ^-  noun-digest:tip5:z
  (hash-hashable:tip5:z (hashable-block-commitment page-commit-tail))
::
::  +has-tx-in-ids: membership in the canonical tx-id z-set.
::
++  has-tx-in-ids
  |=  [tx-ids=(z-set tx-id) tid=tx-id]
  ^-  ?
  (~(has z-in tx-ids) tid)
::
::  +digest-to-ux: flatten a Tip5 digest to a single atom (matches
::  `digest-to-atom:tip5` use sites in the Rust hull).
::
++  digest-to-ux
  |=  d=noun-digest:tip5:z
  ^-  @ux
  (digest-to-atom:tip5:z d)
--
