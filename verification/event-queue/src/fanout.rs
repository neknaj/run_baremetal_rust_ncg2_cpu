//! 正規化した配線を読む素子と公開出力を、配線の番号から引けるようにする。

use alloc::vec::Vec;
use vstd::prelude::*;
#[cfg(verus_only)]
use vstd::seq_lib::lemma_seq_contains_after_push;

verus! {

/// 増補入力または NOR の登録出力を指す番号と、値が固定された配線を区別する。
#[derive(Clone, Copy)]
pub enum Pin {
    /// 変更を通知できる配線の番号。
    Slot(usize),
    /// 実行中に値が変わらない配線。
    Constant(bool),
}

impl Pin {
    /// 正規化した配線の読み出し値を表す。
    pub open spec fn value_model(self, values: Seq<bool>) -> bool {
        match self {
            Self::Slot(slot) => values[slot as int],
            Self::Constant(bit) => bit,
        }
    }

    /// 同じ tick の参照値を得る。
    ///
    /// # Panics
    ///
    /// 指定した配線が値の列の範囲外ならパニックする。
    #[inline]
    pub fn read(self, values: &[bool]) -> (value: bool)
        requires self matches Self::Slot(slot) ==> slot < values@.len(),
        ensures value == self.value_model(values@),
    {
        match self {
            Self::Slot(slot) => values[slot],
            Self::Constant(bit) => bit,
        }
    }
}

/// 評価時に状態素子が読む配線。ラッチのアドレス列は空である。
pub struct StoragePins {
    /// RAM のアドレス。最下位ビットが先頭。
    pub address: Vec<Pin>,
    /// データ。最下位ビットが先頭。
    pub data: Vec<Pin>,
    /// ラッチの保持または RAM の書き込みの許可。
    pub enable: Pin,
}

/// 指定の配線を直接読む素子と公開出力を保持する。
pub struct ReverseDependencies {
    nodes: Vec<Vec<usize>>,
    outputs: Vec<Vec<usize>>,
}

/// 配線の番号を読むかを定める。定数には変更通知の送り元がない。
pub open spec fn reads_slot(pin: Pin, slot: usize) -> bool {
    pin == Pin::Slot(slot)
}

/// NOR のいずれかの入力か、状態素子のいずれかの端子が指定の配線を読む。
pub open spec fn node_reads(gates: Seq<(Pin, Pin)>, storage: Seq<&StoragePins>, target: usize, slot: usize) -> bool {
    if target < gates.len() {
        let gate = gates[target as int];
        reads_slot(gate.0, slot) || reads_slot(gate.1, slot)
    } else {
        let index = target - gates.len();
        &&& index < storage.len()
        &&& {
            let device = storage[index];
            reads_slot(device.enable, slot) || device.data@.contains(Pin::Slot(slot))
                || device.address@.contains(Pin::Slot(slot))
        }
    }
}

/// 公開出力のビットが指定の配線を読む。
pub open spec fn output_reads(outputs: Seq<Pin>, target: usize, slot: usize) -> bool {
    target < outputs.len() && reads_slot(outputs[target as int], slot)
}

/// 逆引き表の一行に指定した対象が入っている。
pub open spec fn has_user(rows: Seq<Vec<usize>>, slot: usize, target: usize) -> bool {
    slot < rows.len() && rows[slot as int]@.contains(target)
}

spec fn ordered(row: Seq<usize>) -> bool {
    forall|i: int, j: int| 0 <= i < j < row.len() ==> row[i] < row[j]
}

spec fn ordered_through(rows: Seq<Vec<usize>>, target: usize) -> bool {
    forall|i: int| 0 <= i < rows.len() ==> {
        let row = (#[trigger] rows[i])@;
        ordered(row) && (row.len() > 0 ==> row.last() <= target)
    }
}

spec fn added(
    before: Seq<Vec<usize>>, after: Seq<Vec<usize>>, target: usize, readers: spec_fn(usize) -> bool,
) -> bool {
    &&& before.len() == after.len()
    &&& forall|slot: usize, user: usize| slot < before.len() ==> #[trigger] has_user(after, slot, user)
        == (has_user(before, slot, user) || user == target && readers(slot))
}

fn empty_rows(slots: usize) -> (rows: Vec<Vec<usize>>)
    ensures rows@.len() == slots, forall|i: int| 0 <= i < slots ==> (#[trigger] rows@[i])@.len() == 0,
{
    let mut rows: Vec<Vec<usize>> = Vec::with_capacity(slots);
    let mut i = 0;
    while i < slots
        invariant i <= slots, rows@.len() == i,
            forall|j: int| 0 <= j < i ==> (#[trigger] rows@[j])@.len() == 0,
        decreases slots - i,
    {
        rows.push(Vec::new());
        i += 1;
    }
    rows
}

fn connect_pin(rows: &mut Vec<Vec<usize>>, target: usize, pin: Pin) -> (result: Result<(), &'static str>)
    requires ordered_through(old(rows)@, target),
    ensures result.is_ok() ==> ordered_through(final(rows)@, target)
        && added(old(rows)@, final(rows)@, target, |slot: usize| reads_slot(pin, slot)),
{
    let Pin::Slot(slot) = pin else { return Ok(()); };
    if slot >= rows.len() {
        return Err("配線の参照先が逆引き表の範囲外。");
    }
    let ghost before = rows@;
    let ghost row_before = rows@[slot as int]@;
    let row = &mut rows[slot];
    let last = row.len();
    if last > 0 && row[last - 1] == target {
        proof {
            assert(row_before.contains(target));
            assert forall|s: usize, user: usize| s < before.len() implies
                has_user(before, s, user) == (has_user(before, s, user) || user == target && reads_slot(pin, s)) by {
                if s == slot && user == target {
                    assert(has_user(before, slot, target));
                }
            };
        }
        return Ok(());
    }
    proof {
        assert(ordered(row_before));
        assert forall|j: int| 0 <= j < row_before.len() implies (#[trigger] row_before[j]) < target by {
            if j + 1 < row_before.len() {
                assert(row_before[j] < row_before.last());
            } else {
                assert(row_before[j] == row_before.last());
            }
        };
    }
    row.push(target);
    proof {
        let now = rows@;
        let next = row_before.push(target);
        assert(ordered(next)) by {
            assert forall|i: int, j: int| 0 <= i < j < next.len() implies next[i] < next[j] by {
                if j < row_before.len() {
                    assert(next[i] == row_before[i] && next[j] == row_before[j]);
                } else {
                    assert(next[i] == row_before[i] && next[j] == target);
                }
            };
        };
        assert forall|s: int| 0 <= s < now.len() implies {
            let row = (#[trigger] now[s])@;
            ordered(row) && (row.len() > 0 ==> row.last() <= target)
        } by {
            if s != slot { assert(now[s] == before[s]); }
        };
        assert forall|s: usize, user: usize| s < before.len() implies #[trigger] has_user(now, s, user)
            == (has_user(before, s, user) || user == target && reads_slot(pin, s)) by {
            if s == slot {
                lemma_seq_contains_after_push(row_before, target, user);
            } else {
                assert(now[s as int] == before[s as int]);
            }
        };
    }
    Ok(())
}

proof fn chain_added(
    start: Seq<Vec<usize>>, middle: Seq<Vec<usize>>, end: Seq<Vec<usize>>, target: usize,
    first: spec_fn(usize) -> bool, second: spec_fn(usize) -> bool, any: spec_fn(usize) -> bool,
)
    requires added(start, middle, target, first), added(middle, end, target, second),
        forall|slot: usize| slot < start.len() ==> #[trigger] any(slot) == (first(slot) || second(slot)),
    ensures added(start, end, target, any),
{
    assert forall|slot: usize, user: usize| slot < start.len() implies #[trigger] has_user(end, slot, user)
        == (has_user(start, slot, user) || user == target && any(slot)) by {
        assert(has_user(middle, slot, user)
            == (has_user(start, slot, user) || user == target && first(slot)));
    };
}

fn connect_list(rows: &mut Vec<Vec<usize>>, target: usize, pins: &[Pin]) -> (result: Result<(), &'static str>)
    requires ordered_through(old(rows)@, target),
    ensures result.is_ok() ==> ordered_through(final(rows)@, target)
        && added(old(rows)@, final(rows)@, target, |s: usize| pins@.contains(Pin::Slot(s))),
{
    let mut i = 0;
    let ghost start = rows@;
    while i < pins.len()
        invariant i <= pins@.len(), start == old(rows)@,
            ordered_through(rows@, target),
            added(start, rows@, target, |s: usize| pins@.take(i as int).contains(Pin::Slot(s))),
        decreases pins@.len() - i,
    {
        let ghost previous = rows@;
        let pin = pins[i];
        connect_pin(rows, target, pin)?;
        proof {
            let prefix = pins@.take(i as int);
            let next = pins@.take(i + 1);
            assert(next =~= prefix.push(pin));
            assert forall|s: usize| s < start.len() implies #[trigger] next.contains(Pin::Slot(s))
                == (prefix.contains(Pin::Slot(s)) || reads_slot(pin, s)) by {
                lemma_seq_contains_after_push(prefix, pin, Pin::Slot(s));
            };
            chain_added(start, previous, rows@, target,
                |s: usize| prefix.contains(Pin::Slot(s)), |s: usize| reads_slot(pin, s),
                |s: usize| next.contains(Pin::Slot(s)));
        }
        i += 1;
    }
    assert(pins@.take(i as int) =~= pins@);
    Ok(())
}

fn connect_node(
    rows: &mut Vec<Vec<usize>>, gates: &[(Pin, Pin)], storage: &[&StoragePins], target: usize,
) -> (result: Result<(), &'static str>)
    requires ordered_through(old(rows)@, target), target < gates@.len() + storage@.len(),
    ensures result.is_ok() ==> ordered_through(final(rows)@, target)
        && added(old(rows)@, final(rows)@, target, |s: usize| node_reads(gates@, storage@, target, s)),
{
    let ghost start = rows@;
    if target < gates.len() {
        let (left, right) = gates[target];
        connect_pin(rows, target, left)?;
        let ghost middle = rows@;
        connect_pin(rows, target, right)?;
        proof {
            chain_added(start, middle, rows@, target, |s: usize| reads_slot(left, s),
                |s: usize| reads_slot(right, s), |s: usize| node_reads(gates@, storage@, target, s));
        }
    } else {
        let device = storage[target - gates.len()];
        let enable = device.enable;
        connect_pin(rows, target, enable)?;
        let ghost enabled = rows@;
        connect_list(rows, target, device.data.as_slice())?;
        let ghost with_data = rows@;
        proof {
            chain_added(start, enabled, with_data, target, |s: usize| reads_slot(enable, s),
                |s: usize| device.data@.contains(Pin::Slot(s)),
                |s: usize| reads_slot(enable, s) || device.data@.contains(Pin::Slot(s)));
        }
        connect_list(rows, target, device.address.as_slice())?;
        proof {
            chain_added(start, with_data, rows@, target,
                |s: usize| reads_slot(enable, s) || device.data@.contains(Pin::Slot(s)),
                |s: usize| device.address@.contains(Pin::Slot(s)),
                |s: usize| node_reads(gates@, storage@, target, s));
        }
    }
    Ok(())
}

spec fn prefix(rows: Seq<Vec<usize>>, end: usize, reads: spec_fn(usize, usize) -> bool) -> bool {
    forall|s: usize, t: usize| s < rows.len() ==> #[trigger] has_user(rows, s, t) == (t < end && reads(t, s))
}

proof fn advance_prefix(
    before: Seq<Vec<usize>>, after: Seq<Vec<usize>>, target: usize, reads: spec_fn(usize, usize) -> bool,
    added_reads: spec_fn(usize) -> bool,
)
    requires target < usize::MAX, prefix(before, target, reads),
        added(before, after, target, added_reads),
        forall|s: usize| s < before.len() ==> #[trigger] added_reads(s) == reads(target, s),
    ensures prefix(after, (target + 1) as usize, reads),
{
    assert forall|s: usize, t: usize| s < after.len() implies #[trigger] has_user(after, s, t)
        == (t < target + 1 && reads(t, s)) by {
        assert(has_user(before, s, t) == (t < target && reads(t, s)));
    };
}

/// 正規化した全配線の逆引き表に、参照する相手が過不足なく一度ずつ現れる。
pub open spec fn matches_wiring(
    index: &ReverseDependencies, slots: usize, gates: Seq<(Pin, Pin)>, storage: Seq<&StoragePins>, outputs: Seq<Pin>,
) -> bool {
    let nodes = index.nodes_model();
    let outs = index.outputs_model();
    &&& nodes.len() == outs.len() == slots
    &&& forall|i: int| 0 <= i < slots ==>
        (#[trigger] nodes[i])@.no_duplicates() && (#[trigger] outs[i])@.no_duplicates()
    &&& forall|s: usize, t: usize| s < slots ==> #[trigger] has_user(nodes, s, t) == node_reads(gates, storage, t, s)
    &&& forall|s: usize, t: usize| s < slots ==> #[trigger] has_user(outs, s, t) == output_reads(outputs, t, s)
}

impl ReverseDependencies {
    /// 配線別の素子番号の列を表す。
    pub closed spec fn nodes_model(&self) -> Seq<Vec<usize>> { self.nodes@ }

    /// 配線別の公開出力ビット番号の列を表す。
    pub closed spec fn outputs_model(&self) -> Seq<Vec<usize>> { self.outputs@ }

    /// NOR、状態素子、公開出力を順に走査し、それぞれの入力配線に依存先を登録する。
    ///
    /// # Errors
    ///
    /// 素子番号を表せない場合、または参照先の配線番号が `slots` 以上の場合に返す。
    pub fn build(
        slots: usize, gates: &[(Pin, Pin)], storage: &[&StoragePins], outputs: &[Pin],
    ) -> (result: Result<Self, &'static str>)
        ensures result matches Ok(index) ==> matches_wiring(&index, slots, gates@, storage@, outputs@),
    {
        let Some(count) = gates.len().checked_add(storage.len()) else {
            return Err("素子の添字が範囲外。");
        };
        let mut nodes = empty_rows(slots);
        let ghost read_node = |t: usize, s: usize| node_reads(gates@, storage@, t, s);
        let mut i = 0;
        while i < count
            invariant i <= count == gates@.len() + storage@.len(), nodes@.len() == slots,
                ordered_through(nodes@, i), prefix(nodes@, i, read_node),
                forall|t: usize, s: usize| #[trigger] read_node(t, s) == node_reads(gates@, storage@, t, s),
            decreases count - i,
        {
            let ghost before = nodes@;
            connect_node(&mut nodes, gates, storage, i)?;
            proof {
                advance_prefix(before, nodes@, i, read_node, |s: usize| node_reads(gates@, storage@, i, s));
            }
            i += 1;
        }

        let mut out = empty_rows(slots);
        let ghost read_output = |t: usize, s: usize| output_reads(outputs@, t, s);
        let mut j = 0;
        while j < outputs.len()
            invariant j <= outputs@.len(), out@.len() == slots,
                ordered_through(out@, j), prefix(out@, j, read_output),
                forall|t: usize, s: usize| #[trigger] read_output(t, s) == output_reads(outputs@, t, s),
            decreases outputs@.len() - j,
        {
            let ghost before = out@;
            connect_pin(&mut out, j, outputs[j])?;
            proof {
                let pin = outputs@[j as int];
                advance_prefix(before, out@, j, read_output, |s: usize| reads_slot(pin, s));
            }
            j += 1;
        }

        proof {
            assert forall|s: int| 0 <= s < slots implies
                (#[trigger] nodes@[s])@.no_duplicates() && (#[trigger] out@[s])@.no_duplicates() by {
                let node_row = nodes@[s]@;
                let output_row = out@[s]@;
                assert(ordered(node_row) && ordered(output_row));
            };
        }
        Ok(Self { nodes, outputs: out })
    }

    /// 指定した配線の素子番号と公開出力番号を借りる。範囲外なら `None` を返す。
    #[inline]
    pub fn readers(&self, slot: usize) -> (result: Option<(&[usize], &[usize])>)
        ensures
            result.is_some() <==> slot < self.nodes_model().len() && slot < self.outputs_model().len(),
            result matches Some((nodes, outputs)) ==> nodes@ == self.nodes_model()[slot as int]@
                && outputs@ == self.outputs_model()[slot as int]@,
    {
        Some((self.nodes.get(slot)?.as_slice(), self.outputs.get(slot)?.as_slice()))
    }
}

}
