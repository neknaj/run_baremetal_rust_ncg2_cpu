; 自然整列と幅を判定し、バイトマスクで書き換える部分と残す部分を決める。
(define-fun sb () Bool (= funct3 #b000))
(define-fun sh () Bool (= funct3 #b001))
(define-fun sw () Bool (= funct3 #b010))
(define-fun expected-valid () Bool
  (or sb
      (and sh (= (bvurem offset #b010) #b000))
      (and sw (= (bvurem offset #b100) #b000))))
(define-fun shift () (_ BitVec 64)
  (bvmul ((_ zero_extend 61) offset) (_ bv8 64)))
(define-fun mask () (_ BitVec 64)
  (bvshl (ite sb #x00000000000000ff
           (ite sh #x000000000000ffff #x00000000ffffffff))
         shift))
(define-fun expected () (_ BitVec 64)
  (ite expected-valid
       (bvor (bvand word (bvnot mask)) (bvand (bvshl value shift) mask))
       word))
(assert (or (distinct valid expected-valid)
            (distinct actual expected)))
