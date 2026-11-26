//! UART CPU の展開済み配線から得た時刻式で、乗算完了後の反復が途絶えることを示す。

use vstd::prelude::*;

verus! {

/// 立ち上がり通知までの遅延、取り込みがないときの保持、反復開始に必要な条件。
/// 反復には running も必要だが、ここではその値を任意にして証明する。
pub open spec fn multiply_control(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
) -> bool {
    &&& forall|t: int| #[trigger] capture(t) ==> start(t - 3)
        || (capture(t - 518) && !done(t - 9))
    &&& forall|t: int| #[trigger] done(t + 1) != done(t) ==> capture(t)
    &&& forall|t: int| #[trigger] finished(t) ==> done(t - 5) && !done(t - 6)
}

/// 完了を確定した取り込みの直前517 tick に別の取り込みがなければ、
/// 新しい開始要求が効くまで、保留中の反復も完了通知も発生しない。
pub proof fn completed_round_stops_repeating(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    last: int,
    until: int,
)
    requires
        multiply_control(start, capture, done, finished),
        last <= until,
        capture(last),
        done(last + 1),
        forall|t: int| last - 517 <= t < last ==> !(#[trigger] capture(t)),
        forall|t: int| last < t <= until ==> !(#[trigger] start(t - 3)),
    ensures
        forall|t: int| last < t <= until ==> !(#[trigger] capture(t)),
        forall|t: int| last < t <= until + 1 ==> #[trigger] done(t),
        forall|t: int| last + 7 <= t <= until + 7 ==> !(#[trigger] finished(t)),
    decreases until - last,
{
    if last < until {
        completed_round_stops_repeating(start, capture, done, finished, last, until - 1);
        assert(!start(until - 3));
        if until == last + 518 {
            assert(done(until - 9));
        } else {
            assert(!capture(until - 518));
        }
        assert(!capture(until));
        assert(done(until));
        assert(done(until + 1));
    }
    assert forall|t: int| last + 7 <= t <= until + 7 implies !(#[trigger] finished(t)) by {
        assert(done(t - 6));
    }
}

} // verus!
