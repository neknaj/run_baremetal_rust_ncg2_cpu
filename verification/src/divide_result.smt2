; 演算幅は32または64ビット。下位32ビットの符号付き値と符号なし値を区別する。
(define-fun normalized ((value (_ BitVec 64))) (_ BitVec 64)
  (ite word-mode
    (ite signed-mode
      ((_ sign_extend 32) ((_ extract 31 0) value))
      ((_ zero_extend 32) ((_ extract 31 0) value)))
    value))
(define-fun left () (_ BitVec 64) (normalized dividend))
(define-fun right () (_ BitVec 64) (normalized divisor))
(define-fun left-negative () Bool (bvslt left (_ bv0 64)))
(define-fun right-negative () Bool (bvslt right (_ bv0 64)))
(define-fun divide-by-zero () Bool (= right (_ bv0 64)))
(define-fun signed-overflow () Bool
  (and signed-mode (= right #xffffffffffffffff)
    (= left (ite word-mode #xffffffff80000000 #x8000000000000000))))

; 与えられた商と余りの大きさを、RISC-V の演算結果に変換する。
(define-fun corrected-quotient () (_ BitVec 64)
  (ite divide-by-zero #xffffffffffffffff
    (ite signed-overflow left
      (ite (and signed-mode (xor left-negative right-negative))
        (bvneg quotient-magnitude) quotient-magnitude))))
(define-fun corrected-remainder () (_ BitVec 64)
  (ite divide-by-zero left
    (ite signed-overflow (_ bv0 64)
      (ite (and signed-mode left-negative)
        (bvneg remainder-magnitude) remainder-magnitude))))
(define-fun writeback ((value (_ BitVec 64))) (_ BitVec 64)
  (ite word-mode ((_ sign_extend 32) ((_ extract 31 0) value)) value))
(assert
  (or (distinct quotient (writeback corrected-quotient))
      (distinct remainder (writeback corrected-remainder))))
