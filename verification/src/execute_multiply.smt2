; rs1、rs2 と rd を除いた符号化を公式の命令表と比較する。
(define-fun encoding () (_ BitVec 32) (bvand instruction #xfe00707f))
(define-fun mul () Bool (= encoding #x02000033))
(define-fun mulh () Bool (= encoding #x02001033))
(define-fun mulhsu () Bool (= encoding #x02002033))
(define-fun mulhu () Bool (= encoding #x02003033))
(define-fun mulw () Bool (= encoding #x0200003b))

(assert
  (or (distinct recognized (or mul mulh mulhsu mulhu mulw))
      (distinct high-half (or mulh mulhsu mulhu))
      (distinct signed-left (or mulh mulhsu))
      (distinct signed-right mulh)
      (distinct word-mode mulw)))
