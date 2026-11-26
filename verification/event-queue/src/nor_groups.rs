//! 同じ回路の複数の NOR を、同一 tick の配線値からまとめて評価する。

use super::fanout::Pin;
use vstd::prelude::*;

verus! {

/// 1ワード内で、ゲートの並びに対応する片側の入力を表す。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WordInputs {
    /// 先頭の添字から、ゲート順に連続した配線を読む。
    Consecutive(usize),
    /// 全ゲートが同じ配線を読む。
    Repeated(usize),
    /// 全ゲートが同じ定数を読む。
    Constant(bool),
}

/// 元の順序で並ぶ一群の NOR の入力。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NorWordGroup {
    /// 1以上64以下のゲート数。
    pub width: usize,
    /// 各ゲートの左側の入力。
    pub left: WordInputs,
    /// 各ゲートの右側の入力。
    pub right: WordInputs,
}

impl WordInputs {
    /// 指定したビットに対応する元の配線。
    pub open spec fn input_at(self, lane: int, pin: Pin) -> bool {
        match (self, pin) {
            (WordInputs::Consecutive(first), Pin::Slot(slot)) => slot == first + lane,
            (WordInputs::Repeated(first), Pin::Slot(slot)) => slot == first,
            (WordInputs::Constant(expected), Pin::Constant(actual)) => expected == actual,
            _ => false,
        }
    }
}

impl NorWordGroup {
    /// ゲートとその両入力の順序を保つことを表す。
    pub open spec fn matches(self, gates: Seq<(Pin, Pin)>) -> bool {
        &&& 1 <= self.width <= 64
        &&& self.width == gates.len()
        &&& forall|lane: int| 0 <= lane < gates.len() ==> {
            let (left, right) = #[trigger] gates[lane];
            &&& self.left.input_at(lane, left)
            &&& self.right.input_at(lane, right)
        }
    }
}

fn input(gate: (Pin, Pin), left: bool) -> (pin: Pin)
    ensures pin == if left { gate.0 } else { gate.1 },
{
    if left { gate.0 } else { gate.1 }
}

fn input_matches(candidate: WordInputs, lane: usize, pin: Pin) -> (matches: bool)
    ensures matches == candidate.input_at(lane as int, pin),
{
    match (candidate, pin) {
        (WordInputs::Consecutive(first), Pin::Slot(slot)) => {
            slot >= first && slot - first == lane
        },
        (WordInputs::Repeated(first), Pin::Slot(slot)) => slot == first,
        (WordInputs::Constant(expected), Pin::Constant(actual)) => expected == actual,
        _ => false,
    }
}

spec fn side_matches(gates: Seq<(Pin, Pin)>, left: bool, candidate: WordInputs) -> bool {
    forall|lane: int| 0 <= lane < gates.len() ==>
        candidate.input_at(lane, if left { (#[trigger] gates[lane]).0 } else { gates[lane].1 })
}

spec fn side_can_group(gates: Seq<(Pin, Pin)>, left: bool) -> bool {
    let first = if left { gates[0].0 } else { gates[0].1 };
    match first {
        Pin::Constant(value) => side_matches(gates, left, WordInputs::Constant(value)),
        Pin::Slot(slot) => side_matches(gates, left, WordInputs::Repeated(slot))
            || side_matches(gates, left, WordInputs::Consecutive(slot)),
    }
}

fn check_side(gates: &[(Pin, Pin)], left: bool, candidate: WordInputs) -> (matches: bool)
    ensures matches == side_matches(gates@, left, candidate),
{
    let mut lane = 0;
    while lane < gates.len()
        invariant
            lane <= gates@.len(),
            forall|i: int| 0 <= i < lane ==>
                candidate.input_at(i, if left { (#[trigger] gates@[i]).0 } else { gates@[i].1 }),
        decreases gates.len() - lane,
    {
        let gate = gates[lane];
        if !input_matches(candidate, lane, input(gate, left)) {
            assert(!side_matches(gates@, left, candidate));
            return false;
        }
        lane += 1;
    }
    true
}

fn classify_side(gates: &[(Pin, Pin)], left: bool) -> (result: Option<WordInputs>)
    requires 1 <= gates@.len() <= 64,
    ensures
        result.is_some() == side_can_group(gates@, left),
        result matches Some(candidate) ==> side_matches(gates@, left, candidate),
{
    match input(gates[0], left) {
        Pin::Constant(value) => {
            let candidate = WordInputs::Constant(value);
            if check_side(gates, left, candidate) { Some(candidate) } else { None }
        },
        Pin::Slot(first) => {
            let repeated = WordInputs::Repeated(first);
            if check_side(gates, left, repeated) { return Some(repeated); }
            let consecutive = WordInputs::Consecutive(first);
            if check_side(gates, left, consecutive) { Some(consecutive) } else { None }
        },
    }
}

/// 両側の入力がワードで表せる形のゲート群を定める。
pub closed spec fn nor_group_available(gates: Seq<(Pin, Pin)>) -> bool {
    1 <= gates.len() <= 64 && side_can_group(gates, true) && side_can_group(gates, false)
}

/// 両側の入力を連続区間、同じ配線または定数で表せるゲート群を照合する。
///
/// 空、65ゲート以上、または対応できない配線の場合は `None` を返す。
pub fn classify_nor_group(gates: &[(Pin, Pin)]) -> (result: Option<NorWordGroup>)
    ensures
        result.is_some() == nor_group_available(gates@),
        result matches Some(group) ==> group.matches(gates@),
{
    if gates.is_empty() || gates.len() > 64 {
        return None;
    }
    let left = classify_side(gates, true)?;
    let right = classify_side(gates, false)?;
    let group = NorWordGroup { width: gates.len(), left, right };
    assert(group.matches(gates@));
    Some(group)
}

/// ゲートに対応するビットを読む。
pub open spec fn word_bit(value: u64, lane: u32) -> bool {
    ((value >> lane) & 1u64) != 0
}

/// ビットごとに異なる NOR を、更新前の2つの入力ワードから評価する。
pub fn evaluate_nor_word(left: u64, right: u64) -> (result: u64)
    ensures forall|lane: u32| lane < 64 ==>
        #[trigger] word_bit(result, lane) == !(word_bit(left, lane) || word_bit(right, lane)),
{
    let result = !(left | right);
    proof {
        assert forall|lane: u32| lane < 64 implies
            #[trigger] word_bit(result, lane) == !(word_bit(left, lane) || word_bit(right, lane)) by {
            assert(word_bit(!(left | right), lane) == !(word_bit(left, lane) || word_bit(right, lane)))
                by (bit_vector)
                requires lane < 64;
        }
    }
    result
}

/// 再評価するビットだけを更新し、値が変わったビットの集合を返す。
///
/// `left`、`right`、`previous` は同じ tick の更新前の値を渡す。
/// ワード末尾にゲートがない場合は、そのビットを `pending` に含めない。
pub fn update_nor_word(left: u64, right: u64, previous: u64, pending: u64)
    -> (result: (u64, u64))
    ensures
        forall|lane: u32| lane < 64 ==> #[trigger] word_bit(result.0, lane) == (
            if word_bit(pending, lane) {
                !(word_bit(left, lane) || word_bit(right, lane))
            } else {
                word_bit(previous, lane)
            }
        ),
        result.1 == result.0 ^ previous,
        result.1 & !pending == 0,
{
    let value = evaluate_nor_word(left, right);
    let changed = (value ^ previous) & pending;
    let next = previous ^ changed;
    proof {
        assert forall|lane: u32| lane < 64 implies #[trigger] word_bit(next, lane) == (
            if word_bit(pending, lane) {
                !(word_bit(left, lane) || word_bit(right, lane))
            } else {
                word_bit(previous, lane)
            }
        ) by {
            assert(word_bit(value, lane) == !(word_bit(left, lane) || word_bit(right, lane)));
            assert({
                let changed = (value ^ previous) & pending;
                let expected = if word_bit(pending, lane) {
                    word_bit(value, lane)
                } else {
                    word_bit(previous, lane)
                };
                word_bit(previous ^ changed, lane) == expected
            }) by (bit_vector)
                requires lane < 64;
        }
        assert(changed == next ^ previous) by (bit_vector)
            requires next == previous ^ changed;
        assert(changed & !pending == 0) by (bit_vector)
            requires changed == (value ^ previous) & pending;
    }
    (next, changed)
}

/// 配線番号をワード番号とワード内のビット位置に分けて読む。
pub open spec fn wire_bit(words: Seq<u64>, slot: int) -> bool {
    word_bit(words[slot / 64], (slot % 64) as u32)
}

impl WordInputs {
    /// 同じ tick に参照する値を、元のゲート順で取り出す。
    pub open spec fn value_at(self, words: Seq<u64>, lane: int) -> bool {
        match self {
            WordInputs::Consecutive(first) => wire_bit(words, first + lane),
            WordInputs::Repeated(slot) => wire_bit(words, slot as int),
            WordInputs::Constant(value) => value,
        }
    }

    /// 最終ワードの余白を除いた範囲に入力配線が収まることを表す。
    pub open spec fn fits(self, width: int, wire_count: int) -> bool {
        match self {
            WordInputs::Consecutive(first) => first + width <= wire_count,
            WordInputs::Repeated(first) => first < wire_count,
            WordInputs::Constant(_) => true,
        }
    }
}

fn inputs_fit(inputs: WordInputs, width: usize, wire_count: usize) -> (result: bool)
    ensures result == inputs.fits(width as int, wire_count as int),
{
    match inputs {
        WordInputs::Consecutive(first) => first <= wire_count && width <= wire_count - first,
        WordInputs::Repeated(first) => first < wire_count,
        WordInputs::Constant(_) => true,
    }
}

fn join_window(low: u64, high: u64, shift: u32) -> (word: u64)
    requires shift < 64,
    ensures forall|lane: u32| lane < 64 ==>
        #[trigger] word_bit(word, lane) == if lane + shift < 64 {
            word_bit(low, (lane + shift) as u32)
        } else {
            word_bit(high, (lane + shift - 64) as u32)
        },
{
    if shift == 0 { return low; }
    let word = (low >> shift) | (high << (64 - shift));
    proof {
        assert forall|lane: u32| lane < 64 implies
            #[trigger] word_bit(word, lane) == if lane + shift < 64 {
                word_bit(low, (lane + shift) as u32)
            } else {
                word_bit(high, (lane + shift - 64) as u32)
            } by {
            let sum = (lane + shift) as u32;
            if sum < 64 {
                assert(word_bit((low >> shift) | (high << ((64 - shift) as u32)), lane)
                    == word_bit(low, sum)) by (bit_vector)
                    requires 0 < shift < 64, lane < 64, sum == (lane + shift) as u32, sum < 64;
            } else {
                assert(word_bit((low >> shift) | (high << ((64 - shift) as u32)), lane)
                    == word_bit(high, (sum - 64) as u32)) by (bit_vector)
                    requires 0 < shift < 64, lane < 64, sum == (lane + shift) as u32, 64 <= sum < 127;
            }
        }
    }
    word
}

fn fill_word(value: bool) -> (word: u64)
    ensures forall|lane: u32| lane < 64 ==> #[trigger] word_bit(word, lane) == value,
{
    let word = if value { u64::MAX } else { 0 };
    proof {
        assert forall|lane: u32| lane < 64 implies #[trigger] word_bit(word, lane) == value by {
            assert(word_bit(if value { u64::MAX } else { 0u64 }, lane) == value) by (bit_vector)
                requires lane < 64;
        }
    }
    word
}

fn read_group_inputs(inputs: WordInputs, words: &[u64], width: usize, _wire_count: usize) -> (value: u64)
    requires
        1 <= width <= 64,
        _wire_count <= 64 * words@.len(),
        inputs.fits(width as int, _wire_count as int),
    ensures forall|lane: u32| lane < width ==>
        #[trigger] word_bit(value, lane) == inputs.value_at(words@, lane as int),
{
    match inputs {
        WordInputs::Constant(value) => fill_word(value),
        WordInputs::Repeated(slot) => {
            let word = words[slot / 64];
            let value = ((word >> ((slot % 64) as u32)) & 1u64) != 0;
            fill_word(value)
        },
        WordInputs::Consecutive(first) => {
            let block = first / 64;
            let shift = (first % 64) as u32;
            let low = words[block];
            let high = if width > 64 - shift as usize {
                assert(first as int + width > 64 * (block as int + 1));
                words[block + 1]
            } else {
                0
            };
            let value = join_window(low, high, shift);
            proof {
                assert(first == 64 * block + shift);
                assert forall|lane: u32| lane < width implies
                    #[trigger] word_bit(value, lane) == inputs.value_at(words@, lane as int) by {
                    let pos = first as int + lane as int;
                    if lane + shift < 64 {
                        assert(pos / 64 == block);
                        assert(pos % 64 == lane + shift);
                    } else {
                        assert(pos / 64 == block + 1);
                        assert(pos % 64 == lane + shift - 64);
                    }
                }
            }
            value
        },
    }
}

/// 1回路の更新前の配線を読み、複数の NOR を同じ tick 内で評価する。
///
/// 配線のワードが足りない場合、入力が実際の配線幅に収まらない場合や
/// ゲート数が範囲外の場合は `None` を返す。
/// 結果のうちゲート数を越える上位ビットは使用しない。
pub fn evaluate_nor_group(group: NorWordGroup, words: &[u64], wire_count: usize) -> (result: Option<u64>)
    ensures
        result.is_some() <==> 1 <= group.width <= 64 && wire_count <= 64 * words@.len()
            && group.left.fits(group.width as int, wire_count as int)
            && group.right.fits(group.width as int, wire_count as int),
        result matches Some(value) ==> forall|lane: u32| lane < group.width ==>
            #[trigger] word_bit(value, lane)
                == !(group.left.value_at(words@, lane as int) || group.right.value_at(words@, lane as int)),
{
    let needed = wire_count / 64 + if wire_count % 64 == 0 { 0 } else { 1 };
    if group.width == 0 || group.width > 64 || needed > words.len()
        || !inputs_fit(group.left, group.width, wire_count)
        || !inputs_fit(group.right, group.width, wire_count)
    {
        return None;
    }
    let left = read_group_inputs(group.left, words, group.width, wire_count);
    let right = read_group_inputs(group.right, words, group.width, wire_count);
    let result = evaluate_nor_word(left, right);
    proof {
        assert forall|lane: u32| lane < group.width implies #[trigger] word_bit(result, lane)
            == !(group.left.value_at(words@, lane as int) || group.right.value_at(words@, lane as int)) by {
            assert(word_bit(left, lane) == group.left.value_at(words@, lane as int));
            assert(word_bit(right, lane) == group.right.value_at(words@, lane as int));
        }
    }
    Some(result)
}

} // verus!
