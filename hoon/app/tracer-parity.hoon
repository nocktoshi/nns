::  app/tracer-parity.hoon — trace formula vs host-spec parity (oracle tests).
::
::  Peek paths: /parity-trace-genesis, /parity-trace-transition-empty,
::  /parity-trace-transition-full. Imported by app.hoon: `/= trcp /app/tracer-parity`
::  (`par` shadows `++par` from `/= * /common/zoon`).
::
/=  tracer  /app/tracer
/=  rb  /app/recursive-build
/=  na  /app/nns-accumulator
/=  np  /app/nns-predicates
/=  tw  /app/tx-witness
|%
::
::  Spec ↔ trace correspondence (partial until Nock 9+):
::    prev-proof-ok     → trace-nonzero proof-len (full)
::    want-h / digest   → trace-eq + trace-eq-based-digests-*
::    FWW               → acc-tag / cand-tag branch (full)
::    genesis TLD has:na → not in trace (genesis: nonzero acc only)
::
++  parity-genesis-trace
  |=  [acc=nns-accumulator:na height=@ud digest=@ux]
  ^-  ?
  =/  samp  (build-genesis-recursive-inputs:rb acc height digest)
  =/  dry-run
    %-  mule  |.  .*(-.samp +.samp)
  ?.  ?=(%& -.dry-run)  %.n
  ?.  ?=(%.y p.dry-run)  %.n
  (genesis-recursive-formula:rb acc height digest)
::
++  parity-transition-empty
  |=  [prev-height=@ud page-d=@ux old-acc=nns-accumulator:na]
  ^-  ?
  =/  want-h=@ud  +(prev-height)
  =/  page=block-id:tw  (from-hull-atom:block-id:tw page-d)
  =/  subj=*
    (empty-transition-subject:tracer prev-height page want-h page)
  =/  form=*
    (build-transition-trace-formula-empty:tracer want-h)
  =/  dry-run
    %-  mule  |.  .*(subj form)
  ?.  ?=(%& -.dry-run)  %.n
  ?.  ?=(%.y p.dry-run)  %.n
  =/  pag=nns-page-summary:np  [page-d ~]
  =/  claims=(list nns-claim:np)  ~
  (transition-spec:rb 0x1 0 0 prev-height old-acc pag claims 0 want-h page-d)
::
++  parity-transition-full
  |=  [prev-proof=* prev-height=@ud old-acc=nns-accumulator:na pag=nns-page-summary:np claims=(list nns-claim:np)]
  ^-  ?
  =/  samp
    %-  build-recursive-transition-inputs:rb
    :*  prev-proof
        0
        0
        prev-height
        old-acc
        pag
        claims
        0
        digest.pag
    ==
  =/  dry-run
    %-  mule  |.  .*(subject.samp formula.samp)
  ?.  ?=(%& -.dry-run)  %.n
  ?.  ?=(%.y p.dry-run)  %.n
  (transition-spec:rb prev-proof 0 0 prev-height old-acc pag claims 0 +(prev-height) digest.pag)
::
++  peek-tracer-parity
  |=  $:  =path
          acc=nns-accumulator:na
          last-proved-height=@ud
          last-proved-digest=@ux
          genesis-height=@ud
      ==
  ^-  (unit (unit *))
  ?+  path  ~
      [%parity-trace-genesis ~]
    ``(parity-genesis-trace acc genesis-height 0x0)
  ::
      [%parity-trace-transition-empty ~]
    =/  prev-h=@ud
      ?:  (gth last-proved-height 0)
        last-proved-height
      genesis-height
    =/  page-d=@ux
      ?:  (gth last-proved-height 0)
        last-proved-digest
      0x1
    ``(parity-transition-empty prev-h page-d acc)
  ::
      [%parity-trace-transition-full ~]
    =/  prev-h=@ud  genesis-height
    =/  page-d=@ux  0x1
    =/  pag=nns-page-summary:np  [page-d ~]
    =/  wit=nns-raw-tx-witness:np  [0x1 0x1 0 '']
    =/  claims=(list nns-claim:np)
      :~  ['nockchain.nock' 'o' 0 0x1 wit]
      ==
    ``(parity-transition-full 0x1 prev-h acc pag claims)
  ==
--
