//! 常に有効な1ビットラッチの直列接続を、入力が変化した時刻から進める。

use alloc::vec::Vec;
use vstd::prelude::*;

verus! {

/// すべての段が旧状態を読むときの1 tick の更新。
pub open spec fn delay_tick(previous: Seq<bool>, sample: bool, next: Seq<bool>) -> bool {
    &&& 0 < previous.len() == next.len()
    &&& next[0] == sample
    &&& forall|i: int| 0 < i < next.len() ==> (#[trigger] next[i]) == previous[i - 1]
}

/// 初期状態から、各 tick に入力を一度サンプルしたときの全段の履歴。
pub open spec fn delay_trace(initial: Seq<bool>, sample: spec_fn(int) -> bool, states: Seq<Seq<bool>>) -> bool {
    &&& 0 < states.len()
    &&& 0 < initial.len()
    &&& states[0] == initial
    &&& forall|t: int| 0 < t < states.len() ==> delay_tick(states[t - 1], sample(t), #[trigger] states[t])
}

/// 通常の tick 更新で得る段の値は、初期段か、入力を読んだ tick のサンプルに一致する。
pub proof fn delay_stage_agrees(
    initial: Seq<bool>, sample: spec_fn(int) -> bool, states: Seq<Seq<bool>>, tick: int, stage: int,
)
    requires delay_trace(initial, sample, states), 0 <= tick < states.len(), 0 <= stage < initial.len(),
    ensures
        states[tick].len() == initial.len(),
        states[tick][stage] == if tick <= stage { initial[stage - tick] } else { sample(tick - stage) },
    decreases tick,
{
    if tick > 0 {
        assert(delay_tick(states[tick - 1], sample(tick), states[tick]));
        if stage == 0 {
            delay_stage_agrees(initial, sample, states, tick - 1, 0);
        } else {
            delay_stage_agrees(initial, sample, states, tick - 1, stage - 1);
        }
    }
}

/// 鎖の端点のみを読む処理には、通常更新と同じ値が同じ tick に届く。
pub proof fn delay_endpoint_agrees(
    initial: Seq<bool>, sample: spec_fn(int) -> bool, states: Seq<Seq<bool>>,
)
    requires delay_trace(initial, sample, states),
    ensures
        forall|t: int| 0 <= t < states.len() ==> {
            let state = #[trigger] states[t];
            let index = initial.len() - 1;
            state.last() == if t <= index { initial[index - t] } else { sample(t - index) }
        },
{
    assert forall|t: int| 0 <= t < states.len() implies {
        let state = #[trigger] states[t];
        let index = initial.len() - 1;
        state.last() == if t <= index { initial[index - t] } else { sample(t - index) }
    } by {
        delay_stage_agrees(initial, sample, states, t, initial.len() - 1);
    };
}

struct Edge {
    tick: u64,
    value: bool,
}

spec fn sampled_at(edges: Seq<Edge>, tick: int, index: int, value: bool) -> bool {
    &&& 0 <= index < edges.len()
    &&& edges[index].tick <= tick
    &&& (index + 1 == edges.len() || tick < edges[index + 1].tick)
    &&& value == edges[index].value
}

proof fn preserve_sample(
    before: Seq<Edge>, after: Seq<Edge>, tick: int, value: bool, pushed: Option<Edge>,
)
    requires after == match pushed { Some(edge) => before.push(edge), None => before },
        pushed matches Some(edge) ==> tick < edge.tick,
    ensures (exists|i: int| #[trigger] sampled_at(before, tick, i, value))
        == (exists|i: int| #[trigger] sampled_at(after, tick, i, value)),
{
    if let Some(edge) = pushed {
        if exists|i: int| #[trigger] sampled_at(before, tick, i, value) {
            let i = choose|i: int| #[trigger] sampled_at(before, tick, i, value);
            assert(after[i] == before[i]);
            if i + 1 < before.len() {
                assert(after[i + 1] == before[i + 1]);
            } else {
                assert(after[i + 1] == edge);
            }
            assert(sampled_at(after, tick, i, value));
        }
        if exists|i: int| #[trigger] sampled_at(after, tick, i, value) {
            let i = choose|i: int| #[trigger] sampled_at(after, tick, i, value);
            assert(after[before.len() as int] == edge);
            assert(i < before.len());
            assert(after[i] == before[i]);
            if i + 1 < before.len() { assert(after[i + 1] == before[i + 1]); }
            assert(sampled_at(before, tick, i, value));
        }
    }
}

/// 入力を読んだ tick とその値のうち、各同値区間の先頭だけを保持する履歴。
///
/// 各段は分岐のない直列接続を前提とする。端点の観測と、保存時に必要な全段の読み出しに使える。
/// 元の内部段を読む別の素子や公開出力があれば、その段を端点として別に扱う。
pub struct DelayHistory {
    initial: Vec<bool>,
    edges: Vec<Edge>,
    through: u64,
}

/// 入力履歴を延ばせなかった理由。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DelayHistoryError {
    /// 既に入力を確定した tick を指定した。
    PastTick,
    /// 入力の変化を記録する領域を確保できない。
    Allocation,
}

impl DelayHistory {
    /// tick 0 に記録した初期段。最初に入力を読む段を先頭に置く。
    pub closed spec fn initial_model(&self) -> Seq<bool> { self.initial@ }

    /// 入力サンプルが確定した最後の tick。
    pub closed spec fn through_model(&self) -> u64 { self.through }

    /// 指定した tick に適用した入力が、この区間の値から得られることを表す。
    pub closed spec fn sample_matches(&self, tick: int, value: bool) -> bool {
        &&& 0 < tick <= self.through
        &&& exists|i: int| #[trigger] sampled_at(self.edges@, tick, i, value)
    }

    /// 空でない初期段と、昇順の区間開始が保たれていることを表す。
    pub closed spec fn valid(&self) -> bool {
        &&& self.initial@.len() > 0
        &&& (self.through == 0 <==> self.edges@.len() == 0)
        &&& (self.edges@.len() > 0 ==> self.edges@[0].tick == 1)
        &&& forall|i: int| 0 <= i < self.edges@.len() ==> (#[trigger] self.edges@[i]).tick <= self.through
        &&& forall|i: int, j: int| 0 <= i < j < self.edges@.len() ==>
            (#[trigger] self.edges@[i]).tick < (#[trigger] self.edges@[j]).tick
        &&& forall|i: int| 0 <= i + 1 < self.edges@.len() && i >= 0 ==>
            (#[trigger] self.edges@[i + 1]).value != self.edges@[i].value
    }

    /// 書き込み前の全段から履歴を始める。空の鎖には `None` を返す。
    pub fn new(initial: Vec<bool>) -> (history: Option<Self>)
        ensures
            history.is_some() <==> initial@.len() > 0,
            history matches Some(value) ==> value.valid() && value.initial_model() == initial@
                && value.through_model() == 0,
    {
        if initial.is_empty() { return None; }
        Some(Self { initial, edges: Vec::new(), through: 0 })
    }

    /// 指定した tick まで、未記録の全 tick に同じ入力を適用する。
    ///
    /// 入力が直前の値から変わる場合だけ、その区間の先頭を記録する。
    /// 内部配線を読む場合は tick 開始時の値を渡す。
    ///
    /// # Errors
    ///
    /// 確定済みの tick を指定した場合と記録領域を確保できない場合にエラーを返す。
    /// いずれの場合も履歴を変更しない。
    pub fn extend_through(&mut self, through: u64, sample: bool) -> (result: Result<(), DelayHistoryError>)
        requires old(self).valid(),
        ensures
            final(self).valid(),
            (result == Err(DelayHistoryError::PastTick)) <==> through <= old(self).through_model(),
            final(self).initial_model() == old(self).initial_model(),
            result is Err ==> final(self).through_model() == old(self).through_model(),
            forall|t: int, value: bool| 0 < t <= old(self).through_model() ==>
                #[trigger] final(self).sample_matches(t, value) == old(self).sample_matches(t, value),
            result is Ok ==> {
                &&& final(self).through_model() == through
                &&& forall|t: int| old(self).through_model() < t <= through ==>
                    #[trigger] final(self).sample_matches(t, sample)
            },
    {
        if through <= self.through { return Err(DelayHistoryError::PastTick); }
        let pushed = match self.edges.last() {
            Some(edge) => edge.value != sample,
            None => true,
        };
        let ghost before = self.edges@;
        let ghost previous = self.through;
        let ghost addition: Option<Edge>;
        if pushed {
            if self.edges.try_reserve(1).is_err() { return Err(DelayHistoryError::Allocation); }
            let edge = Edge { tick: self.through + 1, value: sample };
            proof { addition = Some(edge); }
            self.edges.push(edge);
        } else {
            proof { addition = None; }
        }
        self.through = through;
        proof {
            assert(self.edges@.last().tick <= previous + 1);
            assert(self.edges@.last().value == sample);
            assert forall|i: int, j: int| 0 <= i < j < self.edges@.len() implies
                (#[trigger] self.edges@[i]).tick < (#[trigger] self.edges@[j]).tick by {
                if j < before.len() {
                    assert(self.edges@[i] == before[i] && self.edges@[j] == before[j]);
                }
            };
            assert forall|t: int| previous < t <= through implies #[trigger] self.sample_matches(t, sample) by {
                let i = self.edges@.len() - 1;
                assert(0 <= i < self.edges@.len());
                assert(sampled_at(self.edges@, t, i, sample));
            };
            assert forall|t: int, value: bool| 0 < t <= previous implies
                #[trigger] self.sample_matches(t, value) == old(self).sample_matches(t, value) by {
                preserve_sample(before, self.edges@, t, value, addition);
            };
        }
        Ok(())
    }

    fn upper_bound(&self, tick: u64) -> (index: usize)
        requires self.valid(),
        ensures
            index <= self.edges@.len(),
            forall|i: int| 0 <= i < index ==> (#[trigger] self.edges@[i]).tick <= tick,
            forall|i: int| index <= i < self.edges@.len() ==> tick < (#[trigger] self.edges@[i]).tick,
    {
        let mut left = 0;
        let mut right = self.edges.len();
        while left < right
            invariant self.valid(), left <= right <= self.edges@.len(),
                forall|i: int| 0 <= i < left ==> (#[trigger] self.edges@[i]).tick <= tick,
                forall|i: int| right <= i < self.edges@.len() ==> tick < (#[trigger] self.edges@[i]).tick,
            decreases right - left,
        {
            let middle = left + (right - left) / 2;
            if self.edges[middle].tick <= tick {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        left
    }

    /// 登録段が指定した tick に持つ値を、初期段または確定済みサンプルから定める。
    pub open spec fn stage_value_matches(&self, stage: int, tick: int, value: bool) -> bool {
        &&& 0 <= stage < self.initial_model().len()
        &&& tick >= 0
        &&& if tick <= stage {
            value == self.initial_model()[stage - tick]
        } else {
            self.sample_matches(tick - stage, value)
        }
    }

    /// 範囲内の段が指定した tick に持つ値を返す。
    ///
    /// 入力が未記録でも、その変化が指定した段に届く前の値は確定している。
    /// 段が存在しないか、値が未確定なら `None` を返す。
    pub fn stage(&self, stage: usize, tick: u64) -> (result: Option<bool>)
        requires self.valid(),
        ensures
            result.is_some() <==> stage < self.initial_model().len()
                && tick as int - stage as int <= self.through_model(),
            result matches Some(value) ==> self.stage_value_matches(stage as int, tick as int, value),
    {
        if stage >= self.initial.len() { return None; }
        let offset = stage as u64;
        if tick <= offset { return Some(self.initial[stage - tick as usize]); }
        let sampled = tick - offset;
        if sampled > self.through { return None; }
        let upper = self.upper_bound(sampled);
        assert(self.edges@[0].tick == 1);
        let value = self.edges[upper - 1].value;
        assert(sampled_at(self.edges@, sampled as int, upper as int - 1, value));
        assert(self.sample_matches(sampled as int, value));
        Some(value)
    }

    /// 端点の値が変わらないと分かっている連続区間の末尾を返す。
    ///
    /// 次の入力変化が端点に達する手前、最後に確定した入力が届く tick、および `limit` を
    /// 越えない。初期段の値が届いている間は `now` を返す。区間が逆順の場合と、現在の端点
    /// の値が未確定の場合は `None` を返す。
    pub fn endpoint_unchanged_through(&self, now: u64, limit: u64) -> (result: Option<u64>)
        requires self.valid(),
        ensures
            result.is_some() <==> now <= limit
                && now as int - (self.initial_model().len() - 1) <= self.through_model(),
            result matches Some(end) ==> {
                let stage = self.initial_model().len() - 1;
                &&& now <= end <= limit
                &&& exists|value: bool| {
                    &&& #[trigger] self.stage_value_matches(stage, now as int, value)
                    &&& forall|t: int| now <= t <= end ==> #[trigger] self.stage_value_matches(stage, t, value)
                }
            },
    {
        if now > limit { return None; }
        let stage = self.initial.len() - 1;
        let latency = stage as u64;
        if now <= latency {
            assert(self.stage_value_matches(stage as int, now as int, self.initial@[stage as int - now as int]));
            return Some(now);
        }
        let current = now - latency;
        if current > self.through { return None; }
        let next = self.upper_bound(current);
        assert(self.edges@[0].tick == 1);
        let end = if next < self.edges.len() {
            self.edges[next].tick - 1
        } else {
            self.through
        };
        let sampled_until = end.min(limit - latency);
        let result = sampled_until + latency;
        proof {
            let index = next as int - 1;
            let value = self.edges@[index].value;
            assert forall|t: int| now <= t <= result implies
                #[trigger] self.stage_value_matches(stage as int, t, value) by {
                let sampled = t - stage;
                assert(self.edges@[index].tick <= sampled);
                assert(next == self.edges@.len() || sampled < self.edges@[next as int].tick);
                assert(sampled_at(self.edges@, sampled, index, value));
                assert(self.sample_matches(sampled, value));
            };
            assert(self.stage_value_matches(stage as int, now as int, value));
        }
        Some(result)
    }

    /// 履歴から読んだ値は、記録した入力と元の全段から通常更新を続けた結果である。
    pub proof fn stage_agrees(
        &self, sample: spec_fn(int) -> bool, states: Seq<Seq<bool>>, tick: int, stage: int, value: bool,
    )
        requires
            self.valid(), delay_trace(self.initial_model(), sample, states),
            0 <= tick < states.len(), 0 <= stage < self.initial_model().len(),
            self.stage_value_matches(stage, tick, value),
            tick > stage ==> self.sample_matches(tick - stage, sample(tick - stage)),
        ensures states[tick].len() == self.initial_model().len(), value == states[tick][stage],
    {
        delay_stage_agrees(self.initial_model(), sample, states, tick, stage);
        if tick > stage {
            let at = tick - stage;
            assert(self.sample_matches(at, sample(at)));
            let i = choose|i: int| #[trigger] sampled_at(self.edges@, at, i, value);
            let j = choose|j: int| #[trigger] sampled_at(self.edges@, at, j, sample(at));
            if i < j {
                assert(i + 1 <= j);
                assert(self.edges@[i + 1].tick <= self.edges@[j].tick);
            }
            if j < i {
                assert(j + 1 <= i);
                assert(self.edges@[j + 1].tick <= self.edges@[i].tick);
            }
            assert(i == j);
        }
    }

    /// 一定と判定した区間では、通常の tick 更新でも端点の値が変わらない。
    ///
    /// 呼び出し側は、対象の鎖の端点以外を読む素子がないことを配線から確かめる。
    pub proof fn stable_interval_agrees(
        &self, sample: spec_fn(int) -> bool, states: Seq<Seq<bool>>, start: int, end: int, value: bool,
    )
        requires self.valid(), delay_trace(self.initial_model(), sample, states), 0 <= start <= end < states.len(),
            forall|t: int| 0 < t <= self.through_model() ==> self.sample_matches(t, #[trigger] sample(t)),
            forall|t: int| start <= t <= end ==>
                #[trigger] self.stage_value_matches(self.initial_model().len() - 1, t, value),
        ensures forall|t: int| start <= t <= end ==> (#[trigger] states[t]).last() == value,
    {
        let stage = self.initial_model().len() - 1;
        assert forall|t: int| start <= t <= end implies (#[trigger] states[t]).last() == value by {
            assert(self.stage_value_matches(stage, t, value));
            if t > stage {
                assert(self.sample_matches(t - stage, value));
                assert(self.sample_matches(t - stage, sample(t - stage)));
            }
            self.stage_agrees(sample, states, t, stage, value);
        };
    }
}

} // verus!
