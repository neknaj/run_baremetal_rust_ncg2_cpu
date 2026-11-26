//! UART の完成語を受け取り、RAM を経由して CPU へ制御を渡す。

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use vstd::prelude::*;

verus! {

/// 最初のペイロード語の物理アドレス。
pub const ENTRY_ADDRESS: u64 = 0x10000;
/// 既存 ELF の転送形式と同じ最大語数。
pub const MAX_WORDS: usize = 0x8400;
/// 書き込み中の語も含めたキューの容量。
pub const FIFO_WORDS: usize = 2;
/// 書き込みを指定する前に同じアドレスを駆動する tick 数。
pub const ADDRESS_SETUP: u8 = 32;
/// 既存の UART RAM が許可から確定するまでの tick 数。
pub const RAM_WRITE_LATENCY: u8 = 2;
/// 新しい語がなければ、満杯のキューでも全語の確定に要する最大 tick 数。
pub const FIFO_DRAIN_TICKS: usize = FIFO_WORDS * (ADDRESS_SETUP as usize + RAM_WRITE_LATENCY as usize + 1);

/// UART の受信バッファがこの tick に通知した内容。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Receive {
    /// 新しい語もエラーもない。
    Idle,
    /// 8バイトから組み立てたリトルエンディアンの語。
    Word(u64),
    /// 受信側が通知したフレーミングやバッファのエラー。
    Error,
}

/// reset まで再開できない受信エラー。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootError {
    /// UART の受信側がエラーを通知した。
    Receiver,
    /// 語数ヘッダーが許容範囲の外にある。
    HeaderCount,
    /// 宣言した数を超えてペイロードが届いた。
    ExcessWord,
    /// 保留できる語数を超えた。
    NoCredit,
}

/// 受信した入力をどう処理したかを示す。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Acceptance {
    /// この tick に受信していないか、ブート受信を終えている。
    None,
    /// 語数ヘッダーを受理した。
    Header,
    /// ペイロードをキューの末尾へ入れた。
    Payload,
    /// 受信を停止した理由。
    Rejected(BootError),
}

/// 対象 RAM の1語を指定する。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RamWord {
    /// 整列した物理アドレス。
    pub address: u64,
    /// 転送順に受理した内容。
    pub data: u64,
}

/// この tick に駆動する RAM ポートとブート制御。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootFrame {
    /// 旧キュー先頭、またはこの tick に初めて受理した先頭のアドレスとデータ。
    pub bus: Option<RamWord>,
    /// この tick の RAM 書き込み許可。真なら `bus` がある。
    pub write: bool,
    /// 既存 RAM の固定レイテンシに従い、この tick に確定する以前の要求。
    pub committed: Option<RamWord>,
    /// CPU が次の tick から RAM を使用できることを示す。
    pub cpu_ready: bool,
    /// 次のペイロード語を待たずに受理できる空き数。ヘッダー前と終了後は0。
    pub credit: usize,
    /// 今回の受信結果。
    pub accepted: Acceptance,
}

/// アドレスを保持している先頭語と、後続の受信語を同じ順序で管理する。
pub struct BootLoader {
    total: usize,
    committed: usize,
    fifo: Vec<u64>,
    held: u8,
    error: Option<BootError>,
    #[allow(dead_code)]
    accepted: Ghost<Seq<u64>>,
}

/// ヘッダー、ペイロード上限、入れる前の空き数に従い受信を判断する。
pub open spec fn select_acceptance(
    total: nat, committed: nat, pending: nat, failed: bool, input: Receive,
) -> Acceptance {
    if failed || (total > 0 && committed == total) {
        Acceptance::None
    } else {
        match input {
            Receive::Idle => Acceptance::None,
            Receive::Error => Acceptance::Rejected(BootError::Receiver),
            Receive::Word(word) => if total == 0 {
                if 1 <= word <= MAX_WORDS { Acceptance::Header }
                else { Acceptance::Rejected(BootError::HeaderCount) }
            } else if committed + pending >= total {
                Acceptance::Rejected(BootError::ExcessWord)
            } else if pending >= FIFO_WORDS {
                Acceptance::Rejected(BootError::NoCredit)
            } else {
                Acceptance::Payload
            },
        }
    }
}

/// ペイロードの語番号に対応する物理アドレス。
pub open spec fn payload_address(index: nat) -> u64 {
    (ENTRY_ADDRESS + 8 * index) as u64
}

/// 受理したペイロードの番号が指定する実 RAM の語。
pub open spec fn payload_word(accepted: Seq<u64>, index: nat) -> RamWord
    recommends index < accepted.len(),
{
    RamWord { address: payload_address(index), data: accepted[index as int] }
}

impl BootLoader {
    /// ヘッダーの語数。未受理なら0。
    pub closed spec fn total_model(&self) -> nat { self.total as nat }

    /// RAM へ確定した語の個数。
    pub closed spec fn committed_model(&self) -> nat { self.committed as nat }

    /// 受理した全ペイロード。検証でのみ保持する。
    pub closed spec fn accepted_model(&self) -> Seq<u64> { self.accepted@ }

    /// 現在のキュー先頭のアドレスを、直前まで駆動した tick 数。
    pub closed spec fn held_model(&self) -> u8 { self.held }

    /// 最初の受信違反。
    pub closed spec fn error_model(&self) -> Option<BootError> { self.error }

    /// 受理から書き込み確定までの順序と有限の容量を保つ。
    pub closed spec fn valid(&self) -> bool {
        &&& self.committed <= self.accepted@.len() <= self.total <= MAX_WORDS
        &&& self.fifo@ == self.accepted@.skip(self.committed as int)
        &&& self.fifo@.len() <= FIFO_WORDS
        &&& self.fifo@.len() == 0 ==> self.held == 0
        &&& self.held <= ADDRESS_SETUP + RAM_WRITE_LATENCY
    }

    /// CPU のポートへ切り替えてよい確定状態。
    pub open spec fn cpu_ready_model(&self) -> bool {
        self.error_model().is_none() && self.total_model() > 0
            && self.committed_model() == self.total_model()
    }

    /// 次の完成語まで保持できる追加ペイロードの個数。
    pub open spec fn credit_model(&self) -> nat {
        let pending = self.accepted_model().len() - self.committed_model();
        let remaining = self.total_model() - self.accepted_model().len();
        if self.error_model().is_some() || self.total_model() == 0 {
            0
        } else if FIFO_WORDS - pending <= remaining {
            (FIFO_WORDS - pending) as nat
        } else {
            remaining as nat
        }
    }

    /// ヘッダーを待つ状態に初期化する。
    pub fn reset() -> (loader: Self)
        ensures loader.valid(), loader.total_model() == 0, loader.committed_model() == 0,
            loader.accepted_model().len() == 0, loader.held_model() == 0, loader.error_model().is_none(),
    {
        let fifo = Vec::with_capacity(FIFO_WORDS);
        assert(Seq::<u64>::empty().skip(0) =~= Seq::<u64>::empty());
        Self { total: 0, committed: 0, fifo, held: 0, error: None, accepted: Ghost(Seq::empty()) }
    }

    /// 既に確定した語の個数を返す。
    pub fn committed_words(&self) -> (count: usize)
        ensures count == self.committed_model(),
    {
        self.committed
    }

    /// 最初に検出したエラーを返す。
    pub fn error(&self) -> (error: Option<BootError>)
        ensures error == self.error_model(),
    {
        self.error
    }

    /// 先頭から RAM へ書き込みながら、UART から完成語を受け取る。
    ///
    /// エラーが届いた tick から新しい許可を出さない。以前の許可を RAM がすでに受け取った
    /// 場合は固定レイテンシに従って確定を数える。CPU にはエラーのない全語確定後にポートを渡す。
    pub fn step(&mut self, input: Receive) -> (frame: BootFrame)
        requires old(self).valid(),
        ensures final(self).valid(), boot_step_matches(old(self), input, final(self), frame),
    {
        let accepted = self.accept(input);
        let ghost payload = self.accepted@;
        let count = self.fifo.len();
        let engaged = count > 0 && (self.error.is_none() || self.held > ADDRESS_SETUP);
        let (bus, write, committed) = if engaged {
            let address = ENTRY_ADDRESS + 8 * self.committed as u64;
            let word = RamWord { address, data: self.fifo[0] };
            let write = self.held == ADDRESS_SETUP;
            if self.held == ADDRESS_SETUP + RAM_WRITE_LATENCY {
                let ghost before = self.fifo@;
                self.fifo.remove(0);
                self.committed += 1;
                self.held = 0;
                proof {
                    assert(self.fifo@ =~= before.skip(1));
                    assert(before.skip(1) =~= payload.skip(self.committed as int));
                }
                (Some(word), false, Some(word))
            } else {
                self.held += 1;
                (Some(word), write, None)
            }
        } else {
            (None, false, None)
        };
        let remaining = self.total - self.committed - self.fifo.len();
        let credit = if self.error.is_some() || self.total == 0 {
            0
        } else {
            (FIFO_WORDS - self.fifo.len()).min(remaining)
        };
        BootFrame {
            bus,
            write,
            committed,
            cpu_ready: self.error.is_none() && self.total > 0 && self.committed == self.total,
            credit,
            accepted,
        }
    }

    fn accept(&mut self, input: Receive) -> (decision: Acceptance)
        requires old(self).valid(),
        ensures final(self).valid(), decision == input_decision(old(self), input),
            final(self).total_model() == received_total(old(self), input, decision),
            final(self).accepted_model() == received_words(old(self), input, decision),
            final(self).error_model() == received_error(old(self), decision),
            final(self).held_model() == old(self).held_model(),
            final(self).committed_model() == old(self).committed_model(),
    {
        if self.error.is_some() || (self.total > 0 && self.committed == self.total) {
            return Acceptance::None;
        }
        let word = match input {
            Receive::Idle => return Acceptance::None,
            Receive::Error => {
                self.error = Some(BootError::Receiver);
                return Acceptance::Rejected(BootError::Receiver);
            },
            Receive::Word(word) => word,
        };
        if self.total == 0 {
            if word == 0 || word > MAX_WORDS as u64 {
                self.error = Some(BootError::HeaderCount);
                return Acceptance::Rejected(BootError::HeaderCount);
            }
            self.total = word as usize;
            return Acceptance::Header;
        }
        if self.committed + self.fifo.len() >= self.total {
            self.error = Some(BootError::ExcessWord);
            return Acceptance::Rejected(BootError::ExcessWord);
        }
        if self.fifo.len() == FIFO_WORDS {
            self.error = Some(BootError::NoCredit);
            return Acceptance::Rejected(BootError::NoCredit);
        }
        let ghost old_fifo = self.fifo@;
        let ghost old_accepted = self.accepted@;
        self.fifo.push(word);
        proof {
            self.accepted = Ghost(old_accepted.push(word));
            assert(self.fifo@ =~= self.accepted@.skip(self.committed as int)) by {
                assert forall|i: int| 0 <= i < old_fifo.len() implies
                    (#[trigger] self.fifo@[i]) == self.accepted@.skip(self.committed as int)[i] by {
                    assert(self.fifo@[i] == old_fifo[i]);
                    assert(old_accepted[i + self.committed] == self.accepted@[i + self.committed]);
                };
            };
        }
        Acceptance::Payload
    }
}

/// 既に届いた語と空き容量に従い、この tick の入力の扱いを定める。
pub open spec fn input_decision(before: &BootLoader, input: Receive) -> Acceptance {
    select_acceptance(before.total_model(), before.committed_model(),
        (before.accepted_model().len() - before.committed_model()) as nat,
        before.error_model().is_some(), input)
}

/// 受信処理後のヘッダー語数。
pub open spec fn received_total(before: &BootLoader, input: Receive, decision: Acceptance) -> nat {
    match (decision, input) {
        (Acceptance::Header, Receive::Word(word)) => word as nat,
        _ => before.total_model(),
    }
}

/// 受信処理後のペイロード列。
pub open spec fn received_words(before: &BootLoader, input: Receive, decision: Acceptance) -> Seq<u64> {
    match (decision, input) {
        (Acceptance::Payload, Receive::Word(word)) => before.accepted_model().push(word),
        _ => before.accepted_model(),
    }
}

/// 受信処理後のエラー。最初の原因を保持する。
pub open spec fn received_error(before: &BootLoader, decision: Acceptance) -> Option<BootError> {
    match decision {
        Acceptance::Rejected(error) => Some(error),
        _ => before.error_model(),
    }
}

/// 受信、FIFO 先頭のアドレス保持、RAM の確定とポート移譲を規定する。
pub open spec fn boot_step_matches(before: &BootLoader, input: Receive, after: &BootLoader, frame: BootFrame)
    -> bool {
    let decision = input_decision(before, input);
    let words = received_words(before, input, decision);
    let error = received_error(before, decision);
    let index = before.committed_model();
    let held = before.held_model();
    let engaged = index < words.len() && (error.is_none() || held > ADDRESS_SETUP);
    let commits = engaged && held == ADDRESS_SETUP + RAM_WRITE_LATENCY;
    &&& frame.accepted == decision
    &&& after.total_model() == received_total(before, input, decision)
    &&& after.accepted_model() == words
    &&& after.error_model() == error
    &&& frame.bus == if engaged { Some(payload_word(words, index)) } else { None }
    &&& frame.write == (engaged && held == ADDRESS_SETUP)
    &&& frame.committed == if commits { Some(payload_word(words, index)) } else { None }
    &&& after.committed_model() == index + if commits { 1nat } else { 0nat }
    &&& after.held_model() == if commits { 0int } else if engaged { held + 1 } else { held as int }
    &&& frame.cpu_ready == after.cpu_ready_model()
    &&& frame.credit == after.credit_model()
}

/// reset から、受信と RAM ポートを同じ tick 順に処理した履歴。
pub open spec fn boot_trace(
    states: Seq<BootLoader>, inputs: Seq<Receive>, frames: Seq<BootFrame>,
) -> bool {
    &&& states.len() == inputs.len() + 1
    &&& inputs.len() == frames.len()
    &&& states[0].total_model() == 0
    &&& states[0].held_model() == 0
    &&& states[0].error_model().is_none()
    &&& forall|tick: int| 0 <= tick < states.len() ==> (#[trigger] states[tick]).valid()
    &&& forall|tick: int| 0 <= tick < frames.len() ==>
        boot_step_matches(&states[tick], inputs[tick], &states[tick + 1], #[trigger] frames[tick])
}

proof fn held_follows_drive(before: &BootLoader, input: Receive, after: &BootLoader, frame: BootFrame)
    requires before.valid(), after.valid(), boot_step_matches(before, input, after, frame),
        0 < after.held_model(),
        after.error_model().is_none() || after.held_model() > ADDRESS_SETUP,
    ensures before.committed_model() == after.committed_model(),
        after.held_model() == before.held_model() + 1,
        frame.bus == Some(payload_word(after.accepted_model(), after.committed_model())),
        after.error_model().is_none() ==> before.error_model().is_none(),
{
    if frame.bus.is_none() {
        if after.error_model().is_none() {
            assert(after.committed_model() == after.accepted_model().len());
            assert(after.held_model() == 0);
        } else {
            assert(before.held_model() == after.held_model());
            assert(before.held_model() > ADDRESS_SETUP);
            assert(after.committed_model() == after.accepted_model().len());
            assert(after.held_model() == 0);
        }
    }
}

proof fn held_word(
    states: Seq<BootLoader>, inputs: Seq<Receive>, frames: Seq<BootFrame>, until: int, ago: nat,
)
    requires boot_trace(states, inputs, frames), 0 <= until < states.len(),
        states[until].error_model().is_none(), 0 < ago <= states[until].held_model(),
    ensures ago <= until && frames[until - ago].bus
        == Some(payload_word(states[until].accepted_model(), states[until].committed_model())),
    decreases ago,
{
    let before = &states[until - 1];
    let after = &states[until];
    assert(boot_step_matches(before, inputs[until - 1], after, frames[until - 1]));
    held_follows_drive(before, inputs[until - 1], after, frames[until - 1]);
    if ago > 1 {
        held_word(states, inputs, frames, until - 1, (ago - 1) as nat);
        let index = after.committed_model();
        assert(index < before.accepted_model().len());
        assert(before.accepted_model()[index as int] == after.accepted_model()[index as int]);
    }
}

/// 書き込みを許可したアドレスとデータは、直前の32 tick すべてで駆動されていた。
pub proof fn uart_write_setup(
    states: Seq<BootLoader>, inputs: Seq<Receive>, frames: Seq<BootFrame>, tick: int,
)
    requires boot_trace(states, inputs, frames), 0 <= tick < frames.len(), frames[tick].write,
    ensures
        ADDRESS_SETUP <= tick,
        frames[tick].bus.is_some(),
        forall|prior: int| tick - ADDRESS_SETUP <= prior < tick ==>
            (#[trigger] frames[prior]).bus == frames[tick].bus,
{
    let before = &states[tick];
    let after = &states[tick + 1];
    assert(boot_step_matches(before, inputs[tick], after, frames[tick]));
    held_word(states, inputs, frames, tick, ADDRESS_SETUP as nat);
    assert forall|prior: int| tick - ADDRESS_SETUP <= prior < tick implies
        (#[trigger] frames[prior]).bus == frames[tick].bus by {
        held_word(states, inputs, frames, tick, (tick - prior) as nat);
        let index = before.committed_model();
        assert(index < before.accepted_model().len());
        assert(before.accepted_model()[index as int] == after.accepted_model()[index as int]);
    };
}

/// RAM の確定は、2 tick 前に許可した同じアドレスとデータに対応する。
pub proof fn uart_commit_has_request(
    states: Seq<BootLoader>, inputs: Seq<Receive>, frames: Seq<BootFrame>, tick: int,
)
    requires boot_trace(states, inputs, frames), 0 <= tick < frames.len(),
        frames[tick].committed.is_some(),
    ensures RAM_WRITE_LATENCY <= tick,
        frames[tick - RAM_WRITE_LATENCY].write,
        frames[tick - RAM_WRITE_LATENCY].bus == frames[tick].committed,
{
    let state = &states[tick];
    assert(boot_step_matches(state, inputs[tick], &states[tick + 1], frames[tick]));
    assert(state.held_model() == ADDRESS_SETUP + RAM_WRITE_LATENCY);
    let previous = &states[tick - 1];
    assert(boot_step_matches(previous, inputs[tick - 1], state, frames[tick - 1]));
    held_follows_drive(previous, inputs[tick - 1], state, frames[tick - 1]);
    assert(previous.held_model() == ADDRESS_SETUP + 1);
    let request_state = &states[tick - 2];
    let request = frames[tick - 2];
    assert(boot_step_matches(request_state, inputs[tick - 2], previous, request));
    held_follows_drive(request_state, inputs[tick - 2], previous, request);
    assert(request_state.held_model() == ADDRESS_SETUP);
    let index = state.committed_model();
    assert(index < previous.accepted_model().len());
    assert(index < state.accepted_model().len());
    assert(previous.accepted_model()[index as int] == state.accepted_model()[index as int]);
    assert(state.accepted_model()[index as int] == states[tick + 1].accepted_model()[index as int]);
}

/// ポートに出した書き込み許可は、後続の受信結果によらず2 tick 後に確定する。
pub proof fn uart_request_commits(
    states: Seq<BootLoader>, inputs: Seq<Receive>, frames: Seq<BootFrame>, tick: int,
)
    requires boot_trace(states, inputs, frames), 0 <= tick, tick + RAM_WRITE_LATENCY < frames.len(),
        frames[tick].write,
    ensures frames[tick + RAM_WRITE_LATENCY].committed == frames[tick].bus,
{
    let before = &states[tick];
    let issued = &states[tick + 1];
    let pending = &states[tick + 2];
    let index = before.committed_model();
    assert(boot_step_matches(before, inputs[tick], issued, frames[tick]));
    assert(index < issued.accepted_model().len());
    assert(issued.held_model() == ADDRESS_SETUP + 1);
    assert(boot_step_matches(issued, inputs[tick + 1], pending, frames[tick + 1]));
    assert(pending.held_model() == ADDRESS_SETUP + RAM_WRITE_LATENCY);
    assert(index < pending.accepted_model().len());
    assert(issued.accepted_model()[index as int] == pending.accepted_model()[index as int]);
    let written = &states[tick + 3];
    assert(boot_step_matches(pending, inputs[tick + 2], written, frames[tick + 2]));
    assert(pending.accepted_model()[index as int] == written.accepted_model()[index as int]);
}

/// エラー後の発行を止め、CPU への移譲に全ペイロードの確定を要求する。
pub proof fn boot_frame_safety(before: &BootLoader, input: Receive, after: &BootLoader, frame: BootFrame)
    requires before.valid(), after.valid(), boot_step_matches(before, input, after, frame),
    ensures
        after.error_model().is_some() ==> !frame.write && !frame.cpu_ready,
        frame.cpu_ready ==> after.error_model().is_none()
            && 1 <= after.total_model() <= MAX_WORDS
            && after.total_model() == after.committed_model() == after.accepted_model().len(),
        frame.write ==> frame.bus.is_some() && before.committed_model() < after.accepted_model().len(),
{
}

/// ポートに新しい入力が来なければ、残りの書き込みを確定するまでに要する tick 数。
pub open spec fn boot_work_remaining(state: &BootLoader) -> int {
    (state.accepted_model().len() - state.committed_model())
        * (ADDRESS_SETUP + RAM_WRITE_LATENCY + 1) - state.held_model()
}

proof fn boot_work_bound(state: &BootLoader)
    requires state.valid(),
    ensures 0 <= boot_work_remaining(state) <= FIFO_DRAIN_TICKS,
        (boot_work_remaining(state) == 0) <==> state.committed_model() == state.accepted_model().len(),
{
    let pending = state.accepted_model().len() - state.committed_model();
    assert(FIFO_DRAIN_TICKS == 70) by (compute);
    assert(ADDRESS_SETUP + RAM_WRITE_LATENCY + 1 == 35) by (compute);
    assert(pending == state.fifo@.len());
    assert(0 <= pending <= FIFO_WORDS);
    assert(boot_work_remaining(state) == pending * 35 - state.held_model());
    if pending == 0 {
        assert(state.held_model() == 0);
    } else {
        assert(1 <= pending);
        assert(state.held_model() <= 34);
        assert(35 <= 35 * pending <= 70);
    }
}

proof fn idle_drains_one_tick(before: &BootLoader, after: &BootLoader, frame: BootFrame)
    requires before.valid(), after.valid(), after.error_model().is_none(),
        boot_step_matches(before, Receive::Idle, after, frame),
    ensures before.error_model().is_none(),
        0 <= boot_work_remaining(after) <= FIFO_DRAIN_TICKS,
        boot_work_remaining(after) == if boot_work_remaining(before) > 0 {
            boot_work_remaining(before) - 1
        } else {
            0
        },
{
    boot_work_bound(after);
    let pending = after.accepted_model().len() - after.committed_model();
    let old_pending = before.accepted_model().len() - before.committed_model();
    assert(ADDRESS_SETUP + RAM_WRITE_LATENCY + 1 == 35) by (compute);
    assert(pending == after.fifo@.len());
    assert(old_pending == before.fifo@.len());
    assert(boot_work_remaining(after) == pending * 35 - after.held_model());
    assert(boot_work_remaining(before) == old_pending * 35 - before.held_model());
    if old_pending == 0 {
        assert(before.held_model() == 0);
        assert(pending == 0 && after.held_model() == 0);
    } else {
        assert(1 <= old_pending);
        assert(boot_work_remaining(before) > 0);
        assert(frame.bus.is_some());
        if before.held_model() == 34 {
            assert(pending == old_pending - 1 && after.held_model() == 0);
        } else {
            assert(pending == old_pending && after.held_model() == before.held_model() + 1);
        }
    }
}

proof fn idle_drains_until(
    states: Seq<BootLoader>, inputs: Seq<Receive>, frames: Seq<BootFrame>, start: int, end: int,
)
    requires boot_trace(states, inputs, frames), 0 <= start <= end < states.len(),
        states[end].error_model().is_none(),
        forall|tick: int| start <= tick < end ==> #[trigger] inputs[tick] == Receive::Idle,
    ensures 0 <= boot_work_remaining(&states[end]),
        boot_work_remaining(&states[end]) <= if FIFO_DRAIN_TICKS > end - start {
            FIFO_DRAIN_TICKS - (end - start)
        } else {
            0
        },
    decreases end - start,
{
    if start == end {
        boot_work_bound(&states[end]);
    } else {
        assert(boot_step_matches(&states[end - 1], inputs[end - 1], &states[end], frames[end - 1]));
        idle_drains_one_tick(&states[end - 1], &states[end], frames[end - 1]);
        idle_drains_until(states, inputs, frames, start, end - 1);
        if boot_work_remaining(&states[end - 1]) > 0 {
            assert(FIFO_DRAIN_TICKS > end - 1 - start);
        }
    }
}

/// 十分に受信間隔を空けてエラーが届けば、先行する書き込みは既に確定している。
///
/// UART 受信側との接続では、完成語または受信エラーを通知した tick の間隔を照合する。
pub proof fn idle_before_error_has_no_pending_write(
    states: Seq<BootLoader>, inputs: Seq<Receive>, frames: Seq<BootFrame>, tick: int,
)
    requires boot_trace(states, inputs, frames), FIFO_DRAIN_TICKS <= tick < frames.len(),
        states[tick].error_model().is_none(),
        inputs[tick] == Receive::Error,
        forall|i: int| tick - FIFO_DRAIN_TICKS <= i < tick ==> #[trigger] inputs[i] == Receive::Idle,
    ensures !frames[tick].write && frames[tick].committed.is_none(),
{
    idle_drains_until(states, inputs, frames, tick - FIFO_DRAIN_TICKS, tick);
    let before = &states[tick];
    boot_work_bound(before);
    assert(before.committed_model() == before.accepted_model().len());
    assert(boot_step_matches(before, inputs[tick], &states[tick + 1], frames[tick]));
}

/// 先頭語の8バイト全体がアプリ領域内にあり、語ごとの位置が異なる。
pub proof fn payload_address_bound(index: nat, later: nat)
    requires index <= later < MAX_WORDS,
    ensures
        ENTRY_ADDRESS <= payload_address(index),
        payload_address(later) <= 0x51ff8,
        index < later ==> payload_address(index) + 8 <= payload_address(later),
{
}

} // verus!
