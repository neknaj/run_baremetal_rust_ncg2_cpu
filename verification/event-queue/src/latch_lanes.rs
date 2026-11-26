//! 連続して配線されたラッチ入力の変更箇所をまとめて調べる。

use vstd::prelude::*;

use crate::fanout::Pin;

verus! {

/// ラッチの先頭ビットを除く端子の連続性を検査し、確認できた範囲の先頭を返す。
pub fn consecutive_tail(pins: &[Pin]) -> (start: Option<usize>)
    ensures
        start matches Some(first) ==> pins@.len() > 1
            && first as int + pins@.len() - 2 <= usize::MAX
            && forall|i: int| 1 <= i < pins@.len() ==>
                #[trigger] pins@[i] == Pin::Slot((first as int + i - 1) as usize),
{
    if pins.len() <= 1 {
        return None;
    }
    let Pin::Slot(first) = pins[1] else {
        return None;
    };
    if pins.len() - 2 > usize::MAX - first {
        return None;
    }
    let mut index = 2;
    while index < pins.len()
        invariant
            2 <= index <= pins@.len(),
            first as int + pins@.len() - 2 <= usize::MAX,
            forall|i: int| 1 <= i < index ==> #[trigger] pins@[i] == Pin::Slot((first as int + i - 1) as usize),
        decreases pins.len() - index,
    {
        let Pin::Slot(slot) = pins[index] else {
            return None;
        };
        if slot != first + (index - 1) {
            return None;
        }
        index += 1;
    }
    Some(first)
}

/// 8本の配線のうち、値が変わった配線のビットだけを1にする。
#[inline]
pub fn changed_lanes(old: &[bool; 8], next: &[bool; 8]) -> (mask: u8)
    ensures
        forall|i: usize| i < 8 ==> (#[trigger] (mask >> i) & 1 != 0)
            == (old@[i as int] != next@[i as int]),
{
    let d0 = (old[0] != next[0]) as u8;
    let d1 = (old[1] != next[1]) as u8;
    let d2 = (old[2] != next[2]) as u8;
    let d3 = (old[3] != next[3]) as u8;
    let d4 = (old[4] != next[4]) as u8;
    let d5 = (old[5] != next[5]) as u8;
    let d6 = (old[6] != next[6]) as u8;
    let d7 = (old[7] != next[7]) as u8;
    let result = d0 | (d1 << 1) | (d2 << 2) | (d3 << 3)
        | (d4 << 4) | (d5 << 5) | (d6 << 6) | (d7 << 7);
    assert forall|i: usize| i < 8 implies (#[trigger] (result >> i) & 1)
        == if i == 0 { d0 } else if i == 1 { d1 } else if i == 2 { d2 } else if i == 3 { d3 }
            else if i == 4 { d4 } else if i == 5 { d5 } else if i == 6 { d6 } else { d7 } by {
        assert((result >> i) & 1 ==
            if i == 0 { d0 } else if i == 1 { d1 } else if i == 2 { d2 } else if i == 3 { d3 }
            else if i == 4 { d4 } else if i == 5 { d5 } else if i == 6 { d6 } else { d7 })
        by(bit_vector)
        requires i < 8, d0 <= 1, d1 <= 1, d2 <= 1, d3 <= 1, d4 <= 1, d5 <= 1, d6 <= 1, d7 <= 1,
            result == d0 | (d1 << 1) | (d2 << 2) | (d3 << 3) | (d4 << 4) | (d5 << 5) | (d6 << 6) | (d7 << 7);
    };
    result
}

} // verus!
