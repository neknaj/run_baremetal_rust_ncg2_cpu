; R 形式の XOR と SLTU の値を、命令の固定ビットと64ビット演算で定める。
(define-fun selected () (_ BitVec 32) (bvand instruction #xfe00707f))
(define-fun is-xor () Bool (= selected #x00004033))
(define-fun is-sltu () Bool (= selected #x00003033))
(assert (or (distinct recognized (or is-xor is-sltu))
            (distinct multiply (or (= selected #x02000033) (= selected #x02003033)))
            (distinct high-product (= selected #x02003033))
            (and is-xor (distinct result (bvxor rs1 rs2)))
            (and is-sltu (distinct result (ite (bvult rs1 rs2) (_ bv1 64) (_ bv0 64))))))
