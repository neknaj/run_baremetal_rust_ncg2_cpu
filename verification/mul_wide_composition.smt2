; 反復 i の不変条件は Z = remaining + multiplicand * consumed * 2^(64-i)。
; weight=2^i、half-period=2^(63-i)。i<64 なので反復前の period は偶数。
(set-logic QF_NIA)
(set-option :timeout 30000)
(declare-const multiplicand Int)
(declare-const consumed Int)
(declare-const next-remaining Int)
(declare-const bit Int)
(declare-const weight Int)
(declare-const half-period Int)
(assert (or (= bit 0) (= bit 1)))

(define-fun remaining () Int (+ (* 2 next-remaining) bit))
(define-fun period () Int (* 2 half-period))
(define-fun packed () Int (+ remaining (* multiplicand consumed period)))
(define-fun room () Int (* weight period))
(define-fun consumed-next () Int (+ consumed (* bit weight)))

; 加算項も偶数なので、1ビット右へずらすと捨てるビットは remaining の最下位ビット。
(push)
(assert (distinct (mod (+ packed (* bit multiplicand room)) 2) bit))
(check-sat)
(pop)

; 右シフト後の不変条件は、2倍して比較すると整数の多項式の等式になる。
(assert
  (distinct
    (- (+ packed (* bit multiplicand room)) bit)
    (* 2 (+ next-remaining (* multiplicand consumed-next half-period)))))
(check-sat)
