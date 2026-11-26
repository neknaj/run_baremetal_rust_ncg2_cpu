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

/// 最初の取り込みから round 回分だけ進んだ時刻を表す。
pub open spec fn round_at(first: int, tick: int, round: int) -> bool {
    round >= 0 && tick == first + 518 * round
}

/// リセットから初回の開始要求が効く直前まで、取り込みがなく完了フラグは0のままになる。
pub proof fn reset_multiply_has_no_pending_captures(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    until: int,
)
    requires
        multiply_control(start, capture, done, finished),
        0 <= until,
        forall|t: int| t <= 0 ==> !(#[trigger] capture(t)),
        forall|t: int| t <= 0 ==> !(#[trigger] done(t)),
        forall|t: int| 0 < t <= until ==> !(#[trigger] start(t - 3)),
    ensures
        forall|t: int| t <= until ==> !(#[trigger] capture(t)),
        forall|t: int| t <= until + 1 ==> !(#[trigger] done(t)),
        forall|t: int| t <= until + 6 ==> !(#[trigger] finished(t)),
    decreases until,
{
    if until > 0 {
        reset_multiply_has_no_pending_captures(start, capture, done, finished, until - 1);
        assert(!start(until - 3));
        assert(!capture(until - 518));
    }
    assert(!capture(until));
    assert(!done(until));
    assert(!done(until + 1));
    assert forall|t: int| t <= until + 6 implies !(#[trigger] finished(t)) by {
        assert(!done(t - 5));
    }
}

/// 開始前に反復の遅延列が空なら、同じ開始要求に由来する取り込みは518 tick 間隔に並ぶ。
pub proof fn capture_follows_single_start(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    first: int,
    until: int,
)
    requires
        multiply_control(start, capture, done, finished),
        first <= until,
        forall|t: int| first - 518 <= t < first ==> !(#[trigger] capture(t)),
        forall|t: int| first < t <= until ==> !(#[trigger] start(t - 3)),
    ensures
        forall|t: int| first <= t <= until && #[trigger] capture(t)
            ==> exists|k: int| #[trigger] round_at(first, t, k),
    decreases until - first,
{
    if first < until {
        capture_follows_single_start(start, capture, done, finished, first, until - 1);
    }
    assert forall|t: int| first <= t <= until && #[trigger] capture(t) implies
        exists|k: int| #[trigger] round_at(first, t, k) by {
        if t == until {
            if t == first {
                assert(round_at(first, t, 0));
            } else {
                assert(!start(t - 3));
                assert(capture(t - 518));
                assert(t - 518 >= first);
                let k = choose|k: int| #[trigger] round_at(first, t - 518, k);
                assert(round_at(first, t, k + 1));
            }
        }
    }
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

/// 空の反復遅延列から開始した乗算では、完了した取り込みの直前にも別の取り込みがない。
/// その間隔と完了フラグの保持を使い、次の開始要求までの反復と重複通知を排除する。
pub proof fn isolated_multiply_stops_after_completion(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    first: int,
    last: int,
    until: int,
)
    requires
        multiply_control(start, capture, done, finished),
        first <= last <= until,
        forall|t: int| first - 518 <= t < first ==> !(#[trigger] capture(t)),
        forall|t: int| first < t <= until ==> !(#[trigger] start(t - 3)),
        capture(last),
        done(last + 1),
    ensures
        forall|t: int| last - 517 <= t < last ==> !(#[trigger] capture(t)),
        forall|t: int| last < t <= until ==> !(#[trigger] capture(t)),
        forall|t: int| last + 7 <= t <= until + 7 ==> !(#[trigger] finished(t)),
{
    capture_follows_single_start(start, capture, done, finished, first, last);
    let last_round = choose|k: int| #[trigger] round_at(first, last, k);
    assert forall|t: int| last - 517 <= t < last implies !(#[trigger] capture(t)) by {
        if capture(t) && t >= first {
            let round = choose|k: int| #[trigger] round_at(first, t, k);
            assert(round < last_round);
            assert(t <= last - 518);
        }
    }
    completed_round_stops_repeating(start, capture, done, finished, last, until);
}

/// 次の開始要求から3 tick 後を新しい区間の先頭とする。
/// 先頭まで519 tick 以上離れていれば、前の乗算が停止した結果を次の区間の前提にできる。
pub proof fn completed_multiply_leaves_next_window_empty(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    first: int,
    last: int,
    next_first: int,
)
    requires
        multiply_control(start, capture, done, finished),
        first <= last,
        last + 519 <= next_first,
        forall|t: int| first - 518 <= t < first ==> !(#[trigger] capture(t)),
        forall|t: int| first < t < next_first ==> !(#[trigger] start(t - 3)),
        capture(last),
        done(last + 1),
    ensures
        forall|t: int| next_first - 518 <= t < next_first ==> !(#[trigger] capture(t)),
        forall|t: int| last + 7 <= t <= next_first + 6 ==> !(#[trigger] finished(t)),
{
    isolated_multiply_stops_after_completion(start, capture, done, finished, first, last, next_first - 1);
}

} // verus!
