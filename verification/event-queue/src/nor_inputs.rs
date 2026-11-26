//! 正規化済みの NOR 入力を、定数を含む値の列から直接読む。

use super::fanout::Pin;
use alloc::vec::Vec;
use vstd::prelude::*;

verus! {

/// 定数用の位置は配線の直後とし、他の配線の添字を保つ。
pub open spec fn packed_pin(pin: Pin, wire_count: nat) -> nat {
    match pin {
        Pin::Slot(slot) => slot as nat,
        Pin::Constant(bit) => wire_count + if bit { 1nat } else { 0nat },
    }
}

/// 指定した配線が定数用の位置から分かれていることを表す。
pub open spec fn pin_fits(pin: Pin, wire_count: nat) -> bool {
    pin matches Pin::Slot(slot) ==> slot < wire_count
}

fn pack_pin(pin: Pin, wire_count: usize) -> (result: Option<usize>)
    requires wire_count + 2 <= isize::MAX,
    ensures result.is_some() <==> pin_fits(pin, wire_count as nat),
        result matches Some(index) ==> index == packed_pin(pin, wire_count as nat) && index < wire_count + 2,
{
    match pin {
        Pin::Slot(slot) if slot < wire_count => Some(slot),
        Pin::Slot(_) => None,
        Pin::Constant(true) => Some(wire_count + 1),
        Pin::Constant(false) => Some(wire_count),
    }
}

/// 一つのゲートの左右の添字が、元の入力配線に対応することを表す。
pub closed spec fn pair_matches(gate: (Pin, Pin), pair: (usize, usize), wire_count: nat) -> bool {
    &&& pin_fits(gate.0, wire_count) && pin_fits(gate.1, wire_count)
    &&& pair.0 == packed_pin(gate.0, wire_count)
    &&& pair.1 == packed_pin(gate.1, wire_count)
}

/// 各 NOR の左右の入力添字と、それらが参照する配線値をまとめて保持する。
pub struct PackedNorInputs {
    pairs: Vec<(usize, usize)>,
    bits: Vec<bool>,
}

impl PackedNorInputs {
    /// ゲート順に左右の入力添字を表す。
    pub closed spec fn pairs_model(&self) -> Seq<(usize, usize)> { self.pairs@ }

    /// 定数領域を除く配線の総数を返す。
    pub closed spec fn wire_count_model(&self) -> nat { (self.bits@.len() - 2) as nat }

    /// 配線値の後ろに false と true があることを表す。
    pub closed spec fn has_constants(&self) -> bool {
        &&& self.bits@.len() >= 2
        &&& !self.bits@[self.bits@.len() - 2]
        &&& self.bits@[self.bits@.len() - 1]
    }

    #[verifier::type_invariant]
    spec fn valid(&self) -> bool {
        &&& self.has_constants()
        &&& forall|i: int| 0 <= i < self.pairs@.len() ==> {
            let pair = #[trigger] self.pairs@[i];
            pair.0 < self.bits@.len() && pair.1 < self.bits@.len()
        }
    }

    /// 逆引き表の構築にも使う元のゲート順と入力を保つことを示す。
    pub open spec fn matches_wiring(&self, gates: Seq<(Pin, Pin)>) -> bool {
        &&& self.has_constants()
        &&& self.pairs_model().len() == gates.len()
        &&& self.wire_count_model() + 2 <= isize::MAX
        &&& forall|i: int| 0 <= i < gates.len() ==>
            pair_matches(gates[i], #[trigger] self.pairs_model()[i], self.wire_count_model())
    }

    /// 正規化済みの入力を、同じ数・順序で直接読出し用の添字に変換する。
    ///
    /// 配線値を0にして、定数用の位置を確保する。定数用の位置を追加できないか、元の配線が範囲外なら `None` を返す。
    pub fn build(gates: &[(Pin, Pin)], wire_count: usize) -> (result: Option<Self>)
        ensures
            result.is_some() <==> wire_count + 2 <= isize::MAX
                && (forall|i: int| 0 <= i < gates@.len() ==>
                    pin_fits((#[trigger] gates@[i]).0, wire_count as nat)
                    && pin_fits(gates@[i].1, wire_count as nat)),
            result matches Some(packed) ==> packed.wire_count_model() == wire_count as nat
                && packed.matches_wiring(gates@)
                && packed.wires_model() == Seq::new(wire_count as nat, |_: int| false),
    {
        if wire_count > isize::MAX as usize - 2 {
            return None;
        }
        let mut pairs: Vec<(usize, usize)> = Vec::with_capacity(gates.len());
        let mut i = 0;
        while i < gates.len()
            invariant wire_count + 2 <= isize::MAX,
                i == pairs@.len() <= gates@.len(),
                forall|at: int| 0 <= at < i ==>
                    pair_matches(gates@[at], #[trigger] pairs@[at], wire_count as nat),
            decreases gates.len() - i,
        {
            let (left, right) = gates[i];
            let Some(left) = pack_pin(left, wire_count) else {
                assert(!pin_fits(gates@[i as int].0, wire_count as nat));
                return None;
            };
            let Some(right) = pack_pin(right, wire_count) else {
                assert(!pin_fits(gates@[i as int].1, wire_count as nat));
                return None;
            };
            let ghost before = pairs@;
            pairs.push((left, right));
            proof {
                assert forall|at: int| 0 <= at <= i implies
                    pair_matches(gates@[at], #[trigger] pairs@[at], wire_count as nat)
                by {
                    if at < i {
                        assert(pair_matches(gates@[at], before[at], wire_count as nat));
                    } else {
                        assert(pairs@[at] == (left, right));
                    }
                };
            }
            i += 1;
        }
        assert forall|at: int| 0 <= at < gates@.len() implies
            pin_fits((#[trigger] gates@[at]).0, wire_count as nat)
            && pin_fits(gates@[at].1, wire_count as nat) by {
            assert(pairs@[at].0 == packed_pin(gates@[at].0, wire_count as nat));
        }
        let mut bits: Vec<bool> = Vec::with_capacity(wire_count + 2);
        while bits.len() <= wire_count
            invariant bits@.len() <= wire_count + 1,
                forall|i: int| 0 <= i < bits@.len() ==> !#[trigger] bits@[i],
            decreases wire_count + 1 - bits@.len(),
        {
            bits.push(false);
        }
        bits.push(true);
        assert forall|at: int| 0 <= at < pairs@.len() implies {
            let pair = #[trigger] pairs@[at];
            pair.0 < bits@.len() && pair.1 < bits@.len()
        } by {
            assert(pair_matches(gates@[at], pairs@[at], wire_count as nat));
        };
        let packed = Self { pairs, bits };
        assert(packed.wires_model() =~= Seq::new(wire_count as nat, |_: int| false));
        Some(packed)
    }

    /// ゲートの数を返す。
    #[inline]
    pub fn gate_count(&self) -> (len: usize)
        ensures len == self.pairs_model().len(),
    {
        self.pairs.len()
    }

    /// 外部入力、状態素子、NOR の配線値を添字順に表す。
    pub closed spec fn wires_model(&self) -> Seq<bool> {
        self.bits@.take((self.bits@.len() - 2) as int)
    }

    /// 既存の配線値を0に戻す。定数の値と確保済みの領域を保つ。
    pub fn reset(&mut self)
        ensures final(self).pairs_model() == old(self).pairs_model(),
            final(self).wire_count_model() == old(self).wire_count_model(), final(self).has_constants(),
            final(self).wires_model() == Seq::new(old(self).wires_model().len(), |_: int| false),
    {
        proof { use_type_invariant(&*self); }
        let end = self.bits.len() - 2;
        let mut i = 0;
        while i < end
            invariant i <= end, end + 2 == self.bits@.len() == old(self).bits@.len(),
                self.pairs@ == old(self).pairs@, self.valid(), forall|at: int| 0 <= at < i ==> !#[trigger] self.bits@[at],
            decreases end - i,
        {
            self.bits[i] = false;
            i += 1;
        }
        assert(self.wires_model() =~= Seq::new(old(self).wires_model().len(), |_: int| false));
    }

    /// 指定の配線だけを更新し、変更したか返す。範囲外の配線では false を返す。
    #[inline]
    pub fn set(&mut self, slot: usize, value: bool) -> (changed: bool)
        ensures
            final(self).pairs_model() == old(self).pairs_model(),
            final(self).wire_count_model() == old(self).wire_count_model(), final(self).has_constants(),
            changed ==> slot < old(self).wire_count_model(),
            changed <==> slot < old(self).wires_model().len() && old(self).wires_model()[slot as int] != value,
            final(self).wires_model() == if changed {
                old(self).wires_model().update(slot as int, value)
            } else {
                old(self).wires_model()
            },
    {
        proof { use_type_invariant(&*self); }
        if slot >= self.bits.len() - 2 || self.bits[slot] == value {
            return false;
        }
        self.bits[slot] = value;
        assert(self.wires_model() =~= old(self).wires_model().update(slot as int, value));
        true
    }


    /// 入力列が、現在の配線値の同じ長さの先頭部分と一致するか調べる。
    ///
    /// 定数用の位置は配線値に含めない。入力列が配線数より長い場合は false を返す。
    #[inline]
    pub fn input_matches(&self, input: &[bool]) -> (same: bool)
        ensures same <==> input@.len() <= self.wire_count_model()
            && self.wires_model().take(input@.len() as int) == input@,
    {
        proof { use_type_invariant(self); }
        if input.len() > self.bits.len() - 2 {
            return false;
        }
        let mut same = true;
        let mut i = 0;
        while i < input.len()
            invariant i <= input@.len() <= self.wire_count_model(),
                self.wire_count_model() + 2 == self.bits@.len(),
                same <==> (forall|at: int| 0 <= at < i ==> self.bits@[at] == #[trigger] input@[at]),
            decreases input.len() - i,
        {
            // 全要素を比較し、差の有無を一つの値に集約する。
            let equal = self.bits[i] == input[i];
            proof {
                if same && equal {
                    assert forall|at: int| 0 <= at < i + 1
                        implies self.bits@[at] == #[trigger] input@[at] by {
                        if at == i { assert(self.bits@[at] == input@[at]); }
                    };
                } else if same {
                    assert(self.bits@[i as int] != input@[i as int]);
                }
            }
            same &= equal;
            i += 1;
        }
        assert(same <==> self.wires_model().take(input@.len() as int) =~= input@);
        same
    }

    /// 定数用の位置も含めた読み出し用の列を表す。
    pub closed spec fn slots_model(&self) -> Seq<bool> { self.bits@ }

    /// 状態素子の既存の読み出し処理に渡す配線値と定数を借りる。
    #[inline]
    pub fn read_only(&self) -> (bits: &[bool])
        ensures
            bits@ == self.slots_model(),
            self.wires_model() == bits@.take(self.wire_count_model() as int),
            self.wires_model().len() == self.wire_count_model(),
    {
        proof { use_type_invariant(self); }
        self.bits.as_slice()
    }

    /// 添字で得る NOR の次の登録値を表す。
    pub closed spec fn next_model(&self, gate: usize) -> bool {
        let pair = self.pairs_model()[gate as int];
        !(self.bits@[pair.0 as int] || self.bits@[pair.1 as int])
    }

    /// 元の入力配線を読む式と、変換後の読出しが一致することを示す。
    pub proof fn preserves_nor(
        &self, gate: usize, original: Seq<(Pin, Pin)>,
    )
        requires self.matches_wiring(original), gate < original.len(),
        ensures
            gate < self.pairs_model().len(),
            self.pairs_model()[gate as int].0 < self.slots_model().len(),
            self.pairs_model()[gate as int].1 < self.slots_model().len(),
            self.next_model(gate) == !(original[gate as int].0.value_model(self.wires_model())
                || original[gate as int].1.value_model(self.wires_model())),
    {
        let (left, right) = original[gate as int];
        let pair = self.pairs_model()[gate as int];
        assert(pair.0 == packed_pin(left, self.wire_count_model()));
        assert(pair.1 == packed_pin(right, self.wire_count_model()));
    }

    /// 値の列から指定ゲートの左右の入力を読み、NOR の結果を返す。
    ///
    /// # Panics
    ///
    /// ゲート番号が範囲外ならパニックする。
    #[inline]
    pub fn next(&self, gate: usize) -> (value: bool)
        requires gate < self.pairs_model().len(),
        ensures value == self.next_model(gate),
    {
        proof { use_type_invariant(self); }
        let (left, right) = self.pairs[gate];
        !(self.bits[left] || self.bits[right])
    }
}

}
