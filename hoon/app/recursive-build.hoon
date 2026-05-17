::  app/recursive-build.hoon — recursive STARK subject/formula builders + host spec.
::
::  Shared by app.hoon (prove arms) and tracer-parity.hoon (parity oracles).
::
/=  tracer  /app/tracer
/=  na  /app/nns-accumulator
/=  np  /app/nns-predicates
/=  tw  /app/tx-witness
/=  *  /common/zoon
|%
::
+$  transition-claim
  $:  key=nns-name-key:na
      cand=nns-claim:np
  ==
::
++  nns-genesis-height  63.000
::
++  nns-genesis-tld-name  'nock'
::
++  genesis-recursive-formula
  |=  [acc=nns-accumulator:na height=@ud digest=@ux]
  ^-  ?
  ?&  (has:na acc nns-genesis-tld-name)
      =(height nns-genesis-height)
      =(digest 0x0)
  ==
::
++  build-genesis-recursive-inputs
  |=  [acc=nns-accumulator:na height=@ud digest=@ux]
  ^-  [subject=* formula=*]
  =/  sub=*  [acc [height digest]]
  =/  form=*  (genesis-trace-formula:tracer nns-genesis-height)
  [sub form]
::
++  prekey-claims
  |=  claims=(list nns-claim:np)
  ^-  (list transition-claim)
  %+  turn  claims
  |=  c=nns-claim:np
  [(name-key:na name.c) c]
::
++  z-map-to-name-list
  |=  acc=nns-accumulator:na
  ^-  (list [nns-name-key:na nns-accumulator-entry:na])
  ~(tap z-by acc)
::
++  name-key-limb
  |=  k=nns-name-key:na
  ^-  @ux
  =/  [k0=@ux k1=@ux k2=@ux k3=@ux k4=@ux]  k
  k0
::
++  acc-list-head-key
  |=  rows=(list [nns-name-key:na nns-accumulator-entry:na])
  ^-  @ux
  ?~  rows  0x0
  (name-key-limb +2.i.rows)
::
++  prev-proof-ok-spec
  |=  [prev-proof=* prev-height=@ud]
  ^-  ?
  ?&  ?=(@ prev-proof)
      (gth (met 3 prev-proof) 0)
      (gth prev-height 0)
  ==
::
++  minimal-first-writer-wins
  |=  $:  old-acc=nns-accumulator:na
          claims=(list nns-claim:np)
          height=@ud
          digest=@ux
      ==
  ^-  nns-accumulator:na
  =/  acc  old-acc
  |-  ^-  nns-accumulator:na
  ?~  claims  acc
  =/  c  i.claims
  =/  k  (name-key:na name.c)
  =/  acc
    ?:  (~(has z-by acc) k)
      acc
    =/  ent=nns-accumulator-entry:na
      [name.c owner.c tx-hash.c height digest]
    (~(put z-by acc) k ent)
  $(claims t.claims)
::
++  transition-spec
  |=  $:  prev-proof=*
          prev-subj=*
          prev-form=*
          prev-height=@ud
          old-acc=nns-accumulator:na
          pag=nns-page-summary:np
          claims=(list nns-claim:np)
          block-proof=*
          want-height=@ud
          want-digest=@ux
      ==
  ^-  ?
  ?.  (prev-proof-ok-spec prev-proof prev-height)  %.n
  =/  new-acc  (minimal-first-writer-wins old-acc claims want-height digest.pag)
  ?.  =(want-height +(prev-height))  %.n
  ?.  =(want-digest digest.pag)  %.n
  %.y
::
++  build-recursive-transition-inputs
  |=  $:  prev-proof=*
          prev-subj=*
          prev-form=*
          prev-height=@ud
          old-acc=nns-accumulator:na
          pag=nns-page-summary:np
          claims=(list nns-claim:np)
          block-proof=*
          want-digest=@ux
      ==
  ^-  [subject=* formula=*]
  =/  want-height=@ud  +(prev-height)
  =/  page=block-id:tw  (from-hull-atom:block-id:tw digest.pag)
  =/  want=block-id:tw  (from-hull-atom:block-id:tw want-digest)
  ?:  ?=(@ claims)
    =/  sub=*
      (empty-transition-subject:tracer prev-height page want-height want)
    =/  form=*
      (build-transition-trace-formula-empty:tracer want-height)
    [sub form]
  =/  keyed=(list transition-claim)  (prekey-claims claims)
  ?.  (lte (lent keyed) 1)
    ~|(%recursive-transition-too-many-claims !!)
  =/  acc-rows=(list [nns-name-key:na nns-accumulator-entry:na])
    (z-map-to-name-list old-acc)
  =/  acc-tag=@ux  (acc-list-head-key acc-rows)
  =/  cand-tag=@ux
    ?~  keyed  0x0
    (name-key-limb key.i.keyed)
  =/  proof-len=@ud
    ?:  ?=(@ prev-proof)
      (met 3 prev-proof)
    0
  =/  sub=*
    %-  full-transition-subject:tracer
    :*  proof-len
        prev-height
        page
        acc-tag
        cand-tag
        block-proof
        want-height
        want
    ==
  =/  form=*
    (transition-trace-formula-full:tracer prev-height want-height)
  [sub form]
--
