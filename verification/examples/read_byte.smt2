; オフセットをビット位置に直し、対象の8ビットを切り出す。
(define-fun shift () (_ BitVec 64)
  (bvmul ((_ zero_extend 61) offset) (_ bv8 64)))
(define-fun byte () (_ BitVec 8)
  ((_ extract 7 0) (bvlshr word shift)))
(define-fun expected () (_ BitVec 64)
  (ite i67 ((_ zero_extend 56) byte) ((_ sign_extend 56) byte)))
(assert (distinct actual expected))
