//! 連続して配線されたラッチ入力の変更箇所をまとめて調べる。

use alloc::vec::Vec;
use vstd::prelude::*;
#[cfg(verus_only)]
use vstd::{seq_lib::lemma_seq_contains_after_push, std_specs::bits::axiom_u64_trailing_zeros};

use crate::fanout::Pin;
#[cfg(verus_only)]
use crate::wire_updates::{apply_wire_updates, assigned_wire_value};

verus! {

/// 2番目以降の端子が、指定した配線から順に連続していることを表す。
pub open spec fn consecutive_inputs(pins: Seq<Pin>, first: usize) -> bool {
    &&& pins.len() > 1
    &&& first as int + pins.len() - 2 <= usize::MAX
    &&& forall|i: int| 1 <= i < pins.len() ==>
        #[trigger] pins[i] == Pin::Slot((first as int + i - 1) as usize)
}

/// ラッチの先頭ビットを除く端子の連続性を検査し、確認できた範囲の先頭を返す。
pub fn consecutive_tail(pins: &[Pin]) -> (start: Option<usize>)
    ensures
        forall|first: usize| #[trigger] consecutive_inputs(pins@, first)
            <==> start == Some(first),
{
    if pins.len() <= 1 {
        return None;
    }
    let Pin::Slot(first) = pins[1] else {
        assert forall|base: usize| !#[trigger] consecutive_inputs(pins@, base) by {
            if consecutive_inputs(pins@, base) {
                assert(pins@[1] == Pin::Slot(base));
            }
        };
        return None;
    };
    if pins.len() - 2 > usize::MAX - first {
        assert forall|base: usize| !#[trigger] consecutive_inputs(pins@, base) by {
            if consecutive_inputs(pins@, base) {
                assert(pins@[1] == Pin::Slot(base));
            }
        };
        return None;
    }
    let mut index = 2;
    while index < pins.len()
        invariant
            2 <= index <= pins@.len(),
            pins@[1] == Pin::Slot(first),
            first as int + pins@.len() - 2 <= usize::MAX,
            forall|i: int| 1 <= i < index ==> #[trigger] pins@[i] == Pin::Slot((first as int + i - 1) as usize),
        decreases pins.len() - index,
    {
        let Pin::Slot(slot) = pins[index] else {
            assert forall|base: usize| !#[trigger] consecutive_inputs(pins@, base) by {
                if consecutive_inputs(pins@, base) {
                    assert(pins@[index as int] is Slot);
                }
            };
            return None;
        };
        if slot != first + (index - 1) {
            assert forall|base: usize| !#[trigger] consecutive_inputs(pins@, base) by {
                if consecutive_inputs(pins@, base) {
                    assert(pins@[1] == Pin::Slot(base));
                    assert(pins@[index as int] == Pin::Slot((base as int + index - 1) as usize));
                }
            };
            return None;
        }
        index += 1;
    }
    assert(consecutive_inputs(pins@, first));
    assert forall|base: usize| #[trigger] consecutive_inputs(pins@, base) implies base == first by {
        assert(pins@[1] == Pin::Slot(base));
    };
    Some(first)
}

/// 連続配線の先頭と、ワードシフトで更新できるかを返す。
///
/// ラッチの出力区間の先頭を渡す。ワードシフトは連続比較も有効な場合に選ぶ。
pub fn latch_input_layout(
    pins: &[Pin], output_start: usize, lanes: bool, shift: bool,
) -> (layout: (Option<usize>, bool))
    ensures
        forall|first: usize| layout.0 == Some(first)
            <==> lanes && pins@.len() >= 9 && #[trigger] consecutive_inputs(pins@, first),
        layout.1 <==> lanes && shift && 9 <= pins@.len() <= 64
            && consecutive_inputs(pins@, output_start),
{
    let first = if lanes && pins.len() >= 9 { consecutive_tail(pins) } else { None };
    let same_output = match first {
        Some(first) => first == output_start,
        None => false,
    };
    (first, shift && pins.len() <= 64 && same_output)
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


/// 先頭の採取値と、1つ手前の旧出力から、指定されたラッチの次の値を求める。
pub open spec fn shift_input_at(old: u64, head: bool, lane: usize) -> bool {
    if lane == 0 { head } else { lane_set(old, (lane - 1) as usize) }
}

/// 旧ラッチの各ビットを従来の順序で比較した場合に得る確定候補を表す。
pub open spec fn shifted_latch_updates(
    updates: Seq<(usize, bool)>, base: usize, width: u32, old: u64, head: bool,
) -> bool {
    &&& forall|i: int| 0 <= i < updates.len() ==> {
        let (slot, value) = #[trigger] updates[i];
        let lane = (slot - base) as usize;
        &&& base <= slot < base + width
        &&& value == shift_input_at(old, head, lane)
        &&& value != lane_set(old, lane)
    }
    &&& forall|lane: usize| lane < width ==>
        (shift_input_at(old, head, lane) != lane_set(old, lane))
        == #[trigger] updates.contains(((base + lane) as usize, shift_input_at(old, head, lane)))
    &&& forall|i: int, j: int| #![trigger updates[i], updates[j]]
        0 <= i < j < updates.len() ==> updates[i].0 < updates[j].0
}

/// 次のラッチ状態を返し、旧出力との比較で変わる配線を確定候補へ追加する。
///
/// 幅を表せない場合は None を返し、候補の列を保つ。
///
/// # Panics
///
/// 配線の先頭から63を加算でき、候補の列に最大64件を追加できる範囲を渡す。
#[inline]
pub fn evaluate_shift_latch(
    out: &mut Vec<(usize, bool)>, base: usize, old_word: u64, width: u32, head: bool,
) -> (result: Option<u64>)
    requires
        base as int + 63 <= usize::MAX,
        old(out)@.len() + 64 <= usize::MAX,
    ensures
        result.is_some() <==> 1 <= width <= 64,
        result.is_none() ==> final(out)@ == old(out)@,
        result matches Some(next) ==> {
            &&& forall|lane: usize| lane < 64 ==> #[trigger] lane_set(next, lane)
                == (lane < width && shift_input_at(old_word, head, lane))
            &&& exists|suffix: Seq<(usize, bool)>|
                final(out)@ == old(out)@ + suffix
                    && #[trigger] shifted_latch_updates(suffix, base, width, old_word, head)
        },
{
    let Some((next, mask)) = shift_latch_word(old_word, width, head) else {
        return None;
    };
    assert forall|lane: usize| lane < 64 implies #[trigger] lane_set(next, lane)
        == (lane < width && shift_input_at(old_word, head, lane))
    by {
        let i = lane as u32;
        if lane < width {
            assert(((next >> i) & 1 != 0) ==
                if i == 0 { head } else { ((old_word >> ((i - 1) as u32)) & 1 != 0) });
            assert((next >> i) == (next >> lane)) by(bit_vector) requires i == lane as u32, lane < 64;
            if lane != 0 {
                assert((old_word >> ((i - 1) as u32)) == (old_word >> ((lane - 1) as usize)))
                    by(bit_vector) requires i == lane as u32, 0 < lane < 64;
            }
        } else {
            assert((next >> i) & 1 == 0);
            assert((next >> i) == (next >> lane)) by(bit_vector) requires i == lane as u32, lane < 64;
        }
    };
    assert forall|lane: usize| lane < 64 implies #[trigger] lane_set(mask, lane)
        == (lane < width && lane_set(old_word, lane) != lane_set(next, lane))
    by {
        let i = lane as u32;
        assert((mask >> i) & 1 != 0
            == (i < width && ((old_word >> i) & 1) != ((next >> i) & 1)));
        assert((mask >> i) == (mask >> lane) && (old_word >> i) == (old_word >> lane)
            && (next >> i) == (next >> lane))
            by(bit_vector) requires i == lane as u32, lane < 64;
        assert(((old_word >> lane) & 1 != 0) != ((next >> lane) & 1 != 0)
            == (((old_word >> lane) & 1) != ((next >> lane) & 1))) by(bit_vector);
    };
    append_shift_changes(out, base, next, mask);
    proof {
        let suffix = choose|suffix: Seq<(usize, bool)>|
            out@ == old(out)@ + suffix && shift_changes(suffix, base, next, mask);
        assert forall|i: int| 0 <= i < suffix.len() implies {
            let (slot, value) = #[trigger] suffix[i];
            let lane = (slot - base) as usize;
            &&& base <= slot < base + width
            &&& value == shift_input_at(old_word, head, lane)
            &&& value != lane_set(old_word, lane)
        } by {
            let (slot, value) = suffix[i];
            let lane = (slot - base) as usize;
            assert(lane_set(mask, lane));
            assert(lane_set(next, lane) == (lane < width && shift_input_at(old_word, head, lane)));
        };
        assert forall|lane: usize| lane < width implies
            (shift_input_at(old_word, head, lane) != lane_set(old_word, lane))
            == #[trigger] suffix.contains(((base + lane) as usize, shift_input_at(old_word, head, lane)))
        by {
            assert(lane_set(mask, lane)
                == suffix.contains(((base + lane) as usize, lane_set(next, lane))));
        };
        assert(shifted_latch_updates(suffix, base, width, old_word, head));
    }
    Some(next)
}

/// キャッシュしたワードと確定済みのラッチ出力が一致することを表す。
pub open spec fn word_matches_slots(word: u64, slots: Seq<bool>, base: usize, width: int) -> bool {
    forall|lane: usize| lane < width ==> #[trigger] lane_set(word, lane) == slots[base + lane]
}

/// 実配線の各入力を旧状態から読み、出力が変わる箇所だけを順に選んだ列を表す。
pub open spec fn latch_pin_updates(
    updates: Seq<(usize, bool)>, slots: Seq<bool>, pins: Seq<Pin>, base: usize,
) -> bool {
    &&& forall|i: int| 0 <= i < updates.len() ==> {
        let (slot, value) = #[trigger] updates[i];
        &&& base <= slot < base + pins.len()
        &&& value == pins[slot - base].value_model(slots)
        &&& value != slots[slot as int]
    }
    &&& forall|lane: int| 0 <= lane < pins.len() ==> {
        (pins[lane].value_model(slots) != slots[base + lane])
            == #[trigger] updates.contains(((base + lane) as usize, pins[lane].value_model(slots)))
    }
    &&& forall|i: int, j: int| #![trigger updates[i], updates[j]]
        0 <= i < j < updates.len() ==> updates[i].0 < updates[j].0
}

/// 他の素子がラッチの出力を書かない場合、候補の確定後にキャッシュと配線が一致する。
pub proof fn shift_cache_after_commit(
    slots: Seq<bool>, wires: Seq<bool>, pins: Seq<Pin>, base: usize, next: u64,
    before: Seq<(usize, bool)>, own: Seq<(usize, bool)>, after: Seq<(usize, bool)>,
)
    requires
        9 <= pins.len() <= 64,
        base + pins.len() <= wires.len() <= usize::MAX,
        wires.len() <= slots.len(),
        wires == slots.take(wires.len() as int),
        latch_pin_updates(own, slots, pins, base),
        forall|lane: usize| lane < pins.len() ==> #[trigger] lane_set(next, lane)
            == pins[lane as int].value_model(slots),
        forall|i: int| 0 <= i < before.len() ==>
            !(base <= (#[trigger] before[i]).0 < base + pins.len()),
        forall|i: int| 0 <= i < after.len() ==>
            !(base <= (#[trigger] after[i]).0 < base + pins.len()),
    ensures
        word_matches_slots(next, apply_wire_updates(wires, before + own + after), base, pins.len() as int),
{
    let all = before + own + after;
    assert forall|lane: usize| lane < pins.len() implies #[trigger] lane_set(next, lane)
        == apply_wire_updates(wires, all)[base + lane] by {
        let slot = (base + lane) as usize;
        let value = pins[lane as int].value_model(slots);
        assert forall|i: int| 0 <= i < all.len() && (#[trigger] all[i]).0 == slot
            implies all[i].1 == value by {
            if i < before.len() {
                assert(all[i] == before[i]);
            } else if i < before.len() + own.len() {
                let j = i - before.len();
                assert(all[i] == own[j]);
                assert(own[j].1 == pins[slot - base].value_model(slots));
            } else {
                assert(all[i] == after[i - before.len() - own.len()]);
            }
        };
        if wires[slot as int] != value {
            let offset = lane as int;
            assert(slot as int == base + offset);
            assert(wires[base + offset] == slots[base + offset]);
            assert((pins[offset].value_model(slots) != slots[base + offset])
                == own.contains(((base + offset) as usize, pins[offset].value_model(slots))));
            assert(own.contains((slot, value)));
            let at = choose|at: int| 0 <= at < own.len() && own[at] == (slot, value);
            assert(all[before.len() + at] == (slot, value));
            assert(all.contains((slot, value)));
        }
        assigned_wire_value(wires, all, slot, value);
    };
}

/// 自分の旧出力を1段ずらして読むラッチについて、実配線の確定候補と次のワードを求める。
///
/// 先頭の端子だけを値の列から読み、残りは評価前の出力と一致するワードを使う。
/// 幅9〜64で、先頭以外が連続する旧出力を読むラッチに使う。呼び出し側はワードと
/// 確定済み配線の一致を保ち、候補列に最大64件を追加できる添字の範囲を確保する。
///
/// # Panics
///
/// データ端子が空、または先頭端子が値の列の範囲外を指すとパニックする。
#[inline]
pub fn evaluate_shift_pins(
    out: &mut Vec<(usize, bool)>, slots: &[bool], pins: &[Pin], base: usize, before: u64,
) -> (next: u64)
    requires
        9 <= pins@.len() <= 64,
        base as int + pins@.len() <= slots@.len(),
        base as int + 63 <= usize::MAX,
        old(out)@.len() + 64 <= usize::MAX,
        pins@[0] matches Pin::Slot(slot) ==> slot < slots@.len(),
        consecutive_inputs(pins@, base),
        word_matches_slots(before, slots@, base, pins@.len() as int),
    ensures
        forall|lane: usize| lane < pins@.len() ==> #[trigger] lane_set(next, lane)
            == pins@[lane as int].value_model(slots@),
        exists|suffix: Seq<(usize, bool)>|
            final(out)@ == old(out)@ + suffix && #[trigger] latch_pin_updates(suffix, slots@, pins@, base),
{
    let head = pins[0].read(slots);
    let width = pins.len() as u32;
    let Some(next) = evaluate_shift_latch(out, base, before, width, head) else {
        proof { assert(false); }
        return before;
    };
    proof {
        assert forall|lane: usize| lane < pins@.len() implies
            #[trigger] shift_input_at(before, head, lane) == pins@[lane as int].value_model(slots@) by {
            if lane > 0 {
                assert(pins@[lane as int] == Pin::Slot((base as int + lane - 1) as usize));
                assert(lane_set(before, (lane - 1) as usize) == slots@[base + lane - 1]);
            }
        };
        assert forall|lane: usize| lane < pins@.len() implies #[trigger] lane_set(next, lane)
            == pins@[lane as int].value_model(slots@) by {
            assert(lane_set(next, lane) == shift_input_at(before, head, lane));
        };
        let suffix = choose|suffix: Seq<(usize, bool)>|
            out@ == old(out)@ + suffix && #[trigger] shifted_latch_updates(suffix, base, width, before, head);
        assert forall|i: int| 0 <= i < suffix.len() implies {
            let (slot, value) = #[trigger] suffix[i];
            &&& base <= slot < base + pins@.len()
            &&& value == pins@[slot - base].value_model(slots@)
            &&& value != slots@[slot as int]
        } by {
            let (slot, value) = suffix[i];
            let lane = (slot - base) as usize;
            assert(value == shift_input_at(before, head, lane));
            assert(lane_set(before, lane) == slots@[slot as int]);
        };
        assert forall|lane: int| 0 <= lane < pins@.len() implies {
            (pins@[lane].value_model(slots@) != slots@[base + lane])
                == #[trigger] suffix.contains(((base + lane) as usize, pins@[lane].value_model(slots@)))
        } by {
            let lane_u = lane as usize;
            assert(shift_input_at(before, head, lane_u) == pins@[lane].value_model(slots@));
            assert(lane_set(before, lane_u) == slots@[base + lane]);
            assert((shift_input_at(before, head, lane_u) != lane_set(before, lane_u))
                == suffix.contains(((base + lane_u) as usize, shift_input_at(before, head, lane_u))));
        };
        assert(latch_pin_updates(suffix, slots@, pins@, base));
    }
    next
}

} // verus!
