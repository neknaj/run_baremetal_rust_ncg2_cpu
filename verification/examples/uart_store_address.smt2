; RV64 のストアが指定する実効アドレスと、64ビット RAM 語のアドレス。
(define-fun store-op () Bool (= ((_ extract 6 0) instruction) #b0100011))
(define-fun kind () (_ BitVec 3) ((_ extract 14 12) instruction))
(define-fun effective () (_ BitVec 64)
  (bvadd rs1 ((_ sign_extend 52) (concat ((_ extract 31 25) instruction)
                                        ((_ extract 11 7) instruction)))))
(define-fun aligned-store () Bool
  (or (= kind #b000)
      (and (= kind #b001) (= ((_ extract 0 0) effective) #b0))
      (and (= kind #b010) (= ((_ extract 1 0) effective) #b00))
      (and (= kind #b011) (= ((_ extract 2 0) effective) #b000))))
; 実際に許可される SD の入力が存在することを確かめる。
(push)
(assert (and (= instruction #x0012b023) (= rs1 #x0000000000010000) allowed))
(check-sat)
(pop)

; 許可の出る命令は自然整列したストア。ストアのアドレスは probe の時系列に依存しない。
(assert
  (or (and allowed (not (and store-op aligned-store)))
      (and store-op aligned-store (distinct bus (bvand effective #xfffffffffffffff8)))))
(check-sat)
