//! 遅延鎖の入力変更を、末尾が更新される tick に対応付ける。

use vstd::prelude::*;

#[cfg(verus_only)]
use crate::delay::{DelayHistory, delay_stage_agrees, delay_trace};

verus! {

/// 元の配線で入力値が変わった時点。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DelayInputEdge {
    /// tick の先頭で適用した外部入力。この tick に先頭段が読み取る。
    External {
        /// 入力を適用した tick。最初は1。
        tick: u64,
        /// 適用した値。
        value: bool,
    },
    /// tick の末尾で確定した NOR または状態素子の登録出力。
    Committed {
        /// 出力を確定した tick。reset 直後の値は0を使う。
        tick: u64,
        /// 確定した値。
        value: bool,
    },
}

impl DelayInputEdge {
    /// 先頭段が入力を読み取る時刻を数学上の整数で表す。
    pub open spec fn sample_tick(self) -> int {
        match self {
            Self::External { tick, .. } => tick as int,
            Self::Committed { tick, .. } => tick as int + 1,
        }
    }

    /// 先頭段が読み取る値を表す。
    pub open spec fn sampled_value(self) -> bool {
        match self {
            Self::External { value, .. } | Self::Committed { value, .. } => value,
        }
    }
}

/// 遅延鎖の末尾に届く入力と時刻。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DelayEndUpdate {
    /// 先頭段が元の入力を読み取る tick。
    pub sample_tick: u64,
    /// 末尾の登録値として確定する tick。この tick の公開出力に使う。
    pub output_tick: u64,
    /// 末尾に書く値。
    pub value: bool,
}

/// 入力変更を末尾の予定に登録できなかった理由。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DelayScheduleError {
    /// 遅延鎖に段がない。
    EmptyChain,
    /// tick 0 は外部入力を適用する更新時刻の範囲外である。
    InputBeforeFirstTick,
    /// 入力の採取または末尾への到着が、指定した時刻上限を超える。
    TimeLimit,
}

/// 段数と入力の発生元から、末尾への到着時刻と拒否理由を定める。
pub open spec fn delay_schedule(
    stages: nat, edge: DelayInputEdge, limit: u64,
) -> Result<DelayEndUpdate, DelayScheduleError> {
    let sample = edge.sample_tick();
    let end = sample + stages - 1;
    if stages == 0 {
        Err(DelayScheduleError::EmptyChain)
    } else if sample == 0 {
        Err(DelayScheduleError::InputBeforeFirstTick)
    } else if end > limit {
        Err(DelayScheduleError::TimeLimit)
    } else {
        Ok(DelayEndUpdate {
            sample_tick: sample as u64,
            output_tick: end as u64,
            value: edge.sampled_value(),
        })
    }
}

/// 入力変更から、遅延鎖の末尾を更新する予定を作る。
///
/// 末尾の更新は、その tick の各素子が旧状態を読んだ後に確定する。その値を読む素子には、
/// 次の tick で変更を通知する。端点の公開出力は更新を確定した tick に変わる。
///
/// # Errors
///
/// 空の鎖、tick 0 の外部入力、または `limit` を超える時刻を区別して返す。
pub fn schedule_delay_edge(stages: usize, edge: DelayInputEdge, limit: u64)
    -> (result: Result<DelayEndUpdate, DelayScheduleError>)
    ensures result == delay_schedule(stages as nat, edge, limit),
{
    if stages == 0 { return Err(DelayScheduleError::EmptyChain); }
    let (sample_tick, value) = match edge {
        DelayInputEdge::External { tick: 0, .. } => return Err(DelayScheduleError::InputBeforeFirstTick),
        DelayInputEdge::External { tick, value } => (tick, value),
        DelayInputEdge::Committed { tick, value } => {
            let Some(sample) = tick.checked_add(1) else { return Err(DelayScheduleError::TimeLimit); };
            (sample, value)
        },
    };
    let latency = (stages - 1) as u64;
    if sample_tick > limit || latency > limit - sample_tick {
        return Err(DelayScheduleError::TimeLimit);
    }
    Ok(DelayEndUpdate { sample_tick, output_tick: sample_tick + latency, value })
}

/// 同じ鎖では、採取した入力と末尾更新の時刻の順序が等しい。
pub proof fn delay_schedule_preserves_order(
    stages: nat, first: DelayInputEdge, second: DelayInputEdge, limit: u64,
    a: DelayEndUpdate, b: DelayEndUpdate,
)
    requires delay_schedule(stages, first, limit) == Ok(a), delay_schedule(stages, second, limit) == Ok(b),
    ensures
        (a.sample_tick < b.sample_tick) == (a.output_tick < b.output_tick),
        (a.sample_tick == b.sample_tick) == (a.output_tick == b.output_tick),
{
}

/// 時刻付きの末尾更新は、履歴から同じ末尾の段を読んだ値に一致する。
///
/// 履歴の tick 0 は `origin` の確定状態に対応する。例えば途中の保存から再開するときは、
/// 保存した tick を `origin` に渡す。
pub proof fn scheduled_delay_matches_history(
    history: &DelayHistory, origin: u64, edge: DelayInputEdge, limit: u64, update: DelayEndUpdate,
)
    requires
        delay_schedule(history.initial_model().len(), edge, limit) == Ok(update),
        origin < update.sample_tick,
        history.sample_matches(update.sample_tick as int - origin as int, update.value),
    ensures history.stage_value_matches(
        history.initial_model().len() - 1, update.output_tick as int - origin as int, update.value,
    ),
{
}

/// 同じ入力を与えて1 tick ずつ更新した場合も、予定の tick に末尾から同じ値を読める。
pub proof fn scheduled_delay_matches_trace(
    initial: Seq<bool>, sample: spec_fn(int) -> bool, states: Seq<Seq<bool>>,
    origin: u64, edge: DelayInputEdge, limit: u64, update: DelayEndUpdate,
)
    requires
        delay_schedule(initial.len(), edge, limit) == Ok(update),
        origin < update.sample_tick,
        (update.output_tick as int - origin as int) < states.len(),
        delay_trace(initial, sample, states),
        sample(update.sample_tick as int - origin as int) == update.value,
    ensures states[update.output_tick as int - origin as int].last() == update.value,
{
    delay_stage_agrees(initial, sample, states,
        update.output_tick as int - origin as int, initial.len() - 1);
}


/// 現在の tick から指定の幅だけ進めても、時刻区間の境界を越えず予定を通過しない。
pub open spec fn idle_prefix_allowed(
    now: u64, boundary: u64, next: Option<u64>, requested: u64, span: u64,
) -> bool {
    span <= requested
    && now as int + span <= boundary
    && (span == 0 || next.is_none() || now as int + span < next.unwrap())
}

/// 次の末尾更新を含まない範囲で進められる最大の tick 数を返す。
///
/// 入力と候補キューの確認は呼び出し側で行う。時刻区間の境界を越える要求には、
/// 境界までの幅を返す。現在時刻が境界より後ろにある場合は0を返す。
pub fn idle_prefix(now: u64, boundary: u64, next: Option<u64>, requested: u64) -> (span: u64)
    ensures
        now > boundary ==> span == 0,
        now <= boundary ==> idle_prefix_allowed(now, boundary, next, requested, span),
        forall|other: u64| #[trigger] idle_prefix_allowed(now, boundary, next, requested, other) ==> other <= span,
{
    if now > boundary {
        return 0;
    }
    let remaining = boundary - now;
    let available = if remaining < requested { remaining } else { requested };
    if let Some(next_tick) = next {
        if next_tick <= now {
            return 0;
        }
        let until_event = next_tick - now - 1;
        if until_event < available {
            return until_event;
        }
    }
    available
}

} // verus!
