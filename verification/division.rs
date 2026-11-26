//! 符号なし除算を最上位ビットから反復したときの商と余りを検証する。

use vstd::{
    arithmetic::{
        div_mod::lemma_fundamental_div_mod_converse,
        power2::{lemma_pow2_pos, lemma_pow2_strictly_increases, lemma_pow2_unfold, pow2},
    },
    prelude::*,
};

verus! {

/// 上位から並ぶビット列の整数値。
pub open spec fn bit_value(bits: Seq<bool>) -> nat
    decreases bits.len(),
{
    if bits.len() == 0 {
        0
    } else {
        2 * bit_value(bits.drop_last()) + bit_digit(bits.last())
    }
}

/// 真を1、偽を0と解釈する。
pub open spec fn bit_digit(bit: bool) -> nat { if bit { 1 } else { 0 } }

/// 旧余りに次のビットを取り込み、商の次のビットと確定した余りを返す。
pub open spec fn unsigned_round(remainder: nat, divisor: nat, bit: bool) -> (bool, nat) {
    let trial = 2 * remainder + bit_digit(bit);
    let quotient_bit = trial >= divisor;
    (quotient_bit, if quotient_bit { (trial - divisor) as nat } else { trial })
}

/// 上位ビットから処理した後の商と余り。初期状態は両方ともゼロ。
pub open spec fn unsigned_divide(bits: Seq<bool>, divisor: nat) -> (nat, nat)
    decreases bits.len(),
{
    if bits.len() == 0 {
        (0, 0)
    } else {
        let (quotient, remainder) = unsigned_divide(bits.drop_last(), divisor);
        let (quotient_bit, next_remainder) = unsigned_round(remainder, divisor, bits.last());
        (2 * quotient + bit_digit(quotient_bit), next_remainder)
    }
}

/// 取り込んだビット列の値は、その幅の非負整数に収まる。
pub proof fn bit_value_fits(bits: Seq<bool>)
    ensures bit_value(bits) < pow2(bits.len()),
    decreases bits.len(),
{
    if bits.len() == 0 {
        lemma_pow2_pos(0);
    } else {
        bit_value_fits(bits.drop_last());
        lemma_pow2_unfold(bits.len());
    }
}

/// 反復で被除数の恒等式と余りの範囲を保つ。除数0では商に1が並ぶ。
pub proof fn unsigned_divide_invariant(bits: Seq<bool>, divisor: nat)
    ensures ({
        let (quotient, remainder) = unsigned_divide(bits, divisor);
        if divisor == 0 {
            quotient + 1 == pow2(bits.len()) && remainder == bit_value(bits)
        } else {
            bit_value(bits) == divisor * quotient + remainder && remainder < divisor
        }
    }),
    decreases bits.len(),
{
    if bits.len() == 0 {
        reveal(vstd::arithmetic::power::pow);
        assert(pow2(0) == 1);
        assert(unsigned_divide(bits, divisor) == (0nat, 0nat));
        assert(bit_value(bits) == 0);
        assert(divisor * 0nat == 0) by (nonlinear_arith);
    } else {
        let prior = bits.drop_last();
        unsigned_divide_invariant(prior, divisor);
        let (quotient, remainder) = unsigned_divide(prior, divisor);
        let bit = bits.last();
        let (quotient_bit, next_remainder) = unsigned_round(remainder, divisor, bit);
        let next_quotient = 2 * quotient + bit_digit(quotient_bit);
        let trial = 2 * remainder + bit_digit(bit);
        assert(unsigned_divide(bits, divisor) == (next_quotient, next_remainder));
        assert(bit_value(bits) == 2 * bit_value(prior) + bit_digit(bit));

        if divisor == 0 {
            lemma_pow2_unfold(bits.len());
            assert(quotient + 1 == pow2(prior.len()) && remainder == bit_value(prior));
            assert(quotient_bit && next_remainder == trial);
            assert(next_quotient + 1 == pow2(bits.len()));
            assert(next_remainder == bit_value(bits));
        } else {
            assert(bit_value(prior) == divisor * quotient + remainder && remainder < divisor);
            assert(next_remainder < divisor);
            assert(divisor * next_quotient + next_remainder
                == 2 * (divisor * quotient + remainder) + bit_digit(bit)) by (nonlinear_arith)
                requires
                    next_quotient == 2 * quotient + bit_digit(quotient_bit),
                    bit_digit(quotient_bit) == if quotient_bit { 1nat } else { 0nat },
                    trial == 2 * remainder + bit_digit(bit),
                    next_remainder == if quotient_bit { (trial - divisor) as nat } else { trial },
                    quotient_bit ==> trial >= divisor;
            assert(divisor * next_quotient + next_remainder == bit_value(bits));
        }
    }
}

/// 32ビットまたは64ビットの反復結果を、RISC-V の符号なし除算と剰余の仕様に照合する。
///
/// 結果は選んだ演算幅までの値を表し、32ビット命令の書き戻しに必要な符号拡張は別に行う。
pub proof fn unsigned_divide_rv64(bits: Seq<bool>, divisor: nat)
    requires (bits.len() == 32 || bits.len() == 64), divisor < pow2(bits.len()),
    ensures ({
        let (quotient, remainder) = unsigned_divide(bits, divisor);
        &&& quotient < pow2(bits.len())
        &&& remainder < pow2(bits.len())
        &&& if divisor == 0 {
            quotient + 1 == pow2(bits.len()) && remainder == bit_value(bits)
        } else {
            quotient == bit_value(bits) / divisor && remainder == bit_value(bits) % divisor
        }
    }),
{
    bit_value_fits(bits);
    unsigned_divide_invariant(bits, divisor);
    let (quotient, remainder) = unsigned_divide(bits, divisor);
    if divisor > 0 {
        let dividend = bit_value(bits);
        lemma_fundamental_div_mod_converse(dividend as int, divisor as int, quotient as int, remainder as int);
        assert(quotient <= dividend) by (nonlinear_arith)
            requires divisor >= 1, dividend == divisor * quotient + remainder;
    }
}

/// 64ビットまでの各捕捉で、比較前の試行値と確定した余りが64ビットに収まる。
pub proof fn unsigned_round_width(prefix: Seq<bool>, divisor: nat, bit: bool)
    requires prefix.len() < 64, divisor < pow2(64),
    ensures
        2 * unsigned_divide(prefix, divisor).1 + bit_digit(bit) < pow2(64),
        unsigned_round(unsigned_divide(prefix, divisor).1, divisor, bit).1 < pow2(64),
{
    unsigned_divide_invariant(prefix, divisor);
    let (quotient, remainder) = unsigned_divide(prefix, divisor);
    let (_, next_remainder) = unsigned_round(remainder, divisor, bit);
    let trial = 2 * remainder + bit_digit(bit);
    bit_value_fits(prefix);
    lemma_pow2_unfold(prefix.len() + 1);
    if prefix.len() + 1 < 64 {
        lemma_pow2_strictly_increases(prefix.len() + 1, 64);
    }
    if divisor == 0 {
        assert(remainder == bit_value(prefix));
    } else {
        assert(remainder <= bit_value(prefix)) by (nonlinear_arith)
            requires bit_value(prefix) == divisor * quotient + remainder, 0 < divisor;
    }
    assert(trial < pow2(64));
    assert(next_remainder <= trial);
}

/// 符号付きオペランドの絶対値を整数で表す。最小値の絶対値も含む。
pub open spec fn magnitude(value: int) -> nat {
    if value < 0 { (-value) as nat } else { value as nat }
}

/// 絶対値で得た商と余りを、演算幅内の符号付き結果に戻す。
///
/// `bound` は表現できる最大の値に1を加えた数。ゼロ除算と最小値を負の1で割る場合は
/// RISC-V の規定値を返す。
pub open spec fn signed_result(dividend: int, divisor: int, bound: int, q: nat, r: nat) -> (int, int) {
    if divisor == 0 {
        (-1, dividend)
    } else if dividend == -bound && divisor == -1 {
        (dividend, 0)
    } else {
        let quotient = if (dividend < 0) != (divisor < 0) { -(q as int) } else { q as int };
        let remainder = if dividend < 0 { -(r as int) } else { r as int };
        (quotient, remainder)
    }
}

/// RISC-V の符号付き商・余りを、演算幅、恒等式、余りの範囲と符号で定める。
pub open spec fn signed_division_matches(
    dividend: int, divisor: int, bound: int, result: (int, int),
) -> bool {
    let (quotient, remainder) = result;
    &&& -bound <= quotient < bound
    &&& -bound <= remainder < bound
    &&& if divisor == 0 {
        quotient == -1 && remainder == dividend
    } else if dividend == -bound && divisor == -1 {
        quotient == dividend && remainder == 0
    } else {
        &&& dividend == divisor * quotient + remainder
        &&& magnitude(remainder) < magnitude(divisor)
        &&& remainder != 0 ==> (remainder < 0) == (dividend < 0)
    }
}

/// 絶対値による除算から、ゼロ方向へ丸める符号付き除算の結果を得る。
pub proof fn signed_result_matches(dividend: int, divisor: int, bound: int, q: nat, r: nat)
    requires
        bound >= 2,
        -bound <= dividend < bound,
        -bound <= divisor < bound,
        divisor != 0 ==> magnitude(dividend) == magnitude(divisor) * q + r && r < magnitude(divisor),
    ensures signed_division_matches(dividend, divisor, bound, signed_result(dividend, divisor, bound, q, r)),
{
    if divisor != 0 && !(dividend == -bound && divisor == -1) {
        let n = magnitude(dividend);
        let d = magnitude(divisor);
        assert(q <= n <= bound) by (nonlinear_arith)
            requires d >= 1, n <= bound, n == d * q + r;
        if (dividend < 0) == (divisor < 0) && q == bound {
            assert(d == 1 && n == bound) by (nonlinear_arith)
                requires n <= bound, n == d * q + r, q == bound, bound >= 2, d >= 1;
            assert(dividend == -bound && divisor == -1);
        }
        let (quotient, remainder) = signed_result(dividend, divisor, bound, q, r);
        assert(dividend == divisor * quotient + remainder) by (nonlinear_arith)
            requires
                n == d * q + r,
                n == if dividend < 0 { -dividend } else { dividend },
                d == if divisor < 0 { -divisor } else { divisor },
                quotient == if (dividend < 0) != (divisor < 0) { -(q as int) } else { q as int },
                remainder == if dividend < 0 { -(r as int) } else { r as int };
    }
}

/// 符号付きオペランドを絶対値で除算し、補正後の32ビットまたは64ビットの結果を保証する。
pub proof fn signed_divide_rv64(bits: Seq<bool>, dividend: int, divisor: int)
    requires
        bits.len() == 32 || bits.len() == 64,
        -pow2((bits.len() - 1) as nat) <= dividend < pow2((bits.len() - 1) as nat),
        -pow2((bits.len() - 1) as nat) <= divisor < pow2((bits.len() - 1) as nat),
        bit_value(bits) == magnitude(dividend),
    ensures ({
        let bound = pow2((bits.len() - 1) as nat) as int;
        let (q, r) = unsigned_divide(bits, magnitude(divisor));
        signed_division_matches(dividend, divisor, bound, signed_result(dividend, divisor, bound, q, r))
    }),
{
    let bound = pow2((bits.len() - 1) as nat) as int;
    lemma_pow2_unfold((bits.len() - 1) as nat);
    lemma_pow2_pos((bits.len() - 2) as nat);
    unsigned_divide_invariant(bits, magnitude(divisor));
    let (q, r) = unsigned_divide(bits, magnitude(divisor));
    signed_result_matches(dividend, divisor, bound, q, r);
}

proof fn division_examples() {
    reveal_with_fuel(unsigned_divide, 5);
    assert(unsigned_divide(seq![true, false, true, true], 3) == (3nat, 2nat)) by (compute_only);
    assert(unsigned_divide(seq![true, false, true, true], 0) == (15nat, 11nat)) by (compute_only);
    assert(signed_result(-11, 3, 16, 3, 2) == (-3int, -2int)) by (compute_only);
    assert(signed_result(-16, -1, 16, 16, 0) == (-16int, 0int)) by (compute_only);
}

} // verus!
