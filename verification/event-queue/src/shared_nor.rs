//! 共通のピンを読む NOR を、元のゲート番号との対応を保って1ワードにまとめる。

use super::fanout::Pin;
use alloc::vec::Vec;
use vstd::prelude::*;

verus! {

/// ワードの1ビットに対応する元のゲートと、共通ピンと対になる入力。
pub struct SharedNorLane {
    /// 元のネットリストでのゲート番号。
    pub gate: usize,
    /// 共通ピンと対になる入力。元の両入力が同じなら共通ピンと同じ値になる。
    pub other: Pin,
}

/// 共通ピンと元のゲートを1ビットずつ対応付けた NOR のグループ。
pub struct SharedNorGroup {
    shared: Pin,
    lanes: Vec<SharedNorLane>,
}

/// グループの構築時に最初に見つけた違反。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedNorGroupError {
    /// ゲート番号が空、または64件を超える。
    Width,
    /// 指定したゲート番号が存在しない。
    UnknownGate {
        /// 渡された順序での位置。
        lane: usize,
    },
    /// 既にグループに含めたゲート番号が再び現れた。
    DuplicateGate {
        /// 重複した番号が再び現れた位置。
        lane: usize,
        /// 同じ番号が最初に現れた位置。
        first: usize,
    },
    /// 指定したゲートに共通ピンがない。
    SharedPinMissing {
        /// 渡された順序での位置。
        lane: usize,
    },
}

/// 指定したゲートの、共通ピンと対になる入力。左右どちらに共通ピンがあってもよい。
pub open spec fn shared_other_input(gate: (Pin, Pin), shared: Pin) -> Option<Pin> {
    if gate.0 == shared {
        Some(gate.1)
    } else if gate.1 == shared {
        Some(gate.0)
    } else {
        None
    }
}

/// グループに指定した位置で、元ゲートと共通ピンが対応し、ゲート番号が初出であることを表す。
pub open spec fn shared_group_member(
    gates: Seq<(Pin, Pin)>, ids: Seq<usize>, shared: Pin, lane: int,
) -> bool {
    let gate = ids[lane];
    &&& 0 <= lane < ids.len()
    &&& gate < gates.len()
    &&& forall|i: int| 0 <= i < lane ==> #[trigger] ids[i] != gate
    &&& shared_other_input(gates[gate as int], shared).is_some()
}

/// グループに指定した、先頭から `count` 件が妥当であることを表す。
pub open spec fn shared_group_prefix(
    gates: Seq<(Pin, Pin)>, ids: Seq<usize>, shared: Pin, count: int,
) -> bool {
    &&& 0 <= count <= ids.len()
    &&& forall|lane: int| 0 <= lane < count ==> #[trigger] shared_group_member(gates, ids, shared, lane)
}

/// 件数を先に検査し、続いて指定順で最初の違反を決める。
pub open spec fn shared_group_error(
    gates: Seq<(Pin, Pin)>, ids: Seq<usize>, shared: Pin, error: SharedNorGroupError,
) -> bool {
    if !(1 <= ids.len() <= 64) {
        error == SharedNorGroupError::Width
    } else {
        match error {
            SharedNorGroupError::Width => false,
            SharedNorGroupError::UnknownGate { lane } =>
                shared_group_prefix(gates, ids, shared, lane as int)
                    && lane < ids.len() && ids[lane as int] >= gates.len(),
            SharedNorGroupError::DuplicateGate { lane, first } =>
                shared_group_prefix(gates, ids, shared, lane as int)
                    && first < lane < ids.len() && ids[lane as int] < gates.len()
                    && ids[first as int] == ids[lane as int],
            SharedNorGroupError::SharedPinMissing { lane } =>
                shared_group_prefix(gates, ids, shared, lane as int)
                    && lane < ids.len() && ids[lane as int] < gates.len()
                    && (forall|i: int| 0 <= i < lane ==> #[trigger] ids[i] != ids[lane as int])
                    && shared_other_input(gates[ids[lane as int] as int], shared).is_none(),
        }
    }
}

proof fn invalid_member_rejects_group(gates: Seq<(Pin, Pin)>, ids: Seq<usize>, shared: Pin, lane: int)
    requires 0 <= lane < ids.len(), !shared_group_member(gates, ids, shared, lane),
    ensures !shared_group_prefix(gates, ids, shared, ids.len() as int),
{
    if shared_group_prefix(gates, ids, shared, ids.len() as int) {
        assert(shared_group_member(gates, ids, shared, lane));
    }
}

fn pins_equal(left: Pin, right: Pin) -> (result: bool)
    ensures result == (left == right),
{
    match (left, right) {
        (Pin::Slot(a), Pin::Slot(b)) => a == b,
        (Pin::Constant(a), Pin::Constant(b)) => a == b,
        _ => false,
    }
}

fn other_input(gate: (Pin, Pin), shared: Pin) -> (result: Option<Pin>)
    ensures result == shared_other_input(gate, shared),
{
    if pins_equal(gate.0, shared) {
        Some(gate.1)
    } else if pins_equal(gate.1, shared) {
        Some(gate.0)
    } else {
        None
    }
}

fn first_occurrence(ids: &[usize], limit: usize, gate: usize) -> (found: Option<usize>)
    requires limit <= ids@.len(),
    ensures
        found matches Some(i) ==> i < limit && ids@[i as int] == gate,
        found.is_none() <==> (forall|i: int| 0 <= i < limit ==> #[trigger] ids@[i] != gate),
{
    let mut i = 0;
    while i < limit
        invariant i <= limit <= ids@.len(),
            forall|j: int| 0 <= j < i ==> #[trigger] ids@[j] != gate,
        decreases limit - i,
    {
        if ids[i] == gate { return Some(i); }
        i += 1;
    }
    None
}

/// 指定した番号のビットを表す。
pub open spec fn shared_word_bit(word: u64, lane: u32) -> bool {
    (word >> lane) & 1u64 != 0
}

/// 共通ピンと各 lane のもう一方のピンが、同じ更新前の配線値に対応することを表す。
pub open spec fn shared_operands_match(
    group: &SharedNorGroup, old_wires: Seq<bool>, common: bool, others: u64,
) -> bool {
    &&& common == group.shared_model().value_model(old_wires)
    &&& (!common ==> forall|lane: u32| lane < group.lanes_model().len() ==>
        #[trigger] shared_word_bit(others, lane)
            == group.lanes_model()[lane as int].other.value_model(old_wires))
}

/// 指定したビットと、その位置に割り当てた元のゲートの NOR が一致することを表す。
pub open spec fn shared_output_lane_matches(
    group: &SharedNorGroup, gates: Seq<(Pin, Pin)>, old_wires: Seq<bool>, output: u64, lane: u32,
) -> bool {
    &&& lane < group.lanes_model().len()
    &&& {
        let id = group.lanes_model()[lane as int].gate;
        &&& id < gates.len()
        &&& {
            let gate = gates[id as int];
            shared_word_bit(output, lane) == !(gate.0.value_model(old_wires) || gate.1.value_model(old_wires))
        }
    }
}

/// 各ビットと、その位置に割り当てた元のゲートの NOR が一致することを表す。
pub open spec fn shared_output_matches(
    group: &SharedNorGroup, gates: Seq<(Pin, Pin)>, old_wires: Seq<bool>, output: u64,
) -> bool {
    forall|lane: u32| lane < group.lanes_model().len() ==>
        #[trigger] shared_output_lane_matches(group, gates, old_wires, output, lane)
}

impl SharedNorGroup {
    /// 左右どちらかに共通する元のピン。
    pub closed spec fn shared_model(&self) -> Pin { self.shared }

    /// ワードの下位ビットから順に並ぶ、元のゲート番号ともう一方の入力。
    pub closed spec fn lanes_model(&self) -> Seq<SharedNorLane> { self.lanes@ }

    #[verifier::type_invariant]
    spec fn width_valid(&self) -> bool { 1 <= self.lanes@.len() <= 64 }

    /// ゲート番号の並びと、それぞれの元ゲートを読む両入力の対応を表す。
    pub open spec fn matches(&self, gates: Seq<(Pin, Pin)>, ids: Seq<usize>, shared: Pin) -> bool {
        &&& 1 <= ids.len() <= 64
        &&& shared_group_prefix(gates, ids, shared, ids.len() as int)
        &&& self.shared_model() == shared
        &&& self.lanes_model().len() == ids.len()
        &&& forall|lane: int| 0 <= lane < ids.len() ==> {
            let item = #[trigger] self.lanes_model()[lane];
            &&& item.gate == ids[lane] && item.gate < gates.len()
            &&& shared_other_input(gates[item.gate as int], shared) == Some(item.other)
        }
    }

    /// 元ゲートとの対応を検査し、指定したゲート番号をその順序で下位ビットから割り当てる。
    ///
    /// 同じ共通ピンを左側で読むゲートと右側で読むゲートを、一つのグループに含められる。
    ///
    /// # Errors
    ///
    /// 件数を検査してから、指定順に存在しない番号、重複、共通ピンの欠落を調べる。
    pub fn new(gates: &[(Pin, Pin)], ids: &[usize], shared: Pin)
        -> (result: Result<SharedNorGroup, SharedNorGroupError>)
        ensures
            result.is_ok() <==> (1 <= ids@.len() <= 64 && shared_group_prefix(gates@, ids@, shared, ids@.len() as int)),
            result matches Ok(group) ==> group.matches(gates@, ids@, shared),
            result matches Err(error) ==> shared_group_error(gates@, ids@, shared, error),
    {
        if ids.is_empty() || ids.len() > 64 { return Err(SharedNorGroupError::Width); }
        let mut lanes = Vec::with_capacity(ids.len());
        let mut i = 0;
        while i < ids.len()
            invariant
                i == lanes@.len() <= ids@.len(),
                1 <= ids@.len() <= 64,
                shared_group_prefix(gates@, ids@, shared, i as int),
                forall|lane: int| 0 <= lane < i ==> {
                    let item: SharedNorLane = #[trigger] lanes@[lane];
                    &&& item.gate == ids@[lane] && item.gate < gates@.len()
                    &&& shared_other_input(gates@[item.gate as int], shared) == Some(item.other)
                },
            decreases ids.len() - i,
        {
            let gate = ids[i];
            let Some(pins) = gates.get(gate) else {
                proof { invalid_member_rejects_group(gates@, ids@, shared, i as int); }
                return Err(SharedNorGroupError::UnknownGate { lane: i });
            };
            if let Some(first) = first_occurrence(ids, i, gate) {
                assert(ids@[first as int] == gate);
                proof { invalid_member_rejects_group(gates@, ids@, shared, i as int); }
                return Err(SharedNorGroupError::DuplicateGate { lane: i, first });
            }
            let Some(other) = other_input(*pins, shared) else {
                proof { invalid_member_rejects_group(gates@, ids@, shared, i as int); }
                return Err(SharedNorGroupError::SharedPinMissing { lane: i });
            };
            assert(shared_group_member(gates@, ids@, shared, i as int));
            let ghost before = lanes@;
            lanes.push(SharedNorLane { gate, other });
            proof {
                assert forall|j: int| 0 <= j < i + 1 implies
                    #[trigger] shared_group_member(gates@, ids@, shared, j) by {
                    if j == i { assert(shared_group_member(gates@, ids@, shared, i as int)); }
                };
                assert forall|j: int| 0 <= j < i + 1 implies {
                    let item = #[trigger] lanes@[j];
                    &&& item.gate == ids@[j] && item.gate < gates@.len()
                    &&& shared_other_input(gates@[item.gate as int], shared) == Some(item.other)
                } by {
                    if j < i { assert(lanes@[j] == before[j]); }
                };
            }
            i += 1;
        }
        let group = Self { shared, lanes };
        assert(group.matches(gates@, ids@, shared));
        Ok(group)
    }

    /// 元の共通ピンを返す。
    #[inline]
    pub fn shared(&self) -> (pin: Pin)
        ensures pin == self.shared_model(),
    {
        self.shared
    }

    /// 下位ビットから順に元ゲートの番号ともう一方の入力を借りる。
    #[inline]
    pub fn lanes(&self) -> (lanes: &[SharedNorLane])
        ensures lanes@ == self.lanes_model(),
    {
        self.lanes.as_slice()
    }

    /// 更新前の共通ピンと各ビットのもう一方の入力から NOR を求める。
    ///
    /// グループのゲート数を超える上位ビットは0にする。
    #[inline]
    pub fn evaluate(&self, common: bool, others: u64) -> (output: u64)
        ensures forall|lane: u32| lane < 64 ==>
            #[trigger] shared_word_bit(output, lane)
                == (lane < self.lanes_model().len() && !(common || shared_word_bit(others, lane))),
    {
        proof { use_type_invariant(self); }
        let width = self.lanes.len() as u32;
        let shift = 64u32 - width;
        let output = if common { 0 } else { !others & (u64::MAX >> shift) };
        proof {
            assert forall|lane: u32| lane < 64 implies
                #[trigger] shared_word_bit(output, lane)
                    == (lane < self.lanes_model().len() && !(common || shared_word_bit(others, lane))) by {
                assert(shared_word_bit(
                    if common { 0u64 } else { !others & (u64::MAX >> shift) }, lane)
                    == (lane < width && !(common || shared_word_bit(others, lane))))
                    by (bit_vector)
                    requires 0 < width <= 64, shift == 64 - width, lane < 64;
            };
        }
        output
    }

    /// 全ビットの結果が、同じ旧配線値で各元ゲートを個別に評価した値に一致する。
    pub proof fn evaluation_agrees(
        &self, gates: Seq<(Pin, Pin)>, ids: Seq<usize>, shared: Pin, old_wires: Seq<bool>,
        common: bool, others: u64, output: u64,
    )
        requires
            self.matches(gates, ids, shared), shared_operands_match(self, old_wires, common, others),
            forall|lane: u32| lane < self.lanes_model().len() ==>
                #[trigger] shared_word_bit(output, lane) == !(common || shared_word_bit(others, lane)),
        ensures shared_output_matches(self, gates, old_wires, output),
    {
        assert forall|lane: u32| lane < self.lanes_model().len() implies
            #[trigger] shared_output_lane_matches(self, gates, old_wires, output, lane) by {
            let item = self.lanes_model()[lane as int];
            assert(item.gate < gates.len());
            let gate = gates[item.gate as int];
            assert(shared_other_input(gate, shared) == Some(item.other));
            assert(shared_word_bit(output, lane) == !(common || shared_word_bit(others, lane)));
            if !common {
                assert(shared_word_bit(others, lane) == item.other.value_model(old_wires));
            }
        };
        assert(shared_output_matches(self, gates, old_wires, output));
    }
}

} // verus!
