::  lib/nns-accumulator.hoon — NNS z-map accumulator for Path Y.
::
::  The Path Y ("recursive rollup") plan replaces `names=(map @t
::  name-entry)` plus `tx-hashes=(set @t)` with a single authenticated
::  z-map keyed by `.nock` name (`@t`) and valued by
::  `(owner, tx-hash, claim-height, block-digest)`.
::
/=  *  /common/zoon
|%
::
+$  nns-accumulator-entry
  $:  owner=@t
      tx-hash=@ux
      claim-height=@ud
      block-digest=@ux
  ==
::
+$  nns-accumulator  (z-map @t nns-accumulator-entry)
::
++  new  ^-  nns-accumulator  ~
::
::  +apt: structural sanity check. O(n). Verifies every entry has a
::  non-empty owner and a non-zero tx-hash. The underlying z-map is
::  assumed well-formed because callers construct it exclusively
::  through +insert / +from-list below; we don't re-run `apt:z-by`.
::
++  apt
  |=  acc=nns-accumulator
  ^-  ?
  =/  rows=(list [@t nns-accumulator-entry])  ~(tap z-by acc)
  |-  ^-  ?
  ?~  rows  %.y
  =*  e  +.i.rows
  ?.  ?&  !=(0 (met 3 owner.e))
          !=(0 tx-hash.e)
      ==
    %.n
  $(rows t.rows)
::
++  has
  |=  [acc=nns-accumulator name=@t]
  ^-  ?
  (~(has z-by acc) name)
::
++  get
  |=  [acc=nns-accumulator name=@t]
  ^-  (unit nns-accumulator-entry)
  (~(get z-by acc) name)
::
++  proof-axis
  |=  [acc=nns-accumulator name=@t]
  ^-  (unit @)
  (~(dig z-by acc) name)
::
++  got
  |=  [acc=nns-accumulator name=@t]
  ^-  nns-accumulator-entry
  (~(got z-by acc) name)
::
::  +insert: first-writer-wins put. If `name` is absent, add the
::  entry and return the new accumulator. If it is already present,
::  return `acc` unchanged — the later claim silently loses.
::
++  insert
  |=  [acc=nns-accumulator name=@t entry=nns-accumulator-entry]
  ^-  nns-accumulator
  ?:  (~(has z-by acc) name)  acc
  (~(put z-by acc) name entry)
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
::  value (owner cord, hashes, height).
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
::  +hashable-accumulator: sorted `(list [`@t` entry])` as nested
::  hashables — keys ascending by numeric `@t` order (same as `lth` on
::  cords in `dor-tip`).
::
++  hashable-accumulator
  |=  acc=nns-accumulator
  ^-  hashable:tip5:z
  =/  rows=(list [@t nns-accumulator-entry])
    %+  sort  ~(tap z-by acc)
    |=  [[a=@t *] [b=@t *]]
    (lth a b)
  =/  lis=(list hashable:tip5:z)
    %+  turn  rows
    |=  [n=@t e=nns-accumulator-entry]
    :-  (hashable-atom-chunks n)
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
    acc    (insert acc -.i.items +.i.items)
  ==
::
++  to-list
  |=  acc=nns-accumulator
  ^-  (list [@t nns-accumulator-entry])
  ~(tap z-by acc)
--
