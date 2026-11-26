; instruction、rs1、is-ld、valid、address、register-index は展開結果から宣言する。
; 命令語とアドレス範囲から、独立にフェッチ済み LD の読み出し要求を定義する。
(define-fun expected-is-ld () Bool
  (= (bvand instruction #x0000707f) #x00003003))
(define-fun expected-address () (_ BitVec 64)
  (bvadd rs1 ((_ sign_extend 52) ((_ extract 31 20) instruction))))
(define-fun in-range ((at (_ BitVec 64)) (first (_ BitVec 64)) (end (_ BitVec 64))) Bool
  (and (bvuge at first) (bvult at end)))
(define-fun expected-valid () Bool
  (and expected-is-ld
       (= ((_ extract 2 0) expected-address) #b000)
       (or (in-range expected-address #x0000000000010000 #x0000000000010100)
           (in-range expected-address #x0000000000011000 #x0000000000012000)
           (in-range expected-address #x000000000002c000 #x0000000000030000))))
(assert
  (or (distinct is-ld expected-is-ld)
      (distinct valid expected-valid)
      (distinct address expected-address)
      (distinct register-index ((_ extract 11 7) instruction))))
