::  hoon/app/names-test.hoon — compile-time tests for nns's
::  verification gate.
::
::  G1 name format and fee schedule come from `nns-predicates`; Merkle
::  fixture helpers stay local for building test proofs. The kernel gate
::  lives in `hoon/app/app.hoon`.
::
::  Compile: hoonc --new --arbitrary hoon/app/names-test.hoon hoon/
::  Success (build succeeded) = all assertions passed.
::
/+  *vesl-merkle
/=  np  /app/nns-predicates
::
=>
|%
++  assert-eq
  |*  [a=* b=*]
  ~|  'assert-eq: values not equal'
  ?>  =(a b)
  %.y
::
::  Merkle fixture helpers (test-local scaffolding).
::
++  nth
  |=  [lst=(list @) i=@ud]
  ^-  @
  ?~  lst  ~|('nth: out of bounds' !!)
  ?:  =(i 0)  i.lst
  $(lst t.lst, i (dec i))
::
++  next-level
  |=  level=(list @)
  ^-  (list @)
  ?~  level  ~
  ?~  t.level
    ~[(hash-pair i.level i.level)]
  [(hash-pair i.level i.t.level) $(level t.t.level)]
::
++  compute-root
  |=  leaves=(list @)
  ^-  @
  ?~  leaves  0
  =/  level  (turn leaves hash-leaf)
  |-  ^-  @
  ?:  ?=([@ ~] level)  i.level
  $(level (next-level level))
::
++  proof-for
  |=  [leaves=(list @) idx=@ud]
  ^-  (list [hash=@ side=?])
  =/  level=(list @)  (turn leaves hash-leaf)
  =|  acc=(list [hash=@ side=?])
  =/  i=@ud  idx
  |-  ^-  (list [hash=@ side=?])
  ?:  ?=([@ ~] level)  (flop acc)
  =/  n=@ud  (lent level)
  =/  sibling-idx=@ud
    ?:  =(0 (mod i 2))  +(i)
    (sub i 1)
  =/  sib=@
    ?:  (lth sibling-idx n)  (nth level sibling-idx)
    (nth level i)
  =/  side=?  =(1 (mod i 2))
  %=  $
    level  (next-level level)
    i      (div i 2)
    acc    [[sib side] acc]
  ==
::
::  nns-gate under test — batch shape.
::  data = (list [name owner tx-hash proof]); every leaf must clear
::  G1 (name format) and G2 (Merkle inclusion under expected-root).
::
++  nns-gate
  |=  [data=* expected-root=@]
  ^-  ?
  =/  leaves
    ;;((list [name=@t owner=@t tx-hash=@t proof=(list [hash=@ side=?])]) data)
  |-  ^-  ?
  ?~  leaves  %.y
  =/  chunk=@  (jam [name.i.leaves owner.i.leaves tx-hash.i.leaves])
  ?&  (is-valid-name:np name.i.leaves)
      (verify-chunk chunk proof.i.leaves expected-root)
      $(leaves t.leaves)
  ==
--
::
::  ============================================
::  FIXTURES
::  ============================================
::
=/  alice=@t  'alice-address'
=/  bob=@t    'bob-address'
=/  tx1=@t    'tx-hash-1'
=/  tx2=@t    'tx-hash-2'
=/  tx3=@t    'tx-hash-3'
::
::  ============================================
::  G1: name format
::  ============================================
::
?>  (assert-eq (is-valid-name:np 'a.nock') %.y)
?>  (assert-eq (is-valid-name:np 'abc123.nock') %.y)
?>  (assert-eq (is-valid-name:np 'deadbeef01.nock') %.y)
?>  (assert-eq (is-valid-name:np '.nock') %.n)
?>  (assert-eq (is-valid-name:np 'foo') %.n)
?>  (assert-eq (is-valid-name:np 'foo.bar') %.n)
?>  (assert-eq (is-valid-name:np 'Foo.nock') %.n)
?>  (assert-eq (is-valid-name:np 'foo-bar.nock') %.n)
?>  (assert-eq (is-valid-name:np 'foo.nock.nock') %.y)
?>  (assert-eq (is-valid-name:np 'foo_bar.nock') %.n)
::
::  ============================================
::  Fee tiers match legacy worker (nicks)
::  ============================================
::
?>  (assert-eq (fee-for-name:np 'a.nock') 327.680.000)
?>  (assert-eq (fee-for-name:np 'abcd.nock') 327.680.000)
?>  (assert-eq (fee-for-name:np 'abcde.nock') 32.768.000)
?>  (assert-eq (fee-for-name:np 'abcdefghi.nock') 32.768.000)
?>  (assert-eq (fee-for-name:np 'abcdefghij.nock') 6.553.600)
::
::  ============================================
::  Batch G2: 1-leaf batch (smallest real case)
::  ============================================
::
=/  leaf-a=@             (jam ['alpha.nock' alice tx1])
=/  leaves-1=(list @)    ~[leaf-a]
=/  root-1=@             (compute-root leaves-1)
=/  proof-1=(list [hash=@ side=?])  (proof-for leaves-1 0)
::
?>  %-  assert-eq
    :-  %.y
    %-  nns-gate
    :_  root-1
    ~[[name='alpha.nock' owner=alice tx-hash=tx1 proof=proof-1]]
::
::  ============================================
::  Batch G2: 3-leaf batch (every leaf at every position).
::  Also exercises the duplicate-last padding at odd levels.
::  ============================================
::
=/  leaf-al=@  (jam ['alpha.nock' alice tx1])
=/  leaf-br=@  (jam ['bravo.nock' bob tx2])
=/  leaf-ch=@  (jam ['charlie.nock' alice tx3])
=/  leaves-3=(list @)  ~[leaf-al leaf-br leaf-ch]
=/  root-3=@           (compute-root leaves-3)
=/  proof-al=(list [hash=@ side=?])  (proof-for leaves-3 0)
=/  proof-br=(list [hash=@ side=?])  (proof-for leaves-3 1)
=/  proof-ch=(list [hash=@ side=?])  (proof-for leaves-3 2)
::
?>  %-  assert-eq
    :-  %.y
    %-  nns-gate
    :_  root-3
    ^-  (list [name=@t owner=@t tx-hash=@t proof=(list [hash=@ side=?])])
    :~  [name='alpha.nock' owner=alice tx-hash=tx1 proof=proof-al]
        [name='bravo.nock' owner=bob tx-hash=tx2 proof=proof-br]
        [name='charlie.nock' owner=alice tx-hash=tx3 proof=proof-ch]
    ==
::
::  ============================================
::  Batch G2: subset batches verify independently
::  ============================================
::
?>  %-  assert-eq
    :-  %.y
    %-  nns-gate
    :_  root-3
    ^-  (list [name=@t owner=@t tx-hash=@t proof=(list [hash=@ side=?])])
    :~  [name='alpha.nock' owner=alice tx-hash=tx1 proof=proof-al]
        [name='charlie.nock' owner=alice tx-hash=tx3 proof=proof-ch]
    ==
::
?>  %-  assert-eq
    :-  %.y
    %-  nns-gate
    :_  root-3
    ~[[name='bravo.nock' owner=bob tx-hash=tx2 proof=proof-br]]
::
::  ============================================
::  Batch rejects: one tampered leaf poisons the whole batch
::  ============================================
::
?>  %-  assert-eq
    :-  %.n
    %-  nns-gate
    :_  root-3
    ^-  (list [name=@t owner=@t tx-hash=@t proof=(list [hash=@ side=?])])
    :~  [name='alpha.nock' owner=alice tx-hash=tx1 proof=proof-al]
        [name='bravo.nock' owner=alice tx-hash=tx2 proof=proof-br]
        [name='charlie.nock' owner=alice tx-hash=tx3 proof=proof-ch]
    ==
::
::  ============================================
::  Batch rejects: proof/leaf mismatch (proof for index 0 on leaf
::  that actually lives at index 1)
::  ============================================
::
?>  %-  assert-eq
    :-  %.n
    %-  nns-gate
    :_  root-3
    ~[[name='bravo.nock' owner=bob tx-hash=tx2 proof=proof-al]]
::
::  ============================================
::  Batch rejects: format failure (invalid name in the batch)
::  ============================================
::
=/  bad-leaf=@  (jam ['Bad.nock' alice tx1])
=/  bad-root=@  (compute-root ~[bad-leaf])
=/  bad-proof=(list [hash=@ side=?])  (proof-for ~[bad-leaf] 0)
?>  %-  assert-eq
    :-  %.n
    %-  nns-gate
    :_  bad-root
    ~[[name='Bad.nock' owner=alice tx-hash=tx1 proof=bad-proof]]
::
::  Mixing a well-formed leaf with a malformed one also fails.
::
?>  %-  assert-eq
    :-  %.n
    %-  nns-gate
    :_  root-1
    ^-  (list [name=@t owner=@t tx-hash=@t proof=(list [hash=@ side=?])])
    :~  [name='alpha.nock' owner=alice tx-hash=tx1 proof=proof-1]
        [name='Bad.nock' owner=alice tx-hash=tx1 proof=~]
    ==
::
::  ============================================
::  Batch rejects: root mismatch (valid leaf against a different
::  commitment)
::  ============================================
::
?>  %-  assert-eq
    :-  %.n
    %-  nns-gate
    :_  (compute-root ~[(jam ['other.nock' alice tx1])])
    ~[[name='alpha.nock' owner=alice tx-hash=tx1 proof=proof-1]]
::
::  ============================================
::  Empty batch: vacuously accepted by the gate itself.
::  A full kernel would reject an empty settlement batch before the
::  gate, but the gate has no reason to fail on "nothing
::  to disprove" — `(list)` == `~` is a valid value for `data`.
::  ============================================
::
?>  %-  assert-eq
    :-  %.y
    %-  nns-gate
    [~ root-1]
::
%pass
