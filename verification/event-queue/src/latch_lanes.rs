//! 連続して配線されたラッチ入力の変更箇所をまとめて調べる。

use alloc::vec::Vec;
use vstd::prelude::*;
#[cfg(verus_only)]
use vstd::{seq_lib::lemma_seq_contains_after_push, std_specs::bits::axiom_u64_trailing_zeros};

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


/// 変更マスクから最小の添字を取り出し、更新後の値と組にする。
///
/// マスクが空の場合は None を返す。成功した場合は取り出したビットだけを消す。
#[inline]
pub fn take_latch_change(mask: &mut u64, next: u64) -> (result: Option<(usize, bool)>)
    ensures
        result.is_none() <==> *old(mask) == 0,
        result.is_none() ==> *final(mask) == *old(mask),
        result matches Some((lane, value)) ==> {
            &&& lane < 64
            &&& (*old(mask) >> lane) & 1 == 1
            &&& forall|i: usize| i < lane ==> (#[trigger] (*old(mask) >> i) & 1) == 0
            &&& value == ((next >> lane) & 1 != 0)
            &&& *final(mask) == *old(mask) & !(1u64 << lane)
            &&& *final(mask) < *old(mask)
        },
{
    let before = *mask;
    if before == 0 {
        return None;
    }
    let lane = before.trailing_zeros() as usize;
    proof {
        axiom_u64_trailing_zeros(before);
        assert(lane < 64);
        assert((before >> lane) & 1 == 1) by(bit_vector)
            requires (before >> lane as u64) & 1 == 1;
        assert forall|i: usize| i < lane implies (#[trigger] (before >> i) & 1) == 0 by {
            assert((before >> i as u64) & 1 == 0);
        };
    }
    let remaining = before & !(1u64 << lane);
    assert(remaining < before) by(bit_vector)
        requires lane < 64, remaining == before & !(1u64 << lane), (before >> lane) & 1 == 1;
    *mask = remaining;
    Some((lane, (next >> lane) & 1 != 0))
}


/// ワードの指定ビットを読む。呼び出し側は0から63までの添字を使う。
pub open spec fn lane_set(word: u64, lane: usize) -> bool {
    ((word >> lane) & 1) != 0
}

/// ワードの変更を、配線添字で昇順に並べた変更列として表す。
pub open spec fn shift_changes(
    changes: Seq<(usize, bool)>, base: usize, next: u64, mask: u64,
) -> bool {
    &&& forall|i: int| 0 <= i < changes.len() ==> {
        let change = #[trigger] changes[i];
        &&& base <= change.0
        &&& change.0 - base < 64
        &&& lane_set(mask, (change.0 - base) as usize)
        &&& change.1 == lane_set(next, (change.0 - base) as usize)
    }
    &&& forall|lane: usize| lane < 64 ==>
        #[trigger] lane_set(mask, lane)
        == changes.contains(((base + lane) as usize, lane_set(next, lane)))
    &&& forall|i: int, j: int| #![trigger changes[i], changes[j]] 0 <= i < j < changes.len() ==>
        changes[i].0 < changes[j].0
}

/// 既存の確定候補を保ち、変更マスクの配線を添字の昇順に追加する。
///
/// # Panics
///
/// 呼び出し側は、配線の先頭から63を加算でき、候補の列に最大64件を追加できる範囲の
/// 添字を渡す。実行器は構築時に確保した配線の範囲と候補の列を使う。
#[inline]
pub fn append_shift_changes(out: &mut Vec<(usize, bool)>, base: usize, next: u64, mask: u64)
    requires
        base as int + 63 <= usize::MAX,
        old(out)@.len() + 64 <= usize::MAX,
    ensures
        exists|suffix: Seq<(usize, bool)>|
            final(out)@ == old(out)@ + suffix && shift_changes(suffix, base, next, mask),
{
    let mut remaining = mask;
    let ghost mut added: Seq<(usize, bool)> = Seq::empty();
    let ghost mut floor: usize = 0;
    while remaining != 0
        invariant
            base as int + 63 <= usize::MAX,
            old(out)@.len() + 64 <= usize::MAX,
            out@ == old(out)@ + added,
            added.len() <= floor <= 64,
            forall|i: int| 0 <= i < added.len() ==> {
                let entry = #[trigger] added[i];
                &&& base <= entry.0
                &&& entry.0 - base < floor
                &&& lane_set(mask, (entry.0 - base) as usize)
                &&& entry.1 == lane_set(next, (entry.0 - base) as usize)
            },
            forall|i: int, j: int| #![trigger added[i], added[j]] 0 <= i < j < added.len() ==> added[i].0 < added[j].0,
            forall|lane: usize| lane < floor ==> !#[trigger] lane_set(remaining, lane),
            forall|lane: usize| lane < 64 ==> #[trigger] lane_set(mask, lane)
                == (lane_set(remaining, lane)
                    || added.contains(((base + lane) as usize, lane_set(next, lane)))),
        decreases remaining,
    {
        let ghost before = remaining;
        let (lane, value) = take_latch_change(&mut remaining, next).unwrap();
        assert(lane_set(before, lane));
        assert(lane >= floor);
        let update = (base + lane, value);
        let ghost prefix = added;
        out.push(update);
        proof {
            added = added.push(update);
            floor = (lane + 1) as usize;
            assert(out@ =~= old(out)@ + added);
            assert forall|i: usize| i < 64 implies
                lane_set(remaining, i) == (i != lane && lane_set(before, i))
            by {
                assert(((remaining >> i) & 1 != 0) == (i != lane && ((before >> i) & 1 != 0)))
                    by(bit_vector)
                    requires i < 64, lane < 64, remaining == before & !(1u64 << lane);
            };
            assert forall|i: usize| i < floor implies !#[trigger] lane_set(remaining, i) by {
                if i < lane {
                    assert((before >> i) & 1 == 0);
                }
            };
            assert forall|i: usize| i < 64 implies #[trigger] lane_set(mask, i)
                == (lane_set(remaining, i)
                    || added.contains(((base + i) as usize, lane_set(next, i))))
            by {
                let entry = ((base + i) as usize, lane_set(next, i));
                lemma_seq_contains_after_push(prefix, update, entry);
                assert(entry == update <==> i == lane);
                assert(lane_set(mask, i)
                    == (lane_set(before, i) || prefix.contains(entry)));
            };
        }
    }
    assert forall|lane: usize| lane < 64 implies #[trigger] lane_set(mask, lane)
        == added.contains(((base + lane) as usize, lane_set(next, lane)))
    by {
        assert(((remaining >> lane) & 1) == 0) by(bit_vector) requires remaining == 0, lane < 64;
        assert(!lane_set(remaining, lane));
    };
    assert(shift_changes(added, base, next, mask));
}

} // verus!
