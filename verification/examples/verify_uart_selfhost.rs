//! Core を含む ELF の転送、実行、QEMU との照合を行う。

use std::{collections::BTreeMap, error::Error, fs, path::Path, time::{Duration, Instant}};

use circuitgame2_core::netlist::{ClockedVm, Wire};
use ncg2_event_queue::{fanout::Pin, nor_groups::classify_nor_group};
use ncg2_rv64_arithmetic_proof::event_clocked::EventVm;

#[path = "../support/uart.rs"]
mod uart;

const ENTRY: u64 = 0x10000;
const HALT: u64 = 0x10068;
const STATUS: u64 = 0x103ff0;
const INFORMATION: u64 = 0x103ff8;
const MAX_WORDS: usize = 0x8400;
const STANDARD_TICKS: u64 = 1600;
const WORD_TICKS: u64 = (8 * 10 * uart::BIT_TICKS) as u64;

struct Machine {
    reference: ClockedVm,
    event: EventVm,
    frame: [bool; 65],
    address: [bool; 64],
    stable: u64,
    tick: u64,
    limit: u64,
}

impl Machine {
    fn new(mut reference: ClockedVm, limit: u64, latch_delays: bool) -> Result<Self, Box<dyn Error>> {
        let mut event = if latch_delays {
            EventVm::new_with_latch_delays(&reference)?
        } else {
            EventVm::new(&reference)?
        };
        if event.reset() != reference.reset() {
            return Err("標準 VM と疎実行で reset の出力が異なる。".into());
        }
        Ok(Self {
            reference,
            event,
            frame: [false; 65],
            address: [false; 64],
            stable: 0,
            tick: 0,
            limit,
        })
    }

    fn pc(&self) -> u64 {
        uart::word(&self.event.output()[..64])
    }

    fn step(&mut self, rx: bool) -> Result<bool, Box<dyn Error>> {
        self.step_with_evaluation(rx, |_| {})
    }

    fn step_with_evaluation(
        &mut self,
        rx: bool,
        evaluate: impl FnMut(usize),
    ) -> Result<bool, Box<dyn Error>> {
        if self.tick >= self.limit {
            return Err(format!(
                "上限 {} tick までに終了しなかった。PC={:#x}",
                self.limit,
                self.pc()
            )
            .into());
        }
        self.tick += 1;
        self.frame[0] = rx;
        let output = self
            .event
            .step_with_evaluation(&self.frame, evaluate)
            .map_err(|error| format!("tick {}: {error:?}", self.tick))?;
        if self.tick <= STANDARD_TICKS
            && self
                .reference
                .step(&self.frame)
                .map_err(|error| format!("標準 VM: {error:?}"))?
                != output
        {
            return Err(format!("tick {}: 公開出力が標準 VM と異なる。", self.tick).into());
        }
        let bus = &output[64..128];
        if self.address == bus {
            self.stable += 1;
        } else {
            self.address.copy_from_slice(bus);
            self.stable = 1;
        }
        if output[193] || output[195] || (output[196] && self.stable < 32) {
            return Err(format!(
                "tick {} PC={:#x}: UART エラー={}、CPU 停止={}、アドレス保持={} tick。",
                self.tick,
                uart::word(&output[..64]),
                output[193],
                output[195],
                self.stable
            )
            .into());
        }
        Ok(output[192])
    }

    fn storage_index(&self, address_bits: usize) -> Result<usize, Box<dyn Error>> {
        let mut matches = (0..self.reference.storage_count()).filter(|&index| {
            self.reference
                .storage_wiring(index)
                .is_some_and(|(wiring, _)| {
                    wiring.data.len() == 64
                        && wiring
                            .address
                            .as_ref()
                            .is_some_and(|pins| pins.len() == address_bits)
                })
        });
        let index = matches.next().ok_or("照合先の RAM が見つからない。")?;
        if matches.next().is_some() {
            return Err("照合先と同じアドレス幅の RAM が複数ある。".into());
        }
        Ok(index)
    }

    fn read_word(
        &self,
        bank: usize,
        address: u64,
        address_bits: usize,
    ) -> Result<u64, Box<dyn Error>> {
        let memory = self
            .event
            .ram_memory(bank)
            .ok_or("照合先の RAM の内容を取得できない。")?;
        let start = ((address as usize >> 3) & ((1 << address_bits) - 1)) * 64;
        let bits = memory
            .get(start..start + 64)
            .ok_or_else(|| format!("RAM の語 {address:#x} は範囲外。"))?;
        Ok(uart::word(bits))
    }
}

fn image_words(image: &[u8]) -> Result<&[[u8; 8]], Box<dyn Error>> {
    let (words, tail) = image.as_chunks::<8>();
    let (header, payload) = words.split_first().ok_or("転送データに語数がない。")?;
    if !tail.is_empty()
        || !(1..=MAX_WORDS).contains(&payload.len())
        || u64::from_le_bytes(*header) != payload.len() as u64
    {
        return Err("転送ヘッダの語数とデータ長が一致しないか、容量を超える。".into());
    }
    Ok(words)
}

fn reference_pcs(path: &Path) -> Result<Vec<u64>, Box<dyn Error>> {
    let mut pcs = Vec::new();
    for line in fs::read_to_string(path)?
        .lines()
        .filter(|line| line.starts_with("Trace "))
    {
        let pc = line
            .split_once(" [")
            .and_then(|(_, state)| state.split('/').nth(1))
            .ok_or("QEMU の実行列から PC を読めない。")?;
        pcs.push(u64::from_str_radix(pc, 16)?);
    }
    if pcs.last() == Some(&HALT) {
        pcs.pop();
    }
    if pcs.first() != Some(&ENTRY)
        || pcs
            .iter()
            .any(|&pc| pc == HALT || pc % 4 != 0 || !(ENTRY..0x52000).contains(&pc))
    {
        return Err("QEMU の実行列はアプリの入口から終了位置の直前までを含む。".into());
    }
    Ok(pcs)
}

#[derive(Default)]
struct PhaseMetrics {
    ticks: u64,
    elapsed: Duration,
    evaluations: u64,
    chain_evaluations: u64,
    grouped_nor_evaluations: u64,
    outer_group_ticks: u64,
    dense_group_ticks: u64,
    ticks_without_other_evaluations: u64,
    commit_ticks: u64,
    write_ticks: u64,
}

impl PhaseMetrics {
    fn record(
        &mut self,
        evaluations: u64,
        chain_evaluations: u64,
        grouped_nor_evaluations: u64,
        commit: bool,
        write: bool,
    ) {
        self.ticks += 1;
        self.evaluations += evaluations;
        self.chain_evaluations += chain_evaluations;
        self.grouped_nor_evaluations += grouped_nor_evaluations;
        self.ticks_without_other_evaluations += u64::from(evaluations == chain_evaluations);
        self.commit_ticks += u64::from(commit);
        self.write_ticks += u64::from(write);
    }

    fn report(&self, name: &str) {
        println!(
            "{name} {} tick、{} 素子評価、遅延鎖内 {} 評価、遅延鎖外の評価がない tick {}、退役信号 {} tick、書き込み有効 {} tick",
            self.ticks,
            self.evaluations,
            self.chain_evaluations,
            self.ticks_without_other_evaluations,
            self.commit_ticks,
            self.write_ticks,
        );
        if self.ticks != 0 {
            let seconds = self.elapsed.as_secs_f64();
            println!(
                "  経過時間 {seconds:.6} s、平均 {:.2} µs/tick",
                seconds * 1_000_000.0 / self.ticks as f64,
            );
        }
        println!(
            "  NOR 候補 {} 評価、候補群の稼働 {} 回、同じ群で4ゲート以上を評価 {} 回",
            self.grouped_nor_evaluations, self.outer_group_ticks, self.dense_group_ticks,
        );
    }
}

struct TransferMetrics {
    chain_nodes: Vec<bool>,
    grouped_nor: WordGroupActivity,
    waveform: PhaseMetrics,
    ack: PhaseMetrics,
}

fn flash(
    machine: &mut Machine,
    words: &[[u8; 8]],
    mut metrics: Option<&mut TransferMetrics>,
) -> Result<(), Box<dyn Error>> {
    let program = machine.storage_index(15)?;
    let tail = machine.storage_index(10)?;
    let check = |machine: &Machine, offset: usize, bytes: [u8; 8]| -> Result<(), Box<dyn Error>> {
        let address = ENTRY + offset as u64 * 8;
        let (bank, width) = if address < 0x50000 {
            (program, 15)
        } else {
            (tail, 10)
        };
        let actual = machine.read_word(bank, address, width)?;
        let expected = u64::from_le_bytes(bytes);
        if actual != expected {
            return Err(format!("tick {}: 転送語 {} の書き込み先 RAM {address:#x} は {actual:#018x}、期待値は {expected:#018x}。", machine.tick, offset + 1).into());
        }
        Ok(())
    };
    for _ in 0..uart::BIT_TICKS {
        machine.step(true)?;
    }
    for (index, &bytes) in words.iter().enumerate() {
        let start = machine.tick;
        let started = metrics.as_ref().map(|_| Instant::now());
        let mut waveform_end = None;
        let acknowledged = uart::send(bytes, |rx| -> Result<bool, Box<dyn Error>> {
            let ready = match metrics.as_deref_mut() {
                Some(metrics) => {
                    let mut chain_evaluations = 0;
                    let mut grouped_nor_evaluations = 0;
                    let ready = machine.step_with_evaluation(rx, |node| {
                        chain_evaluations += u64::from(metrics.chain_nodes[node]);
                        grouped_nor_evaluations += u64::from(metrics.grouped_nor.observe(node));
                    })?;
                    let word_tick = machine.tick - start;
                    let phase = if word_tick <= WORD_TICKS {
                        &mut metrics.waveform
                    } else {
                        &mut metrics.ack
                    };
                    metrics.grouped_nor.finish_tick(phase);
                    let output = machine.event.output();
                    phase.record(
                        machine.event.evaluated_last_tick() as u64,
                        chain_evaluations,
                        grouped_nor_evaluations,
                        output[194],
                        output[196],
                    );
                    // 各語の送信波形が終わる境界で時刻を取り、tick ごとの時計読み出しを避ける。
                    if word_tick == WORD_TICKS {
                        waveform_end = Some(Instant::now());
                    }
                    ready
                }
                None => machine.step(rx)?,
            };
            if machine.event.output()[194] && machine.pc() >= ENTRY {
                return Err("転送中にアプリの命令が退役した。".into());
            }
            Ok(ready)
        })
        .map_err(|cause| format!("UART 転送語 {index}: {cause}"))?;
        if let (Some(metrics), Some(started), Some(end)) = (&mut metrics, started, waveform_end) {
            metrics.ack.elapsed += end.elapsed();
            metrics.waveform.elapsed += end.duration_since(started);
        }
        if !acknowledged {
            return Err(format!(
                "転送語 {index} の ACK を受け取れない。PC={:#x}",
                machine.pc()
            )
            .into());
        }
        if index > 0 {
            check(machine, index - 1, bytes)?;
            if index % 256 == 0 {
                println!(
                    "転送・照合 {index}/{} 語、tick {}",
                    words.len() - 1,
                    machine.tick
                );
            }
        }
    }
    if machine.pc() >= ENTRY {
        return Err("転送内容の照合前にアプリが開始した。".into());
    }
    // 後続の書き込みによって先に受け取った語が変わっていないことを確認する。
    for (offset, &bytes) in words[1..].iter().enumerate() {
        check(machine, offset, bytes)?;
    }
    println!(
        "転送・照合した区間の最終 tick {}、{}語一致",
        machine.tick,
        words.len() - 1
    );
    Ok(())
}

struct WordGroupActivity {
    gate_group: Vec<Option<usize>>,
    active_counts: Vec<u8>,
    active_groups: Vec<usize>,
}

impl WordGroupActivity {
    fn observe(&mut self, node: usize) -> bool {
        let Some(group) = self.gate_group.get(node).copied().flatten() else {
            return false;
        };
        if self.active_counts[group] == 0 {
            self.active_groups.push(group);
        }
        self.active_counts[group] += 1;
        true
    }

    fn finish_tick(&mut self, phase: &mut PhaseMetrics) {
        for group in self.active_groups.drain(..) {
            phase.outer_group_ticks += 1;
            phase.dense_group_ticks += u64::from(self.active_counts[group] >= 4);
            self.active_counts[group] = 0;
        }
    }
}

fn candidate_word_gates(vm: &ClockedVm) -> WordGroupActivity {
    let netlist = vm.netlist();
    let slot_count = netlist.input_count + netlist.gates.len();
    let pin = |wire| match wire {
        Wire::Input(slot) => slot,
        Wire::Gate(index) => netlist.input_count + index,
        Wire::Constant(value) => slot_count + usize::from(value),
    };
    let decode = |index| {
        if index < slot_count { Pin::Slot(index) } else { Pin::Constant(index != slot_count) }
    };
    let mut by_inputs: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (index, gate) in netlist.gates.iter().enumerate() {
        let pair = (pin(gate.left), pin(gate.right));
        by_inputs.entry(pair).or_default().push(index);
        // NOR の左右を入れ替えても値は同じになる。
        if pair.0 != pair.1 {
            by_inputs.entry((pair.1, pair.0)).or_default().push(index);
        }
    }
    let mut gate_group = vec![None; netlist.gates.len()];
    let mut count = 0;
    for (left_step, right_step) in [(1, 1), (1, 0), (0, 1)] {
        for &first in by_inputs.keys() {
            let mut members = [0; 64];
            let mut pins = [(Pin::Constant(false), Pin::Constant(false)); 64];
            let mut width = 0;
            while width < 64 {
                let pair = (first.0 + width * left_step, first.1 + width * right_step);
                let Some(gate) = by_inputs.get(&pair).and_then(|gates| {
                    gates.iter().copied().find(|&index| gate_group[index].is_none())
                }) else {
                    break;
                };
                if members[..width].contains(&gate) {
                    break;
                }
                members[width] = gate;
                pins[width] = (decode(pair.0), decode(pair.1));
                width += 1;
            }
            if width >= 4 && classify_nor_group(&pins[..width]).is_some() {
                for &gate in &members[..width] {
                    gate_group[gate] = Some(count);
                }
                count += 1;
            }
        }
    }
    WordGroupActivity {
        gate_group,
        active_counts: vec![0; count],
        active_groups: Vec::with_capacity(count),
    }
}

fn candidate_shared_input_groups(vm: &ClockedVm) -> WordGroupActivity {
    let netlist = vm.netlist();
    let source_slot = |wire| match wire {
        Wire::Input(slot) => Some(slot),
        Wire::Gate(index) => Some(netlist.input_count + index),
        Wire::Constant(_) => None,
    };
    let mut readers = vec![Vec::new(); netlist.input_count + netlist.gates.len()];
    for (gate, inputs) in netlist.gates.iter().enumerate() {
        let left = source_slot(inputs.left);
        let right = source_slot(inputs.right);
        if let Some(slot) = left {
            readers[slot].push(gate);
        }
        if let Some(slot) = right
            && right != left
        {
            readers[slot].push(gate);
        }
    }
    let mut sources: Vec<_> = readers
        .iter()
        .enumerate()
        .filter(|(_, gates)| gates.len() >= 4)
        .collect();
    sources.sort_unstable_by_key(|&(slot, gates)| (std::cmp::Reverse(gates.len()), slot));
    let mut gate_group = vec![None; netlist.gates.len()];
    let mut count = 0;
    for (_, readers) in sources {
        let available: Vec<_> = readers
            .iter()
            .copied()
            .filter(|&gate| gate_group[gate].is_none())
            .collect();
        for group in available.chunks(64).filter(|group| group.len() >= 4) {
            for &gate in group {
                gate_group[gate] = Some(count);
            }
            count += 1;
        }
    }
    WordGroupActivity {
        gate_group,
        active_counts: vec![0; count],
        active_groups: Vec::with_capacity(count),
    }
}

fn measure_transfer(
    project: &Path,
    image: &Path,
    count: usize,
    shared_input: bool,
    latch_delays: bool,
) -> Result<(), Box<dyn Error>> {
    let image = fs::read(image)?;
    let words = image_words(&image)?;
    if count == 0 || count >= words.len() {
        return Err("計測するペイロード語数が転送データの範囲外。".into());
    }
    let setup = Instant::now();
    let mut machine = Machine::new(uart::machine(project)?, u64::MAX, latch_delays)?;
    let setup_elapsed = setup.elapsed();
    let chains = machine.event.linear_latch_chains();
    let stages: usize = chains.iter().map(Vec::len).sum();
    let longest = chains.iter().map(Vec::len).max().unwrap_or(0);
    let gate_count = machine.reference.nor_gate_count();
    let mut chain_nodes = vec![false; machine.event.node_count()];
    for stage in chains.iter().flatten().copied() {
        chain_nodes[gate_count + stage] = true;
    }
    let grouped_nor = if shared_input {
        candidate_shared_input_groups(&machine.reference)
    } else {
        candidate_word_gates(&machine.reference)
    };
    let mut metrics = TransferMetrics {
        chain_nodes,
        grouped_nor,
        waveform: PhaseMetrics::default(),
        ack: PhaseMetrics::default(),
    };
    let started = Instant::now();
    // ヘッダーには元の全語数を残し、最初の count 語の ACK と RAM を同じ経路で調べる。
    flash(&mut machine, &words[..=count], Some(&mut metrics))?;
    let elapsed = started.elapsed();
    println!(
        "転送先頭 {count} 語、構築 {:.3}s、転送・照合 {:.3}s",
        setup_elapsed.as_secs_f64(),
        elapsed.as_secs_f64(),
    );
    metrics.waveform.report("UART 波形");
    metrics.ack.report("ACK 待ち");
    println!(
        "縮約候補の遅延鎖 {} 本、ラッチ {stages} 個、最長 {longest} 段",
        chains.len(),
    );
    let groups = &metrics.grouped_nor;
    let grouped = groups.gate_group.iter().filter(|candidate| candidate.is_some()).count();
    let grouping = if shared_input { "共通入力" } else { "連続した入力" };
    println!(
        "{grouping}で集めた4〜64ゲートの候補は {} 群、NOR {grouped}/{} 個",
        groups.active_counts.len(), groups.gate_group.len(),
    );
    Ok(())
}

fn verify_execution(
    machine: &mut Machine,
    reference: &[u64],
    expected: (u64, u64),
) -> Result<(), Box<dyn Error>> {
    while machine.pc() < ENTRY {
        machine.step(true)?;
    }
    let start = machine.tick;
    let mut retired = 0;
    loop {
        if retired == reference.len() && machine.pc() == HALT {
            break;
        }
        if machine.event.output()[194] {
            let pc = machine.pc();
            if reference.get(retired) != Some(&pc) {
                return Err(format!(
                    "命令 {}、tick {}: 実際の PC {pc:#x}、QEMU {:?}",
                    retired + 1,
                    machine.tick,
                    reference.get(retired)
                )
                .into());
            }
            retired += 1;
            if retired % 1024 == 0 {
                println!(
                    "QEMU と {retired}/{} 命令一致、アプリ {} tick",
                    reference.len(),
                    machine.tick - start
                );
            }
        }
        machine.step(true)?;
    }
    let stack = machine.storage_index(11)?;
    let status = machine.read_word(stack, STATUS, 11)?;
    let information = machine.read_word(stack, INFORMATION, 11)?;
    if (status, information) != expected {
        let (expected_status, expected_info) = expected;
        return Err(format!(
            "RAM の結果が QEMU と異なる。status={status:#x} information={information:#x}、QEMU の結果は {expected_status:#x} {expected_info:#x}。"
        )
        .into());
    }
    println!(
        "アプリ {} tick、{retired} 命令と結果が一致。status={status:#x} information={information:#x}。",
        machine.tick - start
    );
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if let [project, image, flag, count] = args.as_slice()
        && (flag == "--measure-transfer" || flag == "--measure-transfer-shared"
            || flag == "--measure-transfer-delays")
    {
        let count = count
            .to_str()
            .ok_or("計測する語数は十進整数で指定する。")?
            .parse()?;
        return measure_transfer(
            Path::new(project),
            Path::new(image),
            count,
            flag == "--measure-transfer-shared",
            flag == "--measure-transfer-delays",
        );
    }
    let (project, image, log, expected, limit) = match args.as_slice() {
        [project, image, log, info, limit] => {
            let info = u64::from_str_radix(
                info.to_str().ok_or("結果には16進整数を指定する。")?.trim_start_matches("0x"),
                16,
            )?;
            (project, image, log, (1, info), limit)
        }
        [project, image, log, flag, result, limit] if flag == "--result-file" => {
            let bytes = fs::read(result)?;
            let ([status, info], []) = bytes.as_chunks::<8>() else {
                return Err("QEMU の結果ファイルにはリトルエンディアンの64ビット語が2個必要。".into());
            };
            (project, image, log, (u64::from_le_bytes(*status), u64::from_le_bytes(*info)), limit)
        }
        _ => return Err("実行設定、UART 転送データ、QEMU ログ、期待結果（16進または --result-file と結果ファイル）、tick 上限を指定する。".into()),
    };
    let limit = limit
        .to_str()
        .ok_or("tick 上限には十進整数を指定する。")?
        .parse()?;
    let image = fs::read(image)?;
    let words = image_words(&image)?;
    let reference = reference_pcs(Path::new(log))?;
    let mut machine = Machine::new(uart::machine(Path::new(project))?, limit, false)?;
    flash(&mut machine, words, None)?;
    verify_execution(&mut machine, &reference, expected)
}
