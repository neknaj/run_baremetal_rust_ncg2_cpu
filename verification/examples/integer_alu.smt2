; 命令語のフィールドから受理条件と演算を直接定める。
(define-fun opcode () (_ BitVec 7) ((_ extract 6 0) instruction))
(define-fun funct3 () (_ BitVec 3) ((_ extract 14 12) instruction))
(define-fun funct7 () (_ BitVec 7) ((_ extract 31 25) instruction))
(define-fun immediate-upper () (_ BitVec 6) ((_ extract 31 26) instruction))
(define-fun immediate () (_ BitVec 64) ((_ sign_extend 52) ((_ extract 31 20) instruction)))
(define-fun immediate-amount () (_ BitVec 64)
  ((_ zero_extend 58) ((_ extract 25 20) instruction)))
(define-fun register-amount () (_ BitVec 64) ((_ zero_extend 58) ((_ extract 5 0) rs2)))
(define-fun zero () (_ BitVec 64) (_ bv0 64))
(define-fun one () (_ BitVec 64) (_ bv1 64))

(define-fun add () Bool
  (and (= opcode #b0110011) (= funct7 #b0000000) (= funct3 #b000)))
(define-fun addiw () Bool
  (and (= opcode #b0011011) (= funct3 #b000)))
(define-fun and-register () Bool
  (and (= opcode #b0110011) (= funct7 #b0000000) (= funct3 #b111)))
(define-fun andi () Bool
  (and (= opcode #b0010011) (= funct3 #b111)))
(define-fun or-register () Bool
  (and (= opcode #b0110011) (= funct7 #b0000000) (= funct3 #b110)))
(define-fun ori () Bool
  (and (= opcode #b0010011) (= funct3 #b110)))
(define-fun sltiu () Bool
  (and (= opcode #b0010011) (= funct3 #b011)))
(define-fun srai () Bool
  (and (= opcode #b0010011) (= funct3 #b101) (= immediate-upper #b010000)))
(define-fun srl () Bool
  (and (= opcode #b0110011) (= funct7 #b0000000) (= funct3 #b101)))
(define-fun srli () Bool
  (and (= opcode #b0010011) (= funct3 #b101) (= immediate-upper #b000000)))
(define-fun sub () Bool
  (and (= opcode #b0110011) (= funct7 #b0100000) (= funct3 #b000)))
(define-fun xori () Bool
  (and (= opcode #b0010011) (= funct3 #b100)))
(define-fun accepted () Bool
  (or add addiw and-register andi or-register ori sltiu srai srl srli sub xori))

; 命令語全体で受理と拒否を確認する。
(push)
(assert (distinct valid accepted))
(check-sat)
(pop)

; 命令ごとの演算および拒否時の結果を全オペランドで確認する。
(push)
(assert add)
(assert (distinct result (bvadd rs1 rs2)))
(check-sat)
(pop)

(push)
(assert addiw)
(assert (distinct result
  ((_ sign_extend 32)
    (bvadd ((_ extract 31 0) rs1) ((_ sign_extend 20) ((_ extract 31 20) instruction))))))
(check-sat)
(pop)

(push)
(assert and-register)
(assert (distinct result (bvand rs1 rs2)))
(check-sat)
(pop)

(push)
(assert andi)
(assert (distinct result (bvand rs1 immediate)))
(check-sat)
(pop)

(push)
(assert or-register)
(assert (distinct result (bvor rs1 rs2)))
(check-sat)
(pop)

(push)
(assert ori)
(assert (distinct result (bvor rs1 immediate)))
(check-sat)
(pop)

(push)
(assert sltiu)
(assert (distinct result (ite (bvult rs1 immediate) one zero)))
(check-sat)
(pop)

(push)
(assert srai)
(assert (distinct result (bvashr rs1 immediate-amount)))
(check-sat)
(pop)

(push)
(assert srl)
(assert (distinct result (bvlshr rs1 register-amount)))
(check-sat)
(pop)

(push)
(assert srli)
(assert (distinct result (bvlshr rs1 immediate-amount)))
(check-sat)
(pop)

(push)
(assert sub)
(assert (distinct result (bvsub rs1 rs2)))
(check-sat)
(pop)

(push)
(assert xori)
(assert (distinct result (bvxor rs1 immediate)))
(check-sat)
(pop)

(assert (not accepted))
(assert (distinct result zero))
(check-sat)
