//! 符号なし128ビット積から RV64 の上位語と W の結果を求める算術を証明する。

use vstd::{
    arithmetic::div_mod::{
        lemma_fundamental_div_mod_converse, lemma_mod_multiples_vanish, lemma_mul_mod_noop,
    },
    prelude::*,
};

verus! {

/// 64ビット語の値域の大きさ。
pub open spec fn base64() -> int { 0x10000000000000000 }

/// 32ビット語の値域の大きさ。
pub open spec fn base32() -> int { 0x100000000 }

/// 符号付きとして読み、最上位ビットが1の場合に負とみなす。
pub open spec fn is_negative(value: u64, signed: bool) -> bool {
    signed && value >= 0x8000000000000000u64
}

/// 命令の指定に従ってオペランドを整数として読む。
pub open spec fn operand_value(value: u64, signed: bool) -> int {
    if is_negative(value, signed) { value as int - base64() } else { value as int }
}

/// 符号なしの上位語から、負のオペランドに対応する補正量を引く。
pub open spec fn corrected_high(left: u64, right: u64, high: u64, signed_left: bool, signed_right: bool) -> int {
    let left_term = if is_negative(right, signed_right) { left as int } else { 0 };
    let right_term = if is_negative(left, signed_left) { right as int } else { 0 };
    (high as int - left_term - right_term) % base64()
}

/// 整数値の下位128ビットに含まれる上位64ビットを求める。
pub open spec fn product_high(product: int) -> int {
    (product / base64()) % base64()
}

/// 符号なしの積の2語から、指定された符号で乗算した積の上位語を得る。
pub proof fn corrected_high_matches(
    left: u64, right: u64, low: u64, high: u64, signed_left: bool, signed_right: bool,
)
    requires
        (left as int) * (right as int) == base64() * high as int + low as int,
    ensures
        corrected_high(left, right, high, signed_left, signed_right)
            == product_high(operand_value(left, signed_left) * operand_value(right, signed_right)),
        (operand_value(left, signed_left) * operand_value(right, signed_right)) % base64() == low,
{
    let base = base64();
    let left_negative = is_negative(left, signed_left);
    let right_negative = is_negative(right, signed_right);
    let left_term = if right_negative { left as int } else { 0 };
    let right_term = if left_negative { right as int } else { 0 };
    let cross_term = if left_negative && right_negative { 1int } else { 0 };
    let adjusted = high as int - left_term - right_term;
    let quotient = cross_term * base + adjusted;
    let product = operand_value(left, signed_left) * operand_value(right, signed_right);
    // 両入力が負のときだけ生じる2^128の項は、上位語を切り出すと消える。
    assert(product == base * quotient + low) by (nonlinear_arith)
        requires
            (left as int) * (right as int) == base * high as int + low as int,
            base == 0x10000000000000000,
            product == operand_value(left, signed_left) * operand_value(right, signed_right),
            adjusted == high as int - left_term - right_term,
            quotient == cross_term * base + adjusted,
            operand_value(left, signed_left) == if left_negative { left as int - base } else { left as int },
            operand_value(right, signed_right) == if right_negative { right as int - base } else { right as int },
            left_term == if right_negative { left as int } else { 0 },
            right_term == if left_negative { right as int } else { 0 },
            cross_term == if left_negative && right_negative { 1int } else { 0 };
    lemma_fundamental_div_mod_converse(product, base, quotient, low as int);
    lemma_mod_multiples_vanish(cross_term, adjusted, base);
}

/// 32ビットの符号を64ビットに拡張したビット列の整数値を返す。
pub open spec fn sign_extended_word(value: int) -> int {
    let low = value % base32();
    if low >= 0x80000000 { base64() - base32() + low } else { low }
}

/// 符号なし積の下位語から、下位32ビット同士を掛ける MULW の結果を得る。
pub proof fn word_result_matches(left: u64, right: u64, low: u64, high: u64)
    requires (left as int) * (right as int) == base64() * high as int + low as int,
    ensures
        sign_extended_word(low as int)
            == sign_extended_word(((left as int) % base32()) * ((right as int) % base32())),
{
    let base = base32();
    let product = (left as int) * (right as int);
    assert(product == base * (base * high as int) + low as int) by (nonlinear_arith)
        requires
            product == base64() * high as int + low as int,
            base64() == 0x10000000000000000, base == 0x100000000;
    lemma_mod_multiples_vanish(base * high as int, low as int, base);
    lemma_mul_mod_noop(left as int, right as int, base);
}

proof fn multiplication_examples() {
    corrected_high_matches(0xfffffffffffffffe, 0xfffffffffffffffd, 6, 0xfffffffffffffffb, true, true);
    assert(corrected_high(0xfffffffffffffffe, 0xfffffffffffffffd, 0xfffffffffffffffb, true, true) == 0)
        by (compute);
    assert(sign_extended_word(0x80000000) == 0xffffffff80000000) by (compute);
}

} // verus!
