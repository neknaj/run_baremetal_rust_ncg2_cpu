; ISA の命令表で値が定まる funct7、funct3、opcode のみを残す。
(define-fun encoding () (_ BitVec 32) (bvand instruction #xfe00707f))
(define-fun div () Bool (= encoding #x02004033))
(define-fun divu () Bool (= encoding #x02005033))
(define-fun rem () Bool (= encoding #x02006033))
(define-fun remu () Bool (= encoding #x02007033))
(define-fun divw () Bool (= encoding #x0200403b))
(define-fun divuw () Bool (= encoding #x0200503b))
(define-fun remw () Bool (= encoding #x0200603b))
(define-fun remuw () Bool (= encoding #x0200703b))

; 8命令以外では参照する4出力もすべて false になる。
(assert
  (or (distinct recognized (or div divu rem remu divw divuw remw remuw))
      (distinct signed-mode (or div rem divw remw))
      (distinct word-mode (or divw divuw remw remuw))
      (distinct remainder-mode (or rem remu remw remuw))))
