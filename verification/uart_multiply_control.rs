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

/// 開始・反復による取り込みと、実行中の完了通知の等式を表す。
pub open spec fn multiply_pulse_timing(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
) -> bool {
    &&& forall|t: int| #[trigger] capture(t) == (t >= 3 && (start(t - 3)
        || (capture(t - 518) && running(t - 8) && !done(t - 9))))
    &&& forall|t: int| #[trigger] finished(t) == (t >= 5
        && done(t - 5) && !done(t - 6) && running(t - 3))
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
        multiply_pulse_timing(start, capture, done, finished, running),
        forall|t: int| t <= 0 ==> !(#[trigger] capture(t)),
{
    assert forall|t: int| #[trigger] capture(t) == (t >= 3 && (start(t - 3)
        || (capture(t - 518) && running(t - 8) && !done(t - 9)))) by {
        if repeat(t - 3) || capture(t - 518) {
            assert(capture(t - 518));
            // 反復は取り込みから512段以上を経るため、反転ゲートの初期値は届かない。
            assert(t - 518 >= 3);
            assert(not_yet(t - 6) == (running(t - 8) && not_done(t - 8)));
            assert(not_done(t - 8) == !done(t - 9));
        }
    }
    assert forall|t: int| #[trigger] finished(t) == (t >= 5
        && done(t - 5) && !done(t - 6) && running(t - 3)) by {
        if t >= 5 {
            assert(rising(t - 3) == (done(t - 5) && !done(t - 6)));
        }
    }
    assert forall|t: int| #[trigger] capture(t) implies start(t - 3)
        || (capture(t - 518) && !done(t - 9)) by {};
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

/// 開始から11 tick で running が立ち、完了から3 tick までの遅延を含む。
pub open spec fn multiply_running_timing(
    start: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
) -> bool {
    forall|t: int| t >= 11 ==> #[trigger] running(t) == if start(t - 11) || finished(t - 3) {
        start(t - 11)
    } else {
        running(t - 1)
    }
}

/// 開始が有効になってから、完了通知が running に届くまでは実行中のままになる。
pub proof fn multiply_runs_until_finish_reaches_latch(
    start: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
    first_start: int,
    until: int,
)
    requires
        multiply_running_timing(start, finished, running),
        0 <= first_start,
        first_start + 11 <= until,
        start(first_start),
        forall|t: int| first_start + 11 < t <= until ==> !(#[trigger] finished(t - 3)),
    ensures running(until),
    decreases until - first_start - 11,
{
    if first_start + 11 < until {
        multiply_runs_until_finish_reaches_latch(start, finished, running, first_start, until - 1);
        assert(!finished(until - 3));
    }
}

/// 初期化後に走り続け、次回要求の判定時点で未完了なら反復が必ず発生する。
pub proof fn unfinished_capture_causes_next_round(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
    first_start: int,
    captured: int,
)
    requires
        multiply_pulse_timing(start, capture, done, finished, running),
        multiply_running_timing(start, finished, running),
        0 <= first_start,
        first_start + 3 <= captured,
        start(first_start),
        capture(captured),
        !done(captured + 509),
        forall|t: int| first_start + 8 < t <= captured + 507 ==> !(#[trigger] finished(t)),
    ensures capture(captured + 518),
{
    multiply_runs_until_finish_reaches_latch(start, finished, running, first_start, captured + 510);
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

/// 配線検査で照合したラッチと NOR の遷移式。値は各 tick の確定後を指す。
pub open spec fn timed_round_mark(
    capture: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
    mark: spec_fn(int, int) -> bool,
    done: spec_fn(int) -> bool,
) -> bool {
    &&& forall|t: int, bit: int| t >= 5 && 0 <= bit < 64 ==> #[trigger] mark(t + 1, bit) ==
        if capture(t) {
            (!running(t - 5) && bit == 0)
                || (running(t - 4) && bit > 0 && mark(t - 4, bit - 1))
        } else {
            mark(t, bit)
        }
    &&& forall|t: int| t >= 5 ==> #[trigger] done(t + 1) == if capture(t) {
        running(t - 2) && (done(t - 2) || mark(t - 2, 63))
    } else {
        done(t)
    }
}

/// 取り込みがない区間では、マークと完了フラグは次の取り込みの直前まで保たれる。
proof fn round_state_holds(
    capture: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
    mark: spec_fn(int, int) -> bool,
    done: spec_fn(int) -> bool,
    from: int,
    until: int,
)
    requires
        timed_round_mark(capture, running, mark, done),
        5 <= from <= until,
        forall|t: int| from <= t < until ==> !(#[trigger] capture(t)),
    ensures
        done(until) == done(from),
        forall|bit: int| 0 <= bit < 64 ==> #[trigger] mark(until, bit) == mark(from, bit),
    decreases until - from,
{
    if from < until {
        round_state_holds(capture, running, mark, done, from, until - 1);
        assert(!capture(until - 1));
        assert(done((until - 1) + 1) == done(until - 1));
        assert forall|bit: int| 0 <= bit < 64 implies #[trigger] mark(until, bit) == mark(from, bit) by {
            assert(mark((until - 1) + 1, bit) == mark(until - 1, bit));
        }
    }
}

/// 単独の開始要求から各ラウンドが時刻どおり発生し、最終回まで完了通知を出さない。
pub proof fn isolated_multiply_reaches_round(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
    mark: spec_fn(int, int) -> bool,
    s: int,
    round: int,
)
    requires
        multiply_control(start, capture, done, finished),
        multiply_pulse_timing(start, capture, done, finished, running),
        multiply_running_timing(start, finished, running),
        timed_round_mark(capture, running, mark, done),
        2 <= s,
        0 <= round <= 64,
        start(s),
        forall|t: int| s + 3 - 518 <= t < s + 3 ==> !(#[trigger] capture(t)),
        forall|t: int| s < t <= s + 518 * round ==> !(#[trigger] start(t)),
        forall|t: int| s - 3 <= t <= s + 5 ==> !(#[trigger] running(t)),
    ensures
        capture(s + 3 + 518 * round),
        done(s + 4 + 518 * round) == (round == 64),
        forall|t: int| s + 4 <= t <= s + 3 + 518 * round ==> !(#[trigger] done(t)),
        forall|bit: int| 0 <= bit < 64 ==> #[trigger] mark(s + 4 + 518 * round, bit) == (bit == round),
        forall|t: int| s <= t <= s + 8 + 518 * round ==> !(#[trigger] finished(t)),
        round == 64 ==> finished(s + 9 + 518 * round),
    decreases round,
{
    let first = s + 3;
    let captured = first + 518 * round;
    if round == 0 {
        assert(capture(first));
        assert(!running(first - 5) && !running(first - 4) && !running(first - 2));
        assert forall|t: int| s <= t <= first + 5 implies !(#[trigger] finished(t)) by {
            assert(!running(t - 3));
        }
    } else {
        let prior = captured - 518;
        isolated_multiply_reaches_round(start, capture, done, finished, running, mark, s, round - 1);
        capture_follows_single_start(start, capture, done, finished, first, captured);
        assert forall|t: int| prior < t < captured implies !(#[trigger] capture(t)) by {
            if capture(t) {
                let k = choose|k: int| #[trigger] round_at(first, t, k);
                assert(round - 1 < k < round);
            }
        }
        assert forall|t: int| first + 1 <= t <= captured implies !(#[trigger] done(t)) by {
            if t > prior {
                round_state_holds(capture, running, mark, done, prior + 1, t);
            }
        }
        assert forall|t: int| s <= t <= captured + 5 implies !(#[trigger] finished(t)) by {
            if t > prior + 5 {
                assert(!done(t - 5));
            }
        }
        unfinished_capture_causes_next_round(start, capture, done, finished, running, s, prior);
        // マークの選択枝と完了フラグの組み合わせ経路は異なる tick の running を読む。
        multiply_runs_until_finish_reaches_latch(start, finished, running, s, captured - 5);
        multiply_runs_until_finish_reaches_latch(start, finished, running, s, captured - 4);
        multiply_runs_until_finish_reaches_latch(start, finished, running, s, captured - 2);
        round_state_holds(capture, running, mark, done, prior + 1, captured - 4);
        round_state_holds(capture, running, mark, done, prior + 1, captured - 2);
        assert(mark(captured - 2, 63) == (round == 64));
        assert forall|bit: int| 0 <= bit < 64 implies #[trigger] mark(captured + 1, bit) == (bit == round) by {
            assert(mark(captured + 1, bit) == (bit > 0 && mark(captured - 4, bit - 1)));
        }
        if round == 64 {
            assert(done(captured + 1));
            assert(!done(captured));
            multiply_runs_until_finish_reaches_latch(start, finished, running, s, captured + 3);
            assert(finished(captured + 6));
        }
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

/// 通常完了には1,027 tick 前の命令開始が必要になる。
/// 退役から命令開始へ戻る式と組み合わせるときは、対象の時刻だけ展開する。
#[verifier::opaque]
pub open spec fn cpu_simple_path(simple: spec_fn(int) -> bool, issue: spec_fn(int) -> bool) -> bool {
    forall|t: int| #[trigger] simple(t) ==> issue(t - 1027)
}

proof fn simple_notification_follows_issue(simple: spec_fn(int) -> bool, issue: spec_fn(int) -> bool, at: int)
    requires cpu_simple_path(simple, issue), simple(at),
    ensures issue(at - 1027),
{
    reveal(cpu_simple_path);
}

/// 乗算の命令枠が開いた後、最初の完了通知が次の枠に届くまでは issue が再発しない。
pub proof fn multiply_window_rejects_early_issue(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
    mark: spec_fn(int, int) -> bool,
    issue: spec_fn(int) -> bool,
    commit: spec_fn(int) -> bool,
    simple: spec_fn(int) -> bool,
    i: int,
    until: int,
)
    requires
        multiply_control(start, capture, done, finished),
        multiply_pulse_timing(start, capture, done, finished, running),
        multiply_running_timing(start, finished, running),
        timed_round_mark(capture, running, mark, done),
        cpu_multiply_start_path(start, issue, commit, simple, finished),
        cpu_simple_path(simple, issue),
        4 <= i <= until < i + 34197,
        start(i + 1027),
        !simple(i + 1027),
        forall|t: int| i - 1035 <= t < i ==> !(#[trigger] issue(t)),
        forall|t: int| i - 8 <= t < i + 1027 ==> !(#[trigger] finished(t)),
        forall|t: int| i + 1030 - 518 <= t < i + 1030 ==> !(#[trigger] capture(t)),
        forall|t: int| i + 1024 <= t <= i + 1032 ==> !(#[trigger] running(t)),
    ensures
        forall|t: int| i < t <= until ==> !(#[trigger] issue(t)),
    decreases until - i,
{
    if i < until {
        multiply_window_rejects_early_issue(
            start, capture, done, finished, running, mark, issue, commit, simple, i, until - 1,
        );
        if issue(until) {
            let retired = until - 9;
            let s = i + 1027;
            assert(commit(until - 3));
            assert(simple((until - 3) - 6) || finished((until - 3) - 6));
            if simple(retired) {
                simple_notification_follows_issue(simple, issue, retired);
                assert(retired - 1027 == i);
            }
            if retired >= s {
                // 退役通知の候補時刻を覆う最初の取り込みを選ぶ。
                let round = if retired <= s + 8 { 0 } else { (retired - s - 9) / 518 + 1 };
                assert(0 <= round <= 64);
                assert(retired <= s + 8 + 518 * round);
                assert(i + 518 * round < until);
                assert forall|t: int| s < t <= s + 518 * round implies !(#[trigger] start(t)) by {
                    assert(!issue(t - 1027));
                }
                isolated_multiply_reaches_round(start, capture, done, finished, running, mark, s, round);
            }
            assert(!finished(retired));
        }
    }
}

/// 通常完了と過去の完了が残っていない命令枠では、予定時刻に乗算が完了する。
/// この間の新しい issue や乗算開始要求の不在は前提に置かず、退役の経路から導く。
pub proof fn cpu_multiply_window_completes(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
    mark: spec_fn(int, int) -> bool,
    issue: spec_fn(int) -> bool,
    commit: spec_fn(int) -> bool,
    simple: spec_fn(int) -> bool,
    i: int,
)
    requires
        multiply_control(start, capture, done, finished),
        multiply_pulse_timing(start, capture, done, finished, running),
        multiply_running_timing(start, finished, running),
        timed_round_mark(capture, running, mark, done),
        cpu_multiply_start_path(start, issue, commit, simple, finished),
        cpu_simple_path(simple, issue),
        4 <= i,
        start(i + 1027),
        !simple(i + 1027),
        forall|t: int| i - 1035 <= t < i ==> !(#[trigger] issue(t)),
        forall|t: int| i - 8 <= t < i + 1027 ==> !(#[trigger] finished(t)),
        forall|t: int| i + 1030 - 518 <= t < i + 1030 ==> !(#[trigger] capture(t)),
        forall|t: int| i + 1024 <= t <= i + 1032 ==> !(#[trigger] running(t)),
    ensures
        forall|t: int| i < t < i + 34197 ==> !(#[trigger] issue(t)),
        forall|t: int| i - 8 <= t < i + 34188 ==> !(#[trigger] finished(t)),
        finished(i + 34188),
        forall|t: int| i + 34182 < t < i + 35227 ==> !(#[trigger] capture(t)),
        forall|t: int| i + 34188 < t <= i + 35233 ==> !(#[trigger] finished(t)),
        forall|t: int| i + 34191 <= t <= i + 35234 ==> !(#[trigger] running(t)),
{
    multiply_window_rejects_early_issue(
        start, capture, done, finished, running, mark, issue, commit, simple, i, i + 34196,
    );
    let s = i + 1027;
    assert forall|t: int| s < t <= s + 518 * 64 implies !(#[trigger] start(t)) by {
        assert(!issue(t - 1027));
    }
    isolated_multiply_reaches_round(start, capture, done, finished, running, mark, s, 64);
    let end = i + 34182;
    assert forall|t: int| s < t < end + 1042 implies !(#[trigger] start(t)) by {
        assert(!issue(t - 1027));
    }
    quiet_multiply_after_retirement(start, capture, done, finished, running, s, end);
}

/// 乗算の停止から、次の開始要求が届くまでの静穏区間を得る。
proof fn quiet_multiply_after_retirement(
    start: spec_fn(int) -> bool,
    capture: spec_fn(int) -> bool,
    done: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
    s: int,
    last: int,
)
    requires
        multiply_control(start, capture, done, finished),
        multiply_running_timing(start, finished, running),
        2 <= s && s + 3 <= last,
        forall|t: int| s + 3 - 518 <= t < s + 3 ==> !(#[trigger] capture(t)),
        forall|t: int| s < t < last + 1042 ==> !(#[trigger] start(t)),
        capture(last), done(last + 1), finished(last + 6),
    ensures
        forall|t: int| last < t < last + 1045 ==> !(#[trigger] capture(t)),
        forall|t: int| last + 6 < t <= last + 1051 ==> !(#[trigger] finished(t)),
        forall|t: int| last + 9 <= t <= last + 1052 ==> !(#[trigger] running(t)),
{
    isolated_multiply_stops_after_completion(start, capture, done, finished, s + 3, last, last + 1044);
    assert forall|t: int| last + 9 <= t <= last + 1052 implies !(#[trigger] running(t)) by {
        multiply_running_clears(start, finished, running, last + 6, t);
    }
}

/// 完了通知がラッチに届いた後、新しい開始要求が届くまでは実行中にならない。
proof fn multiply_running_clears(
    start: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    running: spec_fn(int) -> bool,
    end: int,
    until: int,
)
    requires
        multiply_running_timing(start, finished, running),
        11 <= end + 3 <= until,
        finished(end),
        forall|t: int| end + 3 <= t <= until ==> !(#[trigger] start(t - 11)),
    ensures !running(until),
    decreases until - end - 3,
{
    if end + 3 < until {
        multiply_running_clears(start, finished, running, end, until - 1);
    }
    assert(!start(until - 11));
}

/// 命令が有効なら、内部の乗算完了通知は commit と次の issue を起こす。
pub proof fn valid_multiply_retirement_opens_next_window(
    valid: spec_fn(int) -> bool,
    finished: spec_fn(int) -> bool,
    commit: spec_fn(int) -> bool,
    issue: spec_fn(int) -> bool,
    notification: int,
)
    requires
        0 <= notification,
        finished(notification),
        valid(notification + 3),
        forall|t: int| t >= 6 && #[trigger] finished(t - 6) && valid(t - 3) ==> commit(t),
        forall|t: int| t >= 4 && #[trigger] commit(t - 3) ==> issue(t),
    ensures commit(notification + 6), issue(notification + 9),
{
    assert(finished((notification + 6) - 6));
    assert(commit((notification + 9) - 3));
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
