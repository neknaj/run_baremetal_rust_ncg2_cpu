; 1回の更新で、積に対する剰余の不変条件が保存されることを調べる。
(set-logic QF_NIA)
(set-option :timeout 30000)
(declare-const multiplicand Int)
(declare-const multiplier Int)
(declare-const accumulated Int)
(declare-const next-multiplicand Int)
(declare-const next-multiplier Int)
(declare-const next-accumulated Int)
(declare-const low-bit Int)
(declare-const shift-overflow Int)
(declare-const add-overflow Int)
(define-fun modulus () Int 18446744073709551616)
(assert (or (= low-bit 0) (= low-bit 1)))
(assert (= multiplier (+ (* 2 next-multiplier) low-bit)))
(assert (= next-multiplicand (- (* 2 multiplicand) (* modulus shift-overflow))))
(assert (= next-accumulated
  (- (+ accumulated (* low-bit multiplicand)) (* modulus add-overflow))))
(assert
  (distinct
    (+ next-accumulated (* next-multiplicand next-multiplier))
    (-
      (+ accumulated (* multiplicand multiplier))
      (* modulus (+ add-overflow (* shift-overflow next-multiplier))))))
(check-sat)
