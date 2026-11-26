//! 登録配線を確定し、依存する素子と公開出力に変化を伝える。

use vstd::prelude::*;
#[cfg(verus_only)]
use vstd::seq_lib::lemma_seq_contains_after_push;

use crate::{EvaluationQueue, fanout::ReverseDependencies, nor_inputs::PackedNorInputs};
#[cfg(verus_only)]
use crate::indexed;

verus! {

/// 一件の書き込みを適用する。範囲外の配線への指示は配線を変更しない。
pub open spec fn write_wire(wires: Seq<bool>, slot: usize, value: bool) -> Seq<bool> {
    if slot < wires.len() { wires.update(slot as int, value) } else { wires }
}

/// 候補の列を先頭から適用した後の登録配線を表す。
pub open spec fn apply_wire_updates(wires: Seq<bool>, changes: Seq<(usize, bool)>) -> Seq<bool>
    decreases changes.len(),
{
    if changes.len() == 0 {
        wires
    } else {
        let (slot, value) = changes.last();
        write_wire(apply_wire_updates(wires, changes.drop_last()), slot, value)
    }
}

/// 一つの配線への書き込みが同じ値を指定するとき、確定後にその値を読めることを示す。
pub proof fn assigned_wire_value(
    wires: Seq<bool>, changes: Seq<(usize, bool)>, slot: usize, value: bool,
)
    requires
        slot < wires.len(),
        forall|i: int| 0 <= i < changes.len() && (#[trigger] changes[i]).0 == slot ==> changes[i].1 == value,
        wires[slot as int] == value || changes.contains((slot, value)),
    ensures
        apply_wire_updates(wires, changes).len() == wires.len(),
        apply_wire_updates(wires, changes)[slot as int] == value,
    decreases changes.len(),
{
    if changes.len() == 0 {
        assert(!changes.contains((slot, value)));
    } else {
        let rest = changes.drop_last();
        let (last_slot, last_value) = changes.last();
        assert forall|i: int| 0 <= i < rest.len() && (#[trigger] rest[i]).0 == slot
            implies rest[i].1 == value by {
            assert(rest[i] == changes[i]);
        };
        if last_slot == slot {
            assert(changes[(changes.len() - 1) as int] == (last_slot, last_value));
            assert(last_value == value);
            wire_update_length(wires, rest);
        } else {
            assert(changes =~= rest.push((last_slot, last_value)));
            lemma_seq_contains_after_push(rest, (last_slot, last_value), (slot, value));
            assigned_wire_value(wires, rest, slot, value);
        }
    }
}

proof fn wire_update_length(wires: Seq<bool>, changes: Seq<(usize, bool)>)
    ensures apply_wire_updates(wires, changes).len() == wires.len(),
    decreases changes.len(),
{
    if changes.len() > 0 {
        wire_update_length(wires, changes.drop_last());
    }
}

/// 公開出力の逆引きが、登録配線と公開出力の幅に収まる条件を表す。
pub open spec fn readers_fit(users: &ReverseDependencies, wires: nat, outputs: nat) -> bool {
    &&& users.nodes_model().len() == users.outputs_model().len() == wires
    &&& forall|s: int, j: int| 0 <= s < wires && 0 <= j < users.outputs_model()[s]@.len()
        ==> #[trigger] users.outputs_model()[s]@[j] < outputs
}

/// 登録配線を一件確定し、その値が変わったときだけ依存先に通知する。
///
/// 逆引き表は配線の全域を覆い、公開出力の添字は出力列に収まることを呼び出し側が保証する。
///
/// # Panics
///
/// 変更された配線に逆引き行がない場合、またはその行の公開出力番号が範囲外の場合にパニックする。
#[inline]
pub fn publish_wire(
    wires: &mut PackedNorInputs, users: &ReverseDependencies, queue: &mut EvaluationQueue,
    output: &mut [bool], slot: usize, value: bool,
)
    requires
        readers_fit(users, old(wires).wire_count_model(), old(output)@.len()),
        indexed(old(queue).items_model(), old(queue).flags_model()),
    ensures
        final(wires).wires_model() == write_wire(old(wires).wires_model(), slot, value),
        final(wires).wire_count_model() == old(wires).wire_count_model(),
        indexed(final(queue).items_model(), final(queue).flags_model()),
        final(queue).flags_model().len() == old(queue).flags_model().len(),
        final(output)@.len() == old(output)@.len(),
        forall|node: usize| #[trigger] final(queue).scheduled(node) <==>
            old(queue).scheduled(node) || (
                slot < old(wires).wires_model().len() && old(wires).wires_model()[slot as int] != value
                    && node < old(queue).flags_model().len() && users.nodes_model()[slot as int]@.contains(node)
            ),
        forall|j: usize| j < old(output)@.len() ==> #[trigger] final(output)@[j as int] ==
            if slot < old(wires).wires_model().len() && old(wires).wires_model()[slot as int] != value
                && users.outputs_model()[slot as int]@.contains(j) { value } else { old(output)@[j as int] },
{
    if !wires.set(slot, value) {
        assert(wires.wires_model() =~= write_wire(old(wires).wires_model(), slot, value));
        return;
    }
    let (nodes, targets) = users.readers(slot).expect("配線の逆引き行が必要");
    queue.insert_all(nodes);
    let mut at = 0;
    while at < targets.len()
        invariant
            at <= targets@.len(),
            output@.len() == old(output)@.len(),
            slot < users.outputs_model().len(),
            targets@ == users.outputs_model()[slot as int]@,
            readers_fit(users, old(wires).wire_count_model(), old(output)@.len()),
            forall|j: usize| j < output@.len() ==> #[trigger] output@[j as int] ==
                if targets@.take(at as int).contains(j) { value } else { old(output)@[j as int] },
        decreases targets@.len() - at,
    {
        let target = targets[at];
        assert(users.outputs_model()[slot as int]@[at as int] == target);
        let ghost prior = output@;
        output[target] = value;
        proof {
            let prefix = targets@.take(at as int);
            assert(targets@.take(at + 1) =~= prefix.push(target));
            assert forall|j: usize| j < output@.len() implies #[trigger] output@[j as int] ==
                if targets@.take(at + 1).contains(j) { value } else { old(output)@[j as int] } by {
                lemma_seq_contains_after_push(prefix, target, j);
                if j != target {
                    assert(output@[j as int] == prior[j as int]);
                }
            };
        }
        at += 1;
    }
    assert(targets@.take(at as int) =~= targets@);
}

/// 評価を終えた候補を記録順に確定し、変更された配線の依存先に通知する。
///
/// 逆引き表と公開出力は [`publish_wire`] と同じ条件を満たす。
///
/// # Panics
///
/// 変更された配線の逆引き行や公開出力番号が範囲外ならパニックする。
#[inline]
pub fn publish_wire_updates(
    wires: &mut PackedNorInputs, users: &ReverseDependencies, queue: &mut EvaluationQueue,
    output: &mut [bool], changes: &[(usize, bool)],
)
    requires
        readers_fit(users, old(wires).wire_count_model(), old(output)@.len()),
        indexed(old(queue).items_model(), old(queue).flags_model()),
    ensures
        final(wires).wires_model() == apply_wire_updates(old(wires).wires_model(), changes@),
        final(wires).wire_count_model() == old(wires).wire_count_model(),
        indexed(final(queue).items_model(), final(queue).flags_model()),
        final(queue).flags_model().len() == old(queue).flags_model().len(),
        final(output)@.len() == old(output)@.len(),
{
    let mut at = 0;
    while at < changes.len()
        invariant
            at <= changes@.len(),
            wires.wires_model() == apply_wire_updates(old(wires).wires_model(), changes@.take(at as int)),
            wires.wire_count_model() == old(wires).wire_count_model(),
            indexed(queue.items_model(), queue.flags_model()),
            queue.flags_model().len() == old(queue).flags_model().len(),
            output@.len() == old(output)@.len(),
            readers_fit(users, old(wires).wire_count_model(), old(output)@.len()),
        decreases changes@.len() - at,
    {
        let (slot, value) = changes[at];
        publish_wire(wires, users, queue, output, slot, value);
        proof {
            let next = changes@.take(at + 1);
            assert(next.last() == (slot, value));
            assert(next.drop_last() =~= changes@.take(at as int));
        }
        at += 1;
    }
    assert(changes@.take(at as int) =~= changes@);
}

} // verus!
