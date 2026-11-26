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


/// 幅以内の各ビットを1段進め、先頭に採取値を入れる。
///
/// 更新後の状態と変更ビットを返す。幅が1から64の範囲外なら None を返す。
#[inline]
pub fn shift_latch_word(old: u64, width: u32, head: bool) -> (result: Option<(u64, u64)>)
    ensures
        result.is_some() <==> 1 <= width <= 64,
        result matches Some((next, changed)) ==>
            (forall|i: u32| i < width ==> (#[trigger] (next >> i) & 1 != 0)
                == (if i == 0 { head } else { (old >> ((i - 1) as u32)) & 1 != 0 }))
            && (forall|i: u32| i < 64 ==> (#[trigger] (changed >> i) & 1 != 0)
                == (i < width && ((old >> i) & 1) != ((next >> i) & 1)))
            && (forall|i: u32| width <= i < 64 ==> (#[trigger] (next >> i) & 1) == 0),
{
    if width == 0 || width > 64 {
        return None;
    }
    let head_bit = head as u64;
    let limit = u64::MAX >> (64 - width);
    let next = ((old << 1) | head_bit) & limit;
    let changed = (old ^ next) & limit;
    assert forall|i: u32| i < 64 implies {
        &&& i < width ==> (#[trigger] (next >> i) & 1) ==
            (if i == 0 { head_bit } else { (old >> ((i - 1) as u32)) & 1 })
        &&& width <= i ==> ((next >> i) & 1) == 0
        &&& ((changed >> i) & 1 != 0)
            == (i < width && ((old >> i) & 1) != ((next >> i) & 1))
    } by {
        assert(
            (i < width ==> ((next >> i) & 1) ==
                (if i == 0 { head_bit } else { (old >> ((i - 1) as u32)) & 1 }))
            && (width <= i ==> ((next >> i) & 1) == 0)
            && (((changed >> i) & 1 != 0)
                == (i < width && ((old >> i) & 1) != ((next >> i) & 1)))
        ) by(bit_vector)
            requires 1 <= width <= 64, i < 64, head_bit <= 1,
                limit == u64::MAX >> ((64 - width) as u32),
                next == ((old << 1) | head_bit) & limit,
                changed == (old ^ next) & limit;
    };
    assert forall|i: u32| i < 64 implies (#[trigger] (changed >> i) & 1 != 0)
            == (i < width && ((old >> i) & 1) != ((next >> i) & 1)) by {
        assert(i < width ==> (next >> i) & 1 == if i == 0 { head_bit } else { (old >> ((i - 1) as u32)) & 1 });
    };
    Some((next, changed))
}

} // verus!
