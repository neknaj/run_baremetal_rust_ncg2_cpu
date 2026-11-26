; 上限と自然整列からフェッチの可否を求める。
(define-fun expected-valid () Bool
  (and (bvult address #x0000000000006000)
       (= (bvurem address (_ bv4 64)) (_ bv0 64))))
(define-fun word-index () (_ BitVec 32)
  ((_ extract 31 0) (bvudiv address (_ bv8 64))))
(define-fun high-half () Bool (= (bvurem address (_ bv8 64)) (_ bv4 64)))
(define-fun expected () (_ BitVec 32)
  (bvor word-index (ite high-half #x5678a000 #x12345000)))
(assert (or (distinct valid expected-valid)
            (and expected-valid (distinct actual expected))))
