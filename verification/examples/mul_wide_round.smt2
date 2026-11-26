; 連結した128ビット値に、必要なら被乗数を2^64倍して加える。
; 加算を129ビットで行い、論理右シフト後に上位語と下位語を取り出す。
(define-fun zero () (_ BitVec 64) (_ bv0 64))
(define-fun one () (_ BitVec 64) (_ bv1 64))
(define-fun packed () (_ BitVec 129) ((_ zero_extend 1) (concat upper lower)))
(define-fun wide-addend () (_ BitVec 129)
  (ite (= ((_ extract 0 0) lower) #b1)
       ((_ zero_extend 1) (concat multiplicand zero))
       (_ bv0 129)))
(define-fun updated () (_ BitVec 129)
  (bvlshr (bvadd packed wide-addend) (_ bv1 129)))
(assert
  (or (distinct next-multiplicand (ite initialize left multiplicand))
      (distinct next-lower (ite initialize right ((_ extract 63 0) updated)))
      (distinct next-upper (ite initialize zero ((_ extract 127 64) updated)))
      (distinct next-mark (ite initialize one (bvshl mark one)))
      (distinct next-done
                (and (not initialize) (or done (= ((_ extract 63 63) mark) #b1))))))
(check-sat)
