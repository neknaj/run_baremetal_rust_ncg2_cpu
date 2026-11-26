//! 疎実行で評価する素子を重複なく管理する。

#![no_std]
extern crate alloc;

pub mod fanout;
pub mod nor_inputs;
pub mod latch_lanes;
pub mod ram_address;
pub mod wire_updates;

use alloc::vec::Vec;
use vstd::prelude::*;
#[cfg(verus_only)]
use vstd::seq_lib::lemma_seq_contains_after_push;

verus! {

/// 固定範囲の素子を取り出すまで保持する。
///
/// 候補は一覧の末尾から取り出す。
pub struct EvaluationQueue {
    flags: Vec<bool>,
    items: Vec<usize>,
}

/// 候補の一覧とビット列は、同じ素子集合を重複なく表す。
pub open spec fn indexed(items: Seq<usize>, flags: Seq<bool>) -> bool {
    &&& flags.len() <= usize::MAX
    &&& items.no_duplicates()
    &&& forall|i: int| 0 <= i < items.len() ==> (#[trigger] items[i]) < flags.len()
    &&& forall|i: int| 0 <= i < flags.len() ==> ((#[trigger] flags[i]) <==> items.contains(i as usize))
}

proof fn appended(items: Seq<usize>, flags: Seq<bool>, node: usize)
    requires indexed(items, flags), node < flags.len(), !flags[node as int],
    ensures indexed(items.push(node), flags.update(node as int, true)),
{
    let next = items.push(node);
    assert(!items.contains(node));
    assert forall|i: int, j: int| 0 <= i < j < next.len() implies
        next[i] != next[j] by {
        if j < items.len() {
            assert(next[i] == items[i] && next[j] == items[j]);
        } else {
            assert(next[i] == items[i] && next[j] == node);
        }
    };
    assert forall|i: int| 0 <= i < next.len() implies (#[trigger] next[i]) < flags.len() by {
        if i < items.len() { assert(next[i] == items[i]); }
    };
    let marked = flags.update(node as int, true);
    assert forall|i: int| 0 <= i < marked.len() implies
        (#[trigger] marked[i]) == next.contains(i as usize) by {
        lemma_seq_contains_after_push(items, node, i as usize);
        assert(flags[i] == items.contains(i as usize));
    };
}

proof fn removed(items: Seq<usize>, flags: Seq<bool>, node: usize)
    requires indexed(items, flags), 0 < items.len(), node == items.last(),
    ensures
        node < flags.len(),
        indexed(items.drop_last(), flags.update(node as int, false)),
        forall|i: usize| i != node ==> items.drop_last().contains(i) == #[trigger] items.contains(i),
{
    assert(node == items[items.len() - 1]);
    let prefix = items.drop_last();
    assert(prefix.push(node) =~= items);
    if prefix.contains(node) {
        let at = choose|at: int| 0 <= at < prefix.len() && prefix[at] == node;
        assert(items[at] == node);
        assert(items[at] != items[items.len() - 1]);
    }
    assert forall|i: usize| i != node implies prefix.contains(i) == #[trigger] items.contains(i) by {
        lemma_seq_contains_after_push(prefix, node, i);
        assert(prefix.push(node) == items);
    };
    assert forall|i: int, j: int| 0 <= i < j < prefix.len() implies
        prefix[i] != prefix[j] by {
        assert(prefix[i] == items[i] && prefix[j] == items[j]);
    };
    assert forall|i: int| 0 <= i < prefix.len() implies (#[trigger] prefix[i]) < flags.len() by {
        assert(prefix[i] == items[i]);
    };
    let marked = flags.update(node as int, false);
    assert forall|i: int| 0 <= i < marked.len() implies
        (#[trigger] marked[i]) == prefix.contains(i as usize) by {
        if i != node {
            assert(i as usize != node);
            assert(items.contains(i as usize) == prefix.contains(i as usize));
        }
    };
}

fn seed(flags: &mut Vec<bool>, items: &mut Vec<usize>, count: usize)
    ensures
        final(flags)@ == Seq::new(count as nat, |_: int| true),
        final(items)@ == Seq::new(count as nat, |i: int| i as usize),
{
    flags.clear();
    items.clear();
    let mut i = 0;
    while i < count
        invariant i <= count,
            flags@ == Seq::new(i as nat, |_: int| true),
            items@ == Seq::new(i as nat, |j: int| j as usize),
        decreases count - i,
    {
        flags.push(true);
        items.push(i);
        i += 1;
        assert(flags@ =~= Seq::new(i as nat, |_: int| true));
        assert(items@ =~= Seq::new(i as nat, |j: int| j as usize));
    }
}

proof fn seed_indexed(items: Seq<usize>, flags: Seq<bool>, count: usize)
    requires items == Seq::new(count as nat, |i: int| i as usize),
        flags == Seq::new(count as nat, |_: int| true),
    ensures indexed(items, flags),
{
    assert forall|i: int| 0 <= i < flags.len() implies
        (#[trigger] flags[i]) == items.contains(i as usize) by {
        assert(items[i] == i as usize);
    };
}

impl EvaluationQueue {
    /// 候補の一覧を格納順に表す。
    pub closed spec fn items_model(&self) -> Seq<usize> { self.items@ }

    /// 素子番号ごとの登録の有無。列の長さが扱う素子数になる。
    pub closed spec fn flags_model(&self) -> Seq<bool> { self.flags@ }

    /// 指定した素子を取り出す予定があることを表す。
    pub open spec fn scheduled(&self, node: usize) -> bool {
        node < self.flags_model().len() && self.flags_model()[node as int]
    }

    /// 最初の tick に全素子が評価されるよう、全番号を登録する。
    pub fn new_all(count: usize) -> (queue: Self)
        ensures
            queue.flags_model() == Seq::new(count as nat, |_: int| true),
            queue.items_model() == Seq::new(count as nat, |i: int| i as usize),
            indexed(queue.items_model(), queue.flags_model()),
    {
        let mut flags = Vec::with_capacity(count);
        let mut items = Vec::with_capacity(count);
        seed(&mut flags, &mut items, count);
        proof { seed_indexed(items@, flags@, count); }
        Self { flags, items }
    }

    /// 候補を全素子に戻す。確保済みの領域を使う。
    pub fn reset_all(&mut self)
        ensures
            final(self).flags_model() == Seq::new(old(self).flags_model().len(), |_: int| true),
            final(self).items_model() == Seq::new(old(self).flags_model().len(), |i: int| i as usize),
            indexed(final(self).items_model(), final(self).flags_model()),
    {
        let count = self.flags.len();
        seed(&mut self.flags, &mut self.items, count);
        proof { seed_indexed(self.items@, self.flags@, count); }
    }

    /// レイアウトに含まれる全素子の数を返す。
    #[inline]
    pub fn node_count(&self) -> (count: usize)
        ensures count == self.flags_model().len(),
    {
        self.flags.len()
    }

    /// 現在の候補数を返す。
    #[inline]
    pub fn pending_count(&self) -> (count: usize)
        ensures count == self.items_model().len(),
    {
        self.items.len()
    }

    /// 範囲内の未登録素子を候補に加える。登録済みか範囲外なら変更しない。
    #[inline]
    pub fn insert(&mut self, node: usize)
        requires indexed(old(self).items_model(), old(self).flags_model()),
        ensures
            indexed(final(self).items_model(), final(self).flags_model()),
            final(self).flags_model().len() == old(self).flags_model().len(),
            final(self).items_model() == if node < old(self).flags_model().len() && !old(self).scheduled(node) {
                old(self).items_model().push(node)
            } else {
                old(self).items_model()
            },
            forall|i: usize| #[trigger] final(self).scheduled(i) <==>
                old(self).scheduled(i) || (i == node && i < old(self).flags_model().len()),
    {
        if node >= self.flags.len() || self.flags[node] {
            return;
        }
        proof { appended(self.items@, self.flags@, node); }
        self.flags.set(node, true);
        self.items.push(node);
    }

    /// 渡された素子のうち、範囲内の全件を候補に加える。
    #[inline]
    pub fn insert_all(&mut self, nodes: &[usize])
        requires indexed(old(self).items_model(), old(self).flags_model()),
        ensures
            indexed(final(self).items_model(), final(self).flags_model()),
            final(self).flags_model().len() == old(self).flags_model().len(),
            forall|i: usize| #[trigger] final(self).scheduled(i) <==>
                old(self).scheduled(i) || (i < old(self).flags_model().len() && nodes@.contains(i)),
    {
        let mut at = 0;
        while at < nodes.len()
            invariant
                at <= nodes@.len(),
                indexed(self.items_model(), self.flags_model()),
                self.flags_model().len() == old(self).flags_model().len(),
                forall|i: usize| #[trigger] self.scheduled(i) <==>
                    old(self).scheduled(i) || (i < old(self).flags_model().len() && nodes@.take(at as int).contains(i)),
            decreases nodes@.len() - at,
        {
            let ghost prior = *self;
            let node = nodes[at];
            self.insert(node);
            proof {
                let prefix = nodes@.take(at as int);
                assert(nodes@.take(at + 1) =~= prefix.push(node));
                assert forall|i: usize| #[trigger] self.scheduled(i) <==>
                    (old(self).scheduled(i) || (i < old(self).flags_model().len() && nodes@.take(at + 1).contains(i))) by {
                    assert(prior.scheduled(i) == (old(self).scheduled(i)
                        || (i < old(self).flags_model().len() && prefix.contains(i))));
                    lemma_seq_contains_after_push(prefix, node, i);
                };
            }
            at += 1;
        }
        assert(nodes@.take(at as int) =~= nodes@);
    }

    /// 末尾の候補を一件取り出す。空なら `None` を返す。
    #[inline]
    pub fn pop(&mut self) -> (result: Option<usize>)
        requires indexed(old(self).items_model(), old(self).flags_model()),
        ensures
            indexed(final(self).items_model(), final(self).flags_model()),
            final(self).flags_model().len() == old(self).flags_model().len(),
            result.is_none() <==> old(self).items_model().len() == 0,
            match result {
                Some(node) => node < old(self).flags_model().len() && node == old(self).items_model().last()
                    && final(self).items_model() == old(self).items_model().drop_last()
                    && !final(self).scheduled(node)
                    && forall|i: usize| i != node ==> (#[trigger] final(self).scheduled(i) == old(self).scheduled(i)),
                None => final(self).items_model() == old(self).items_model()
                    && final(self).flags_model() == old(self).flags_model(),
            },
    {
        let ghost before = self.items@;
        let node = self.items.pop()?;
        proof { removed(before, self.flags@, node); }
        self.flags.set(node, false);
        Some(node)
    }
}

}
