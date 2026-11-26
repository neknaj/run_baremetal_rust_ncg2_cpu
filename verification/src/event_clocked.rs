//! 入力や状態が変わった素子だけを、同期回路の tick に従って再評価する。

use circuitgame2_core::netlist::{ClockedVm, TickInputError, Wire};
use ncg2_event_queue::EvaluationQueue;
use std::ops::Range;

#[derive(Clone, Copy)]
enum Pin {
    Slot(usize),
    Constant(bool),
}

impl Pin {
    fn from_wire(wire: Wire, augmented: usize) -> Self {
        match wire {
            Wire::Input(index) => Self::Slot(index),
            Wire::Gate(index) => Self::Slot(augmented + index),
            Wire::Constant(value) => Self::Constant(value),
        }
    }

    fn read(self, values: &[bool]) -> bool {
        match self {
            Self::Slot(index) => values[index],
            Self::Constant(value) => value,
        }
    }

    fn connect(self, users: &mut [Vec<usize>], target: usize) {
        if let Self::Slot(index) = self {
            let uses = &mut users[index];
            if uses.last() != Some(&target) {
                uses.push(target);
            }
        }
    }
}

struct Ram {
    address: Vec<Pin>,
    bits: Vec<bool>,
}

struct Storage {
    output: Range<usize>,
    data: Vec<Pin>,
    enable: Pin,
    ram: Option<Ram>,
}

/// Core の展開済み配線で、変化のある素子を再評価する検証用の実行器。
///
/// [`ClockedVm`] と同じ順に外部入力と公開出力のビットを並べる。
/// 構築時に配線を所有し、VM 本体から借用を外す。tick の処理に使う領域は構築時に確保する。
pub struct EventVm {
    external: usize,
    augmented: usize,
    gates: Vec<(Pin, Pin)>,
    storage: Vec<Storage>,
    // 増補入力、NOR の登録出力の順に配置する。
    values: Vec<bool>,
    // 素子の添字に続けて公開出力の添字を割り当てる。
    users: Vec<Vec<usize>>,
    dependent_nodes: Vec<usize>,
    queue: EvaluationQueue,
    changes: Vec<(usize, bool)>,
    ram_writes: Vec<(usize, usize)>,
    write_data: Vec<bool>,
    reset_output: Vec<bool>,
    output: Vec<bool>,
    evaluated: usize,
}

impl EventVm {
    /// 展開済みの配線を受け取り、reset 直後の状態を作る。
    ///
    /// 渡した VM が進行済みでも、記憶内容や出力の現在値は引き継がない。
    ///
    /// # Errors
    ///
    /// 外部入力、状態素子の出力区間または RAM の容量が展開済みの配線と整合しない場合に返す。
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
        nodes
            .checked_add(net.outputs.len())
            .ok_or("公開出力の添字が範囲外。")?;
        let pin = |wire| Pin::from_wire(wire, augmented);
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
            let ram = if let Some(address) = &wiring.address {
                let bits = vm.ram_memory(index).ok_or("RAM に記憶領域がない。")?.len();
                let expected = u32::try_from(address.len())
                    .ok()
                    .and_then(|width| 1usize.checked_shl(width))
                    .and_then(|words| words.checked_mul(output.len()));
                if expected != Some(bits) {
                    return Err("RAM の容量がアドレス幅と語幅に合わない。");
                }
                Some(Ram {
                    address: address.iter().copied().map(pin).collect(),
                    bits: vec![false; bits],
                })
            } else {
                None
            };
            storage.push(Storage {
                output,
                data: wiring.data.iter().copied().map(pin).collect(),
                enable: pin(wiring.enable),
                ram,
            });
        }
        if next != augmented {
            return Err("外部入力と状態素子で増補入力の全域を覆わない。");
        }

        let mut users = vec![Vec::new(); slots];
        for (index, &(left, right)) in gates.iter().enumerate() {
            left.connect(&mut users, index);
            right.connect(&mut users, index);
        }
        for (index, device) in storage.iter().enumerate() {
            let node = gates.len() + index;
            device.enable.connect(&mut users, node);
            for &data in &device.data {
                data.connect(&mut users, node);
            }
            if let Some(ram) = &device.ram {
                for &address in &ram.address {
                    address.connect(&mut users, node);
                }
            }
        }
        let dependent_nodes = users.iter().map(Vec::len).collect();
        let mut reset_output = Vec::with_capacity(net.outputs.len());
        for (index, &wire) in net.outputs.iter().enumerate() {
            pin(wire).connect(&mut users, nodes + index);
            reset_output.push(wire == Wire::Constant(true));
        }
        let output = reset_output.clone();
        Ok(Self {
            external,
            augmented,
            gates,
            storage,
            values: vec![false; slots],
            users,
            dependent_nodes,
            queue: EvaluationQueue::new_all(nodes),
            changes: Vec::with_capacity(slots),
            ram_writes: Vec::with_capacity(vm.storage_count()),
            write_data: vec![false; augmented],
            reset_output,
            output,
            evaluated: 0,
        })
    }

    /// すべての内部状態と外部入力を0にし、最初の tick に全素子を評価する。
    pub fn reset(&mut self) -> &[bool] {
        self.values.fill(false);
        for device in &mut self.storage {
            if let Some(ram) = &mut device.ram {
                ram.bits.fill(false);
            }
        }
        self.queue.reset_all();
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
        Some(&self.storage.get(index)?.ram.as_ref()?.bits)
    }

    /// 毎 tick 全体を走査する場合に訪れる NOR と状態素子の数。
    pub fn node_count(&self) -> usize {
        self.queue.node_count()
    }

    /// 直近の成功した tick で評価した素子の数。reset の直後は0。
    pub fn evaluated_last_tick(&self) -> usize {
        self.evaluated
    }

    /// 外部入力を適用し、1 tick 後の公開出力を借りる。
    ///
    /// 外部入力の変更は同じ tick の更新に使う。素子の出力は全候補を評価した後で確定する。
    ///
    /// # Errors
    ///
    /// 外部入力の幅が違う場合は、状態を変更せず `TickInputError::InputCount` を返す。
    pub fn step(&mut self, input: &[bool]) -> Result<&[bool], TickInputError> {
        if input.len() != self.external {
            return Err(TickInputError::InputCount);
        }
        for (slot, &value) in input.iter().enumerate() {
            self.publish(slot, value);
        }

        self.evaluated = self.queue.pending_count();
        while let Some(node) = self.queue.pop() {
            if node < self.gates.len() {
                let (left, right) = self.gates[node];
                let value = !(left.read(&self.values) || right.read(&self.values));
                let slot = self.augmented + node;
                if self.values[slot] != value {
                    self.changes.push((slot, value));
                }
            } else {
                self.evaluate_storage(node - self.gates.len());
            }
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
            ram.bits[start..start + word.len()].copy_from_slice(word);

            // 同じ入力のままでも、次の read-first 読み出しは新しい語を読む。
            let node = self.gates.len() + index;
            self.queue.insert(node);
        }
        self.ram_writes.clear();
        Ok(&self.output)
    }

    fn publish(&mut self, slot: usize, value: bool) {
        if self.values[slot] == value {
            return;
        }
        self.values[slot] = value;
        let (nodes, outputs) = self.users[slot].split_at(self.dependent_nodes[slot]);
        self.queue.insert_all(nodes);
        let base = self.queue.node_count();
        for &target in outputs {
            self.output[target - base] = value;
        }
    }

    fn evaluate_storage(&mut self, index: usize) {
        let device = &self.storage[index];
        let enabled = device.enable.read(&self.values);
        if let Some(ram) = &device.ram {
            let mut word = 0;
            for (bit, &pin) in ram.address.iter().enumerate() {
                word |= usize::from(pin.read(&self.values)) << bit;
            }
            let start = word * device.output.len();
            let read = &ram.bits[start..start + device.output.len()];
            for (slot, &value) in device.output.clone().zip(read) {
                if self.values[slot] != value {
                    self.changes.push((slot, value));
                }
            }
            if enabled {
                let mut changed = false;
                for ((slot, &pin), &old) in device.output.clone().zip(&device.data).zip(read) {
                    let value = pin.read(&self.values);
                    changed |= value != old;
                    self.write_data[slot] = value;
                }
                if changed {
                    self.ram_writes.push((index, start));
                }
            }
        } else if enabled {
            for (slot, &pin) in device.output.clone().zip(&device.data) {
                let value = pin.read(&self.values);
                if self.values[slot] != value {
                    self.changes.push((slot, value));
                }
            }
        }
    }
}
