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

/// 実配線から求めた起動時の遷移式をもとに、取り込みと完了の時刻条件を導く。
pub proof fn multiply_control_from_reset_wiring(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
    repeat: spec_fn(int) -> bool,
    not_done: spec_fn(int) -> bool,
    not_yet: spec_fn(int) -> bool,
    rising: spec_fn(int) -> bool,
)
    requires
        forall|t: int| #[trigger] capture(t) == (t >= 3 && (start(t - 3) || repeat(t - 3))),
        forall|t: int| #[trigger] repeat(t) == (t >= 5 && capture(t - 515) && not_yet(t - 3)),
        forall|t: int| t >= 1 ==> #[trigger] not_done(t) == !done(t - 1),
        forall|t: int| t >= 2 ==> #[trigger] not_yet(t) == (running(t - 2) && not_done(t - 2)),
        forall|t: int| t >= 2 ==> #[trigger] rising(t) == (done(t - 2) && !done(t - 3)),
        forall|t: int| #[trigger] finished(t) == (t >= 5 && rising(t - 3) && running(t - 3)),
        forall|t: int| t <= 0 ==> !(#[trigger] done(t)),
        forall|t: int| t >= 0 && !(#[trigger] capture(t)) ==> #[trigger] done(t + 1) == done(t),
    ensures
        multiply_control(start, capture, done, finished),
        forall|t: int| t <= 0 ==> !(#[trigger] capture(t)),
{
    assert forall|t: int| #[trigger] capture(t) implies start(t - 3)
        || (capture(t - 518) && !done(t - 9)) by {
        if !start(t - 3) {
            assert(repeat(t - 3));
            assert(t - 3 >= 5);
            assert(capture(t - 518));
            // 最初の取り込みから512段以上を経るため、反転ゲートの初期値は反復に届かない。
            assert(t - 518 >= 3);
            assert(t - 8 >= 1);
            assert(not_yet(t - 6));
            assert(not_done(t - 8));
        }
    }
    assert forall|t: int| #[trigger] done(t + 1) != done(t) implies capture(t) by {
        if t < 0 {
            assert(!done(t + 1) && !done(t));
        }
    }
    assert forall|t: int| #[trigger] finished(t) implies done(t - 5) && !done(t - 6) by {
        assert(t >= 5);
        assert(rising(t - 3));
    }
}

/// 最初の取り込みから round 回分だけ進んだ時刻を表す。
pub open spec fn round_at(first: int, tick: int, round: int) -> bool {
    round >= 0 && tick == first + 518 * round
}

/// 取り込み後のマークと完了フラグを、乗算を初期化する取り込みを0回目として表す。
pub open spec fn sampled_round_mark(
    mark: spec_fn(int, int) -> bool,
    done: spec_fn(int) -> bool,
) -> bool {
    &&& !done(0)
    &&& forall|bit: int| 0 <= bit < 64 ==> #[trigger] mark(0, bit) == (bit == 0)
    &&& forall|k: int, bit: int| 0 <= k < 64 && 0 <= bit < 64 ==>
        #[trigger] mark(k + 1, bit) == (bit > 0 && mark(k, bit - 1))
    &&& forall|k: int| 0 <= k < 64 ==> #[trigger] done(k + 1) == (done(k) || mark(k, 63))
}

/// オペランドに関わらず、初期化後の64回目の取り込みで初めて完了フラグが立つ。
pub proof fn round_mark_finishes_at_sixty_four(
    mark: spec_fn(int, int) -> bool,
    done: spec_fn(int) -> bool,
    round: int,
)
    requires sampled_round_mark(mark, done), 0 <= round <= 64,
    ensures
        forall|bit: int| 0 <= bit < 64 ==> #[trigger] mark(round, bit) == (bit == round),
        done(round) == (round == 64),
    decreases round,
{
    if round > 0 {
        round_mark_finishes_at_sixty_four(mark, done, round - 1);
        assert forall|bit: int| 0 <= bit < 64 implies #[trigger] mark(round, bit) == (bit == round) by {
            assert(mark((round - 1) + 1, bit) == (bit > 0 && mark(round - 1, bit - 1)));
        }
        assert(done((round - 1) + 1) == (done(round - 1) || mark(round - 1, 63)));
    }
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

/// CPU 内部の信号は tick 終了時の値で表す。
/// tick 4 の初回 issue を除き、乗算を始めるには退役から commit と issue を経由する。
pub open spec fn cpu_multiply_start_path(
    start: spec_fn(int) -> bool,
    issue: spec_fn(int) -> bool,
    commit: spec_fn(int) -> bool,
    simple: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
) -> bool {
    &&& forall|t: int| #[trigger] start(t) ==> issue(t - 1027)
    &&& forall|t: int| #[trigger] issue(t) ==> t == 4 || commit(t - 3)
    &&& forall|t: int| #[trigger] commit(t) ==> simple(t - 6) || finished(t - 6)
}

/// 一つの乗算の実行中に別の退役通知がなければ、次の開始までに反復は止まる。
pub proof fn retirement_gap_clears_multiply_history(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    issue: spec_fn(int) -> bool,
    commit: spec_fn(int) -> bool,
    simple: spec_fn(int) -> bool,
    first_start: int,
    last_capture: int,
)
    requires
        multiply_control(start, capture, done, finished),
        cpu_multiply_start_path(start, issue, commit, simple, finished),
        first_start >= 1031,
        first_start + 3 <= last_capture,
        forall|t: int| first_start + 3 - 518 <= t < first_start + 3 ==> !(#[trigger] capture(t)),
        forall|t: int| first_start - 1036 < t < last_capture + 6 ==> !(#[trigger] simple(t)),
        forall|t: int| first_start - 1036 < t < last_capture + 6 ==> !(#[trigger] finished(t)),
        capture(last_capture),
        done(last_capture + 1),
    ensures
        forall|t: int| first_start - 1027 < t < last_capture + 15 ==> !(#[trigger] issue(t)),
        forall|t: int| first_start < t < last_capture + 1042 ==> !(#[trigger] start(t)),
        forall|t: int| last_capture < t < last_capture + 1045 ==> !(#[trigger] capture(t)),
        forall|t: int| last_capture + 7 <= t <= last_capture + 1051 ==> !(#[trigger] finished(t)),
{
    assert forall|t: int| first_start - 1027 < t < last_capture + 15 implies !(#[trigger] issue(t)) by {
        if issue(t) {
            assert(t != 4);
            assert(commit(t - 3));
            assert(simple((t - 3) - 6) || finished((t - 3) - 6));
            assert(!simple(t - 9));
            assert(!finished(t - 9));
        }
    }
    assert forall|t: int| first_start < t < last_capture + 1042 implies !(#[trigger] start(t)) by {
        assert(!issue(t - 1027));
    }
    isolated_multiply_stops_after_completion(
        start, capture, done, finished, first_start + 3, last_capture, last_capture + 1044,
    );
}

} // verus!
