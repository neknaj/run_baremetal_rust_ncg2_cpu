; バンクの物理アドレスは半開区間で定める。
(define-fun within ((first (_ BitVec 64)) (last (_ BitVec 64))) Bool
  (and (= (bvurem address (_ bv8 64)) (_ bv0 64))
       (bvuge address first) (bvult address last)))
(define-fun rom () Bool (within #x0000000000010000 #x0000000000016000))
(define-fun bss () Bool (within #x0000000000016000 #x0000000000019000))
(define-fun mailbox () Bool (within #x000000000002bf00 #x000000000002c000))
(define-fun stack () Bool (within #x000000000002c000 #x0000000000030000))
