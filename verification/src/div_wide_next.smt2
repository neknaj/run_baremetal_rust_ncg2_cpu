; 1回の捕捉後に各レジスタが保持する値を定める。
(define-fun trial () (_ BitVec 65) (concat remainder ((_ extract 63 63) quotient)))
(define-fun wide-divisor () (_ BitVec 65) ((_ zero_extend 1) divisor))
(define-fun take () Bool (bvuge trial wide-divisor))
(define-fun step-remainder () (_ BitVec 64)
  ((_ extract 63 0) (ite take (bvsub trial wide-divisor) trial)))
(define-fun step-quotient () (_ BitVec 64)
  (concat ((_ extract 62 0) quotient) (ite take #b1 #b0)))
(define-fun last-round () Bool (= ((_ extract 63 63) mark) #b1))

(assert (or
  (distinct next-quotient (ite initialize dividend (ite running step-quotient quotient)))
  (distinct next-remainder (ite initialize (_ bv0 64) (ite running step-remainder remainder)))
  (distinct next-divisor (ite initialize input-divisor divisor))
  (distinct next-mark (ite initialize (_ bv1 64) (ite running (bvshl mark (_ bv1 64)) mark)))
  (distinct next-running (or initialize (and running (not last-round))))
  (distinct next-done (and (not initialize) (or done (and running last-round))))))
