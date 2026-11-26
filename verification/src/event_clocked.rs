//! 入力や状態が変わった素子だけを、同期回路の tick に従って再評価する。

use circuitgame2_core::netlist::{ClockedVm, TickInputError, Wire};
use ncg2_event_queue::{
    EvaluationQueue,
    delay_schedule::{DelayInputEdge, idle_prefix, schedule_delay_edge},
    fanout::{Pin, ReverseDependencies, StoragePins},
    nor_inputs::PackedNorInputs,
};
use std::{cmp::Reverse, collections::BinaryHeap, ops::Range};

fn pin_for_wire(wire: Wire, augmented: usize) -> Pin {
    match wire {
        Wire::Input(index) => Pin::Slot(index),
        Wire::Gate(index) => Pin::Slot(augmented + index),
        Wire::Constant(value) => Pin::Constant(value),
    }
}

struct Storage {
    output: Range<usize>,
    pins: StoragePins,
    ram: Option<Vec<bool>>,
}

impl Storage {
    fn delay_stage(&self) -> bool {
        self.ram.is_none()
            && self.output.len() == 1
            && self.pins.data.len() == 1
            && matches!(self.pins.enable, Pin::Constant(true))
    }
}

/// 先頭段が読み取った値を、末尾の配線へ届ける。
#[derive(Clone, Copy)]
struct DelayRoute {
    stages: usize,
    output: usize,
}

struct LatchDelays {
    tick: u64,
    eliminated: Vec<bool>,
    source_rows: Vec<usize>,
    routes: Vec<Vec<DelayRoute>>,
    initial: Vec<DelayRoute>,
    pending: BinaryHeap<Reverse<(u64, usize, bool)>>,
}

impl LatchDelays {
    // 予定を相対時刻に戻す境界。鎖の長さもここまでに制限する。
    const REBASE_TICK: u64 = u64::MAX / 2;

    fn advance(&mut self) {
        if self.tick == Self::REBASE_TICK {
            // 確定済みの予定は除去されているため、残りはすべて現在より先にある。
            let mut pending = std::mem::take(&mut self.pending).into_vec();
            for Reverse((tick, _, _)) in &mut pending {
                *tick -= self.tick;
            }
            self.pending = BinaryHeap::from(pending);
            self.tick = 0;
        }
        self.tick += 1;
    }

    fn reset(&mut self) {
        self.tick = 0;
        self.pending.clear();
        for &route in &self.initial {
            Self::schedule(&mut self.pending, route, DelayInputEdge::External { tick: 1, value: true });
        }
    }

    fn notify(&mut self, slot: usize, value: bool, external: bool) {
        let edge = if external {
            DelayInputEdge::External { tick: self.tick, value }
        } else {
            DelayInputEdge::Committed { tick: self.tick, value }
        };
        for &route in &self.routes[self.source_rows[slot]] {
            Self::schedule(&mut self.pending, route, edge);
        }
    }

    fn schedule(
        pending: &mut BinaryHeap<Reverse<(u64, usize, bool)>>,
        route: DelayRoute,
        edge: DelayInputEdge,
    ) {
        // 各鎖は2段以上で、現在時刻と段数の和は u64 の範囲に収まる。
        let event = schedule_delay_edge(route.stages, edge, u64::MAX)
            .expect("遅延鎖の到着時刻は現在の時刻区間に収まる");
        pending.push(Reverse((event.output_tick, route.output, event.value)));
    }

    fn advance_idle(&mut self, limit: u64) -> u64 {
        let next = self.pending.peek().map(|&Reverse((tick, _, _))| tick);
        let span = idle_prefix(self.tick, Self::REBASE_TICK, next, limit);
        self.tick += span;
        span
    }

    fn due(&mut self, changes: &mut Vec<(usize, bool)>) {
        while let Some(&Reverse((tick, slot, value))) = self.pending.peek() {
            if tick != self.tick {
                break;
            }
            self.pending.pop();
            changes.push((slot, value));
        }
    }
}

/// Core の展開済み配線で、変化のある素子を再評価する検証用の実行器。
///
/// [`ClockedVm`] と同じ順に外部入力と公開出力のビットを並べる。
/// 構築時に配線を所有し、VM 本体から借用を外す。tick の処理に使う領域は構築時に確保する。
pub struct EventVm {
    external: usize,
    augmented: usize,
    nor: PackedNorInputs,
    storage: Vec<Storage>,
    users: ReverseDependencies,
    queue: EvaluationQueue,
    changes: Vec<(usize, bool)>,
    ram_writes: Vec<(usize, usize)>,
    write_data: Vec<bool>,
    reset_output: Vec<bool>,
    output: Vec<bool>,
    evaluated: usize,
    delays: Option<LatchDelays>,
}

impl EventVm {
    /// 展開済みの配線を受け取り、reset 直後の状態を作る。
    ///
    /// 渡した VM が進行済みでも、記憶内容や出力の現在値は引き継がない。
    ///
    /// # Errors
    ///
    /// 外部入力、状態素子の出力区間または RAM の容量が展開済みの配線と整合しない場合や、定数を含む配線の添字を表せない場合に返す。
    pub fn new(vm: &ClockedVm) -> Result<Self, &'static str> {
        let net = vm.netlist();
        let external = vm.external_input_count();
        let augmented = net.input_count;
        let slots = augmented
            .checked_add(net.gates.len())
            .ok_or("配線の添字が範囲外。")?;
        let nodes = net
            .gates
            .len()
            .checked_add(vm.storage_count())
            .ok_or("素子の添字が範囲外。")?;
        let pin = |wire| pin_for_wire(wire, augmented);
        let gates: Vec<_> = net
            .gates
            .iter()
            .map(|gate| (pin(gate.left), pin(gate.right)))
            .collect();
        let mut storage = Vec::with_capacity(vm.storage_count());
        let mut next = external;
        for index in 0..vm.storage_count() {
            let (wiring, output) = vm.storage_wiring(index).ok_or("状態素子の配線がない。")?;
            if output.start != next
                || output.end < output.start
                || output.end > augmented
                || output.len() != wiring.data.len()
            {
                return Err("状態素子の出力区間が増補入力の配置と異なる。");
            }
            next = output.end;
            let (address, ram) = if let Some(address) = &wiring.address {
                let bits = vm.ram_memory(index).ok_or("RAM に記憶領域がない。")?.len();
                let expected = u32::try_from(address.len())
                    .ok()
                    .and_then(|width| 1usize.checked_shl(width))
                    .and_then(|words| words.checked_mul(output.len()));
                if expected != Some(bits) {
                    return Err("RAM の容量がアドレス幅と語幅に合わない。");
                }
                (
                    address.iter().copied().map(pin).collect(),
                    Some(vec![false; bits]),
                )
            } else {
                (Vec::new(), None)
            };
            storage.push(Storage {
                output,
                pins: StoragePins {
                    address,
                    data: wiring.data.iter().copied().map(pin).collect(),
                    enable: pin(wiring.enable),
                },
                ram,
            });
        }
        if next != augmented {
            return Err("外部入力と状態素子で増補入力の全域を覆わない。");
        }

        let storage_pins: Vec<_> = storage.iter().map(|device| &device.pins).collect();
        let outputs: Vec<_> = net.outputs.iter().copied().map(pin).collect();
        let users = ReverseDependencies::build(slots, &gates, &storage_pins, &outputs)?;
        let reset_output: Vec<_> = outputs
            .iter()
            .map(|pin| matches!(pin, Pin::Constant(true)))
            .collect();
        let output = reset_output.clone();
        let nor = PackedNorInputs::build(&gates, slots).ok_or("NOR 入力の添字を表せない。")?;
        Ok(Self {
            external,
            augmented,
            nor,
            storage,
            users,
            queue: EvaluationQueue::new_all(nodes),
            changes: Vec::with_capacity(slots),
            ram_writes: Vec::with_capacity(vm.storage_count()),
            write_data: vec![false; augmented],
            reset_output,
            output,
            evaluated: 0,
            delays: None,
        })
    }

    /// 内部段を観測できないラッチ鎖を、末尾の到着時刻で実行する。
    ///
    /// 初期値は [`Self::new`] と同じ。評価通知と評価数には省いたラッチを含まない。
    ///
    /// # Errors
    ///
    /// [`Self::new`] の構築条件を満たさない場合、または鎖の段数が実行器の時刻区間を超える場合に返す。
    pub fn new_with_latch_delays(vm: &ClockedVm) -> Result<Self, &'static str> {
        let mut event = Self::new(vm)?;
        let chains = event.linear_latch_chains();
        if chains.is_empty() {
            return Ok(event);
        }

        let mut delays = LatchDelays {
            tick: 0,
            eliminated: vec![false; event.storage.len()],
            source_rows: vec![0; event.nor.read_only().len()],
            routes: vec![Vec::new()],
            initial: Vec::new(),
            pending: BinaryHeap::with_capacity(event.storage.len()),
        };
        for chain in chains {
            if chain.len() as u64 > LatchDelays::REBASE_TICK {
                return Err("遅延鎖の段数が時刻区間を超える。");
            }
            let first = chain[0];
            let last = chain[chain.len() - 1];
            let route = DelayRoute { stages: chain.len(), output: event.storage[last].output.start };
            for index in chain {
                delays.eliminated[index] = true;
            }
            match event.storage[first].pins.data[0] {
                Pin::Slot(slot) => {
                    let row = &mut delays.source_rows[slot];
                    if *row == 0 {
                        *row = delays.routes.len();
                        delays.routes.push(Vec::new());
                    }
                    delays.routes[*row].push(route);
                }
                Pin::Constant(true) => delays.initial.push(route),
                Pin::Constant(false) => {}
            }
        }
        delays.reset();
        event.delays = Some(delays);
        Ok(event)
    }

    /// すべての内部状態と外部入力を0にし、初回の評価と遅延鎖の初期予定を用意する。
    pub fn reset(&mut self) -> &[bool] {
        self.nor.reset();
        for device in &mut self.storage {
            if let Some(ram) = &mut device.ram {
                ram.fill(false);
            }
        }
        self.queue.reset_all();
        if let Some(delays) = &mut self.delays {
            delays.reset();
        }
        self.output.copy_from_slice(&self.reset_output);
        self.evaluated = 0;
        &self.output
    }

    /// 直近の tick で確定した公開出力を借りる。
    pub fn output(&self) -> &[bool] {
        &self.output
    }

    /// 状態素子の添字から、直近の tick で確定した RAM の全記憶内容を借りる。
    ///
    /// 語はアドレスの昇順、語内のビットは最下位から並ぶ。ラッチと範囲外の添字は `None` を返す。
    pub fn ram_memory(&self, index: usize) -> Option<&[bool]> {
        self.storage.get(index)?.ram.as_deref()
    }

    /// 毎 tick 全体を走査する場合に訪れる NOR と状態素子の数。
    pub fn node_count(&self) -> usize {
        self.queue.node_count()
    }

    /// 直近の成功した tick で評価した素子の数。reset の直後は0。
    pub fn evaluated_last_tick(&self) -> usize {
        self.evaluated
    }

    /// 内部段に分岐や公開出力がない、常時有効な1ビットラッチ列を探す。
    ///
    /// 先頭から末尾の順に状態素子の添字を返す。閉じた環と単独の素子は含めない。
    pub fn linear_latch_chains(&self) -> Vec<Vec<usize>> {
        let mut next = vec![None; self.storage.len()];
        let mut has_previous = vec![false; self.storage.len()];
        let gate_count = self.nor.gate_count();
        for (index, stage) in self.storage.iter().enumerate() {
            if !stage.delay_stage() {
                continue;
            }
            let slot = stage.output.start;
            let Some(([reader], [])) = self.users.readers(slot) else {
                continue;
            };
            let Some(successor) = reader.checked_sub(gate_count) else {
                continue;
            };
            let Some(device) = self.storage.get(successor) else {
                continue;
            };
            if device.delay_stage() && matches!(device.pins.data[0], Pin::Slot(input) if input == slot) {
                next[index] = Some(successor);
                has_previous[successor] = true;
            }
        }
        let mut chains = Vec::new();
        for (root, successor) in next.iter().enumerate() {
            if has_previous[root] || successor.is_none() {
                continue;
            }
            let mut chain = vec![root];
            let mut current = root;
            while let Some(successor) = next[current] {
                chain.push(successor);
                current = successor;
            }
            chains.push(chain);
        }
        chains
    }

    /// 外部入力を適用し、1 tick 後の公開出力を借りる。
    ///
    /// 外部入力の変更は同じ tick の更新に使う。素子の出力は全候補を評価した後で確定する。
    ///
    /// # Errors
    ///
    /// 外部入力の幅が違う場合は、状態を変更せず `TickInputError::InputCount` を返す。
    pub fn step(&mut self, input: &[bool]) -> Result<&[bool], TickInputError> {
        self.step_with_evaluation(input, |_| {})
    }

    /// 評価候補と末尾の更新予定がない範囲で、同じ外部入力の tick をまとめて進める。
    ///
    /// 進めた tick 数を返す。入力が変わる場合、幅が合わない場合、または
    /// ラッチ鎖の予定を使わない実行器では0を返す。進めた間は公開出力と
    /// RAM が現在の値を保ち、直近の評価数も0になる。
    pub fn advance_idle(&mut self, input: &[bool], limit: u64) -> u64 {
        if self.queue.pending_count() != 0 || input != &self.nor.read_only()[..self.external] {
            return 0;
        }
        let Some(delays) = &mut self.delays else {
            return 0;
        };
        let span = delays.advance_idle(limit);
        if span != 0 {
            self.evaluated = 0;
        }
        span
    }

    /// 1 tick 進め、評価する素子の添字を評価順に通知する。
    ///
    /// 添字は NOR を先に並べ、その後に状態素子を並べる。
    ///
    /// # Errors
    ///
    /// 入力の幅が異なる場合は、状態を変更せず一件も通知しない。
    pub fn step_with_evaluation(
        &mut self,
        input: &[bool],
        mut evaluate: impl FnMut(usize),
    ) -> Result<&[bool], TickInputError> {
        if input.len() != self.external {
            return Err(TickInputError::InputCount);
        }
        if let Some(delays) = &mut self.delays {
            delays.advance();
        }
        for (slot, &value) in input.iter().enumerate() {
            self.publish(slot, value);
        }

        self.evaluated = self.queue.pending_count();
        while let Some(node) = self.queue.pop() {
            if node < self.nor.gate_count() {
                evaluate(node);
                let value = self.nor.next(node);
                let slot = self.augmented + node;
                if self.nor.read_only()[slot] != value {
                    self.changes.push((slot, value));
                }
            } else {
                let index = node - self.nor.gate_count();
                if self.delays.as_ref().is_some_and(|delays| delays.eliminated[index]) {
                    self.evaluated -= 1;
                    continue;
                }
                evaluate(node);
                self.evaluate_storage(index);
            }
        }
        if let Some(delays) = &mut self.delays {
            delays.due(&mut self.changes);
        }

        // ここから登録出力を確定する。依存先は次の tick で再評価する。
        for i in 0..self.changes.len() {
            let (slot, value) = self.changes[i];
            self.publish(slot, value);
        }
        self.changes.clear();
        for &(index, start) in &self.ram_writes {
            let device = &mut self.storage[index];
            let ram = device.ram.as_mut().expect("書き込み候補は RAM だけ");
            let word = &self.write_data[device.output.clone()];
            ram[start..start + word.len()].copy_from_slice(word);

            // 同じ入力のままでも、次の read-first 読み出しは新しい語を読む。
            let node = self.nor.gate_count() + index;
            self.queue.insert(node);
        }
        self.ram_writes.clear();
        Ok(&self.output)
    }

    fn publish(&mut self, slot: usize, value: bool) {
        if !self.nor.set(slot, value) {
            return;
        }
        let (nodes, outputs) = self
            .users
            .readers(slot)
            .expect("値の列と逆引き表は同じ配線番号を使う");
        if let Some(delays) = self.delays.as_mut().filter(|delays| delays.source_rows[slot] != 0) {
            let gates = self.nor.gate_count();
            for &node in nodes {
                // 遅延鎖の先頭への通知は、評価キューに入れず末尾の予定へ反映する。
                if node < gates || !delays.eliminated[node - gates] {
                    self.queue.insert(node);
                }
            }
            delays.notify(slot, value, slot < self.external);
        } else {
            self.queue.insert_all(nodes);
        }
        for &target in outputs {
            self.output[target] = value;
        }
    }

    fn evaluate_storage(&mut self, index: usize) {
        let device = &self.storage[index];
        let enabled = device.pins.enable.read(self.nor.read_only());
        if let Some(ram) = &device.ram {
            let mut word = 0;
            for (bit, &pin) in device.pins.address.iter().enumerate() {
                word |= usize::from(pin.read(self.nor.read_only())) << bit;
            }
            let start = word * device.output.len();
            let read = &ram[start..start + device.output.len()];
            for (slot, &value) in device.output.clone().zip(read) {
                if self.nor.read_only()[slot] != value {
                    self.changes.push((slot, value));
                }
            }
            if enabled {
                let mut changed = false;
                for ((slot, &pin), &old) in device.output.clone().zip(&device.pins.data).zip(read) {
                    let value = pin.read(self.nor.read_only());
                    changed |= value != old;
                    self.write_data[slot] = value;
                }
                if changed {
                    self.ram_writes.push((index, start));
                }
            }
        } else if enabled {
            for (slot, &pin) in device.output.clone().zip(&device.pins.data) {
                let value = pin.read(self.nor.read_only());
                if self.nor.read_only()[slot] != value {
                    self.changes.push((slot, value));
                }
            }
        }
    }
}

#[cfg(test)]
mod latch_delay_tests {
    use super::*;

    #[test]
    fn rebasing_keeps_pending_updates_in_their_original_order() {
        let boundary = LatchDelays::REBASE_TICK;
        let mut delays = LatchDelays {
            tick: boundary,
            eliminated: Vec::new(),
            source_rows: Vec::new(),
            routes: Vec::new(),
            initial: Vec::new(),
            pending: BinaryHeap::from([Reverse((boundary + 3, 1, true)), Reverse((boundary + 1, 2, false))]),
        };
        let mut changes = Vec::new();
        delays.advance();
        delays.due(&mut changes);
        assert_eq!(changes, [(2, false)]);
        delays.advance();
        delays.due(&mut changes);
        assert_eq!(changes, [(2, false)]);
        delays.advance();
        delays.due(&mut changes);
        assert_eq!(changes, [(2, false), (1, true)]);
    }
}
