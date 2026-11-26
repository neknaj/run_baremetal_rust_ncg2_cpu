; 初期化と反復1回の数学的な状態遷移との食い違いを探す。
(define-fun zero () (_ BitVec 64) (_ bv0 64))
(define-fun one () (_ BitVec 64) (_ bv1 64))
(define-fun addend () (_ BitVec 64)
  (ite (= ((_ extract 0 0) multiplier) #b1) multiplicand zero))
(assert
  (or
    (distinct next-multiplicand (ite initialize left (bvshl multiplicand one)))
    (distinct next-multiplier (ite initialize right (bvlshr multiplier one)))
    (distinct next-accumulated (ite initialize zero (bvadd accumulated addend)))
    (distinct next-mark (ite initialize one (bvshl mark one)))
    (distinct next-done
      (and (not initialize) (or done (= ((_ extract 63 63) mark) #b1))))))
