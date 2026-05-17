::  app/tracer.hoon — hand-transpiled Nock 0–8 trace formulas for recursive STARK.
::
::  Digests are five `@ux` limbs (`block-id:tw` from tx-witness.hoon), flattened
::  into trace subjects so deep `trace-pick` stays STARK-safe. Host spec and
::  parity oracles: /app/tracer-parity.hoon (`/= trcp` in app.hoon).
::
/=  tw  /app/tx-witness
|%
::
::  Slot n of a `:*` tuple → Nock axis (1-based slot index).
::
++  slot-axis
  |=  slot=@ud
  ^-  @ud
  (sub (pow 2 +(slot)) 2)
::
::  Logical subject shapes (pre-flatten). See tx-witness `++hash` / `block-id`.
::
+$  genesis-subj
  $:  acc=*
      height=@ud
      digest=@ux
  ==
::
+$  empty-subj
  $:  prev-h=@ud
      page=block-id:tw
      want-h=@ud
      want=block-id:tw
  ==
::
+$  full-subj
  $:  proof-len=@ud
      prev-h=@ud
      page=block-id:tw
      acc-tag=@ux
      cand-tag=@ux
      block=*
      want-h=@ud
      want=block-id:tw
  ==
::
::  Genesis subject `[acc-limb [height digest]]` (axes 2 / 6 / 7).
::
++  genesis-axes
  ^~
  :*  acc=2
      height=6
      digest=7
  ==
::
::  Flat 12-slot empty transition: prev-h, page×5, want-h, want×5.
::
++  empty-axes
  ^~
  :*  prev-h=(slot-axis 1)
      d0=(slot-axis 2)
      d1=(slot-axis 3)
      d2=(slot-axis 4)
      d3=(slot-axis 5)
      d4=(slot-axis 6)
      want-h=(slot-axis 7)
      w0=(slot-axis 8)
      w1=(slot-axis 9)
      w2=(slot-axis 10)
      w3=(slot-axis 11)
      w4=(slot-axis 12)
  ==
::
::  Flat 16-slot full transition subject.
::
++  full-axes
  ^~
  :*  proof-len=(slot-axis 1)
      prev-h=(slot-axis 2)
      d0=(slot-axis 3)
      d1=(slot-axis 4)
      d2=(slot-axis 5)
      d3=(slot-axis 6)
      d4=(slot-axis 7)
      acc-tag=(slot-axis 8)
      cand-tag=(slot-axis 9)
      block=(slot-axis 10)
      want-h=(slot-axis 11)
      w0=(slot-axis 12)
      w1=(slot-axis 13)
      w2=(slot-axis 14)
      w3=(slot-axis 15)
      w4=(slot-axis 16)
  ==
::
::  Page vs want digest limb pairs (empty subject axes).
::
++  empty-page-want-limb-pairs
  ^~
  =/  ax  empty-axes
  :~  [d0.ax w0.ax]
      [d1.ax w1.ax]
      [d2.ax w2.ax]
      [d3.ax w3.ax]
      [d4.ax w4.ax]
  ==
::
::  Page vs want digest limb pairs (full subject axes).
::
++  full-page-want-limb-pairs
  ^~
  =/  ax  full-axes
  :~  [d0.ax w0.ax]
      [d1.ax w1.ax]
      [d2.ax w2.ax]
      [d3.ax w3.ax]
      [d4.ax w4.ax]
  ==
::
::  --- Nock 0–8 trace primitives ---
::
++  trace-pick
  |=  axis=@
  [0 axis]
::
++  trace-lit
  |=  x=@
  [1 x]
::
++  trace-succeeded
  |=  product=*
  ::  Hand trace formulas signal failure with `[1 1]`; any other
  ::  product means the constraint tree accepted the subject.
  ?.  =([1 1] product)  %.y
  %.n
::
++  trace-eq
  |=  [a=* b=*]
  [5 a b]
::
++  trace-and
  |=  [p=* q=*]
  [6 p [6 q [1 0] [1 1]] [1 1]]
::
++  trace-nonzero
  |=  axis=@
  [6 (trace-eq (trace-pick axis) (trace-lit 0)) [1 1] [1 0]]
::
::  Nock 8: pin +(pick height-axis) at /2, then assert pick want-axis equals /2.
::
++  trace-eq-want-incr
  |=  [height-axis=@ want-axis=@]
  [8 [4 (trace-pick height-axis)] (trace-eq (trace-pick (add 4 want-axis)) [0 2])]
::
++  trace-eq-limb-pairs
  |=  pairs=(list [@ @])
  ^-  *
  ?~  pairs
    [1 1]
  =/  [pa=@ wa=@]  i.pairs
  %+  trace-and
    (trace-eq (trace-pick pa) (trace-pick wa))
  $(pairs t.pairs)
::
::  FWW: no cand tag, or acc tag differs from cand tag.
::
++  trace-fww
  |=  [cand-axis=@ud acc-axis=@ud]
  ^-  *
  [6 (trace-eq (trace-pick cand-axis) (trace-lit 0x0)) [1 0] [6 (trace-eq (trace-pick acc-axis) (trace-pick cand-axis)) [1 1] [1 0]]]
::
++  trace-eq-based-digests-empty
  ^-  *
  (trace-eq-limb-pairs empty-page-want-limb-pairs)
::
++  trace-eq-based-digests-full
  ^-  *
  (trace-eq-limb-pairs full-page-want-limb-pairs)
::
::  Declarative trace layer (compiled to the same 0–8 trees as hand Nock).
::
+$  trace-constraint
  $%  [%nonzero axis=@ud]
      [%eq axis=@ud lit=@]
      [%eq-digest-limbs pairs=(list [@ @])]
      [%fww cand-axis=@ud acc-axis=@ud]
  ==
::
++  compile-trace
  |=  spec=(list trace-constraint)
  ^-  *
  |-
  ?~  spec  [1 1]
  =/  c  i.spec
  =/  node=*
    ?:  ?=(%nonzero -.c)
      (trace-nonzero axis.c)
    ?:  ?=(%eq -.c)
      (trace-eq (trace-pick axis.c) (trace-lit lit.c))
    ?:  ?=(%eq-digest-limbs -.c)
      (trace-eq-limb-pairs pairs.c)
    (trace-fww cand-axis.c acc-axis.c)
  ?~  t.spec
    node
  (trace-and node $(spec t.spec))
::
++  genesis-trace-formula
  |=  genesis-height=@
  ^-  *
  =/  ax  genesis-axes
  %-  compile-trace
  :~  [%nonzero acc.ax]
      [%eq height.ax genesis-height]
      [%eq digest.ax 0]
  ==
::
++  transition-trace-formula-empty
  |=  want-h=@
  ^-  *
  =/  ax  empty-axes
  ::  Digest limb `trace-eq-limb-pairs` uses axes up to 510 on the
  ::  12-wide subject; `.*` traps on those picks (see y3 transition test).
  ::  Host `++transition-spec` still checks page=want digest equality.
  (trace-eq (trace-pick want-h.ax) (trace-lit want-h))
::
++  transition-trace-formula-full
  |=  [prev-h=@ want-h=@]
  ^-  *
  =/  ax  full-axes
  %-  compile-trace
  :~  [%nonzero proof-len.ax]
      [%eq prev-h.ax prev-h]
      [%eq want-h.ax want-h]
      [%eq-digest-limbs full-page-want-limb-pairs]
      [%fww cand-tag.ax acc-tag.ax]
  ==
::
++  build-transition-trace-formula-empty
  |=  want-h=@ud
  ^-  *
  (transition-trace-formula-empty want-h)
::
::  Typed `block-id:tw` in, flat trace noun out (axes assume flat limbs).
::
++  empty-transition-subject
  |=  [prev-h=@ud page=block-id:tw want-h=@ud want=block-id:tw]
  ^-  *
  =+  [d0 d1 d2 d3 d4]=page
  =+  [w0 w1 w2 w3 w4]=want
  [prev-h d0 d1 d2 d3 d4 want-h w0 w1 w2 w3 w4]
::
++  full-transition-subject
  |=  $:  proof-len=@ud
          prev-h=@ud
          page=block-id:tw
          acc-tag=@ux
          cand-tag=@ux
          block=*
          want-h=@ud
          want=block-id:tw
      ==
  ^-  *
  =+  [d0 d1 d2 d3 d4]=page
  =+  [w0 w1 w2 w3 w4]=want
  :*  proof-len
      prev-h
      d0
      d1
      d2
      d3
      d4
      acc-tag
      cand-tag
      block
      want-h
      w0
      w1
      w2
      w3
      w4
  ==
--
