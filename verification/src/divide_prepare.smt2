(define-fun normalized ((value (_ BitVec 64))) (_ BitVec 64)
  (ite word-mode
    (ite signed-mode
      ((_ sign_extend 32) ((_ extract 31 0) value))
      ((_ zero_extend 32) ((_ extract 31 0) value)))
    value))
(define-fun magnitude ((value (_ BitVec 64))) (_ BitVec 64)
  (ite (and signed-mode (bvslt value (_ bv0 64))) (bvneg value) value))
(assert
  (or (distinct prepared-dividend (magnitude (normalized dividend)))
      (distinct prepared-divisor (magnitude (normalized divisor)))))
