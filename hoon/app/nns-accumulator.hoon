::  lib/nns-accumulator.hoon — NNS z-map accumulator for Path Y.
::
::  The Path Y ("recursive rollup") plan replaces `names=(map @t
::  name-entry)` plus `tx-hashes=(set @t)` with a single authenticated
::  z-map keyed by a **Tip5 5-limb digest** of the `.nock` name cord
::  (same ``based`` atom layout as v1 `tx-id` / block `hash` in
::  `tx-witness.hoon`), not by the raw `@t` atom. Raw cords can exceed
::  Goldilocks belt width; `~(put z-by …)` / Tip5 jets expect belt-shaped
::  key limbs like note-data / tx-id hashing paths.
::
::  Each row still carries `name=@t` on the entry for HTTP peeks and
::  canonical `++hashable-accumulator` ordering (lexicographic cord).
::
/=  *  /common/zoon
|%
::
::  +$nns-name-key: z-map key — five `@ux` limbs (Tip5 / ``based``).
::
+$  nns-name-key  [@ux @ux @ux @ux @ux]
::
+$  nns-accumulator-entry
  $:  name=@t
      owner=@t
      tx-hash=@ux
      claim-height=@ud
      block-digest=@ux
  ==
::
+$  nns-accumulator  (z-map nns-name-key nns-accumulator-entry)
::
++  new  ^-  nns-accumulator  ~
::
::  +name-key: Tip5 digest of the UTF-8 cord, as z-map key limbs.
::
++  name-key
  |=  name=@t
  ^-  nns-name-key
  =/  d=noun-digest:tip5:z
    (hash-hashable:tip5:z (hashable-atom-chunks name))
  ;;(nns-name-key d)
::
::  +apt: structural sanity check. O(n). Verifies every entry has a
::  non-empty owner and a non-zero tx-hash. The underlying z-map is
::  assumed well-formed because callers construct it exclusively
::  through +insert / +from-list below; we don't re-run `apt:z-by`.
::
++  apt
  |=  acc=nns-accumulator
  ^-  ?
  =/  rows=(list [nns-name-key nns-accumulator-entry])  ~(tap z-by acc)
  |-  ^-  ?
  ?~  rows  %.y
  =*  e  +.i.rows
  ?.  ?&  !=(0 (met 3 name.e))
          !=(0 (met 3 owner.e))
          !=(0 tx-hash.e)
      ==
    %.n
  $(rows t.rows)
::
++  has
  |=  [acc=nns-accumulator name=@t]
  ^-  ?
  (~(has z-by acc) (name-key name))
::
++  get
  |=  [acc=nns-accumulator name=@t]
  ^-  (unit nns-accumulator-entry)
  (~(get z-by acc) (name-key name))
::
++  proof-axis
  |=  [acc=nns-accumulator name=@t]
  ^-  (unit @)
  (~(dig z-by acc) (name-key name))
::
++  got
  |=  [acc=nns-accumulator name=@t]
  ^-  nns-accumulator-entry
  (~(got z-by acc) (name-key name))
::
::  +insert: first-writer-wins put. If `name` is absent, add the
::  entry and return the new accumulator. If it is already present,
::  return `acc` unchanged — the later claim silently loses.
::
++  insert
  |=  [acc=nns-accumulator name=@t entry=nns-accumulator-entry]
  ^-  nns-accumulator
  =/  k  (name-key name)
  ?:  (~(has z-by acc) k)  acc
  =/  ent=nns-accumulator-entry  entry(name name)
  (~(put z-by acc) k ent)
::
::  +atom-u32-le: split an atom into little-endian base-2^32 limbs (each
::  limb fits in a Goldilocks belt). Avoids feeding one enormous `%leaf`
::  through `hash-noun-varlen`, which assumes belt-shaped atoms along
::  structural walks.
::
++  atom-u32-le
  |=  a=@
  ^-  (list @)
  |-  ^-  (list @)
  ?:  =(0 a)  ~
  =/  qr  (dvr a (bex 32))
  [+.qr $(a -.qr)]
::
::  +hashable-atom-chunks: encode arbitrary `@` as `%list` of small
::  `%leaf` limbs (Tip5 `hash-hashable` `%list` branch).
::
++  hashable-atom-chunks
  |=  a=@
  ^-  hashable:tip5:z
  [%list (turn (atom-u32-le a) |=(w=@ leaf+w))]
::
::  +hashable-entry: canonical hashable view of one accumulator row's
::  value (owner cord, hashes, height). `name` is hashed as the row key
::  limb in `++hashable-accumulator`, not duplicated here.
::
++  hashable-entry
  |=  ent=nns-accumulator-entry
  ^-  hashable:tip5:z
  =/  lis=(list hashable:tip5:z)
    :~  (hashable-atom-chunks owner.ent)
        (hashable-atom-chunks tx-hash.ent)
        leaf+claim-height.ent
        (hashable-atom-chunks block-digest.ent)
    ==
  [%list lis]
::
::  +hashable-accumulator: sorted rows as nested hashables — order by
::  lexicographic `name` on each entry (same as former `@t` key order).
::
++  hashable-accumulator
  |=  acc=nns-accumulator
  ^-  hashable:tip5:z
  =/  rows=(list [nns-name-key nns-accumulator-entry])
    %+  sort  ~(tap z-by acc)
    |=  [[* a=nns-accumulator-entry] [* b=nns-accumulator-entry]]
    (lth name.a name.b)
  =/  lis=(list hashable:tip5:z)
    %+  turn  rows
    |=  [* e=nns-accumulator-entry]
    :-  (hashable-atom-chunks name.e)
    (hashable-entry e)
  [%list lis]
::
::  +root-from-hashable: Tip5 digest of `++hashable-accumulator`.
::
++  root-from-hashable
  |=  acc=nns-accumulator
  ^-  noun-digest:tip5:z
  (hash-hashable:tip5:z (hashable-accumulator acc))
::
::  +root: Tip5 noun-digest of the accumulator (hashable encoding).
::
++  root
  |=  acc=nns-accumulator
  ^-  noun-digest:tip5:z
  (root-from-hashable acc)
::
::  +root-atom: `root` flattened to a single `@`.
::
++  root-atom
  |=  acc=nns-accumulator
  ^-  @
  (digest-to-atom:tip5:z (root-from-hashable acc))
::
++  size
  |=  acc=nns-accumulator
  ^-  @ud
  ~(wyt z-by acc)
::
++  from-list
  |=  items=(list [@t nns-accumulator-entry])
  ^-  nns-accumulator
  =|  acc=nns-accumulator
  |-  ^-  nns-accumulator
  ?~  items  acc
  %=  $
    items  t.items
    acc    (insert acc -.i.items +.i.items(name -.i.items))
  ==
::
++  to-list
  |=  acc=nns-accumulator
  ^-  (list [@t nns-accumulator-entry])
  %+  turn  ~(tap z-by acc)
  |=  [* e=nns-accumulator-entry]
  [name.e e]
--