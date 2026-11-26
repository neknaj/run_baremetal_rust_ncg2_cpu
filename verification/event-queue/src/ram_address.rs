//! RAM の入力ピンをアドレス順に解釈し、対象の語を求める。

use crate::fanout::Pin;
use vstd::prelude::*;
#[cfg(verus_only)]
use crate::nor_inputs::pin_fits;
#[cfg(verus_only)]
use vstd::{
    arithmetic::power2::{lemma_pow2_pos, lemma_pow2_unfold, pow2},
    bits::{lemma_usize_pow2_no_overflow, lemma_usize_shl_is_mul},
};

verus! {

/// 最下位ビットを先頭に置くアドレスの値を、ピンが読む配線値から定める。
pub open spec fn address_value(pins: Seq<Pin>, values: Seq<bool>) -> nat
    decreases pins.len(),
{
    if pins.len() == 0 {
        0
    } else {
        2 * address_value(pins.skip(1), values) + if pins[0].value_model(values) { 1nat } else { 0nat }
    }
}

proof fn address_examples() {
    reveal_with_fuel(address_value, 4);
    assert(address_value(seq![Pin::Slot(1), Pin::Constant(false), Pin::Slot(0)], seq![true, false]) == 4)
        by (compute_only);
    assert(address_value(seq![Pin::Constant(true), Pin::Slot(1)], seq![true, false]) == 1)
        by (compute_only);
}

/// アドレス幅と語幅に対応する記憶ビット数を返す。
///
/// 語数または記憶ビット数が `usize` の範囲外なら `None` を返す。
pub fn ram_capacity(address_width: usize, word_width: usize) -> (capacity: Option<usize>)
    ensures
        capacity.is_some() <==> address_width < usize::BITS
            && pow2(address_width as nat) * word_width <= usize::MAX,
        capacity matches Some(bits) ==> bits == pow2(address_width as nat) * word_width,
{
    if address_width >= usize::BITS as usize {
        return None;
    }
    proof {
        lemma_usize_pow2_no_overflow(address_width as nat);
        lemma_usize_shl_is_mul(1, address_width);
    }
    (1usize << address_width).checked_mul(word_width)
}

/// 同じ tick のアドレス入力を読み、対象の語の先頭を返す。
///
/// 呼び出し側は、構築時に `ram_capacity` で記憶領域の容量を検査し、すべてのピンが
/// 値の列の範囲内を指すことを確認する。
///
/// # Panics
///
/// アドレス用のピンが値の列の範囲外を指す場合にパニックする。
/// オーバーフロー検査が有効なビルドでは、アドレス値の累積または語の先頭添字の計算が
/// `usize` を超えた場合にもパニックする。
#[inline]
pub fn ram_word_start(pins: &[Pin], values: &[bool], word_width: usize) -> (start: usize)
    requires
        pins@.len() < usize::BITS,
        pow2(pins@.len()) * word_width <= usize::MAX,
        forall|i: int| 0 <= i < pins@.len() ==> pin_fits(#[trigger] pins@[i], values@.len()),
    ensures
        start == address_value(pins@, values@) * word_width,
        start + word_width <= pow2(pins@.len()) * word_width,
{
    let mut word = 0usize;
    let mut left = pins.len();
    proof { lemma_pow2_pos(0); }
    while left != 0
        invariant
            left <= pins@.len() < usize::BITS,
            word == address_value(pins@.skip(left as int), values@),
            word < pow2((pins@.len() - left) as nat),
            forall|i: int| 0 <= i < pins@.len() ==> pin_fits(#[trigger] pins@[i], values@.len()),
        decreases left,
    {
        left -= 1;
        let bit = pins[left].read(values);
        proof {
            let width = (pins@.len() - left) as nat;
            lemma_pow2_unfold(width);
            lemma_usize_pow2_no_overflow(width);
            assert(pins@.skip(left as int)[0] == pins@[left as int]);
            assert(pins@.skip(left as int).skip(1) =~= pins@.skip(left + 1));
        }
        word = 2 * word + bit as usize;
    }
    assert(pins@.skip(0) =~= pins@);
    assert((word as int + 1) * word_width <= pow2(pins@.len()) * word_width) by (nonlinear_arith)
        requires word < pow2(pins@.len());
    assert((word as int + 1) * word_width == word * word_width + word_width) by (nonlinear_arith);
    word * word_width
}

} // verus!
