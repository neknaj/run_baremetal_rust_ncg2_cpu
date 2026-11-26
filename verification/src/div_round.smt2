; 余りに被除数の次のビットを連結し、65ビットの整数として比較・減算する。
(define-fun trial () (_ BitVec 65) (concat remainder next-bit))
(define-fun wide-divisor () (_ BitVec 65) ((_ zero_extend 1) divisor))
(define-fun expected-quotient-bit () Bool (bvuge trial wide-divisor))
(define-fun selected () (_ BitVec 65)
  (ite expected-quotient-bit (bvsub trial wide-divisor) trial))

; 除数0や余りが除数以上の場合も含め、ゲートの出力と参照式を全入力で照合する。
(push)
(assert (or (distinct quotient-bit expected-quotient-bit)
            (distinct next-remainder ((_ extract 63 0) selected))))
(check-sat)
(pop)

; 反復中の余りが除数未満なら、次の余りも除数未満で上位桁は0になる。
(assert (bvult remainder divisor))
(assert (not (bvult selected wide-divisor)))
