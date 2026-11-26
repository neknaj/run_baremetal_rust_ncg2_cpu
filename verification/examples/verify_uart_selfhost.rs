//! Core を含む ELF の転送、実行、QEMU との照合を行う。

use std::{collections::VecDeque, error::Error, fs, path::Path, time::Instant};

use circuitgame2_core::netlist::ClockedVm;
use ncg2_uart_loader::{Acceptance, BootLoader, BootFrame, RamWord, Receive};
use ncg2_rv64_arithmetic_proof::event_clocked::EventVm;

#[path = "../support/uart.rs"]
mod uart;
use uart::ContinuousRate;

const ENTRY: u64 = 0x10000;
const HALT: u64 = 0x10068;
const STATUS: u64 = 0x103ff0;
const INFORMATION: u64 = 0x103ff8;
const MAX_WORDS: usize = 0x8400;
const STANDARD_TICKS: u64 = 1600;

enum Loader {
    Rom,
    Host,
    Circuit,
}

struct Machine {
    reference: ClockedVm,
    event: EventVm,
    frame: [bool; 194],
    loader: Loader,
    bus: Option<RamWord>,
    address: [bool; 64],
    stable: u64,
    tick: u64,
    limit: u64,
}

impl Machine {
    fn new(mut reference: ClockedVm, limit: u64, loader: Loader) -> Result<Self, Box<dyn Error>> {
        let mut event = EventVm::new(&reference)?;
        if event.reset() != reference.reset() {
            return Err("標準 VM と疎実行で reset の出力が異なる。".into());
        }
        Ok(Self {
            reference,
            event,
            frame: [false; 194],
            loader,
            bus: None,
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
        if !matches!(self.loader, Loader::Host) {
            self.frame[0] = rx;
        }
        self.step_frame()
    }

    fn drive_loader(&mut self, frame: BootFrame) -> Result<bool, Box<dyn Error>> {
        if self.bus != frame.bus {
            let (address, data) = frame.bus.map_or((0, 0), |word| (word.address, word.data));
            for bit in 0..64 {
                self.frame[64 + bit] = address >> bit & 1 != 0;
                self.frame[128 + bit] = data >> bit & 1 != 0;
            }
            self.bus = frame.bus;
        }
        self.frame[192] = frame.write;
        self.frame[193] = frame.cpu_ready;
        self.step_frame()
    }

    fn step_frame(&mut self) -> Result<bool, Box<dyn Error>> {
        if self.tick >= self.limit {
            return Err(format!(
                "上限 {} tick までに終了しなかった。PC={:#x}",
                self.limit,
                self.pc()
            )
            .into());
        }
        self.tick += 1;
        let frame = if matches!(self.loader, Loader::Host) { &self.frame[..] } else { &self.frame[..65] };
        let output = self
            .event
            .step(frame)
            .map_err(|error| format!("tick {}: {error:?}", self.tick))?;
        if self.tick <= STANDARD_TICKS
            && self
                .reference
                .step(frame)
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
        let uart_error = match self.loader {
            Loader::Rom => output[193],
            Loader::Host => false,
            Loader::Circuit => output[197],
        };
        if uart_error || output[195] || (output[196] && self.stable < 32) {
            return Err(format!(
                "tick {} PC={:#x}: UART エラー={}、CPU 停止={}、アドレス保持={} tick。",
                self.tick,
                uart::word(&output[..64]),
                uart_error,
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

fn flash(machine: &mut Machine, words: &[[u8; 8]]) -> Result<(), Box<dyn Error>> {
    let banks = (machine.storage_index(15)?, machine.storage_index(10)?);
    for _ in 0..uart::BIT_TICKS {
        machine.step(true)?;
    }
    for (index, &bytes) in words.iter().enumerate() {
        let acknowledged = uart::send(bytes, |rx| -> Result<bool, Box<dyn Error>> {
            let ready = machine.step(rx)?;
            if machine.event.output()[194] && machine.pc() >= ENTRY {
                return Err("転送中にアプリの命令が退役した。".into());
            }
            Ok(ready)
        })
        .map_err(|cause| format!("UART 転送語 {index}: {cause}"))?;
        if !acknowledged {
            return Err(format!(
                "転送語 {index} の ACK を受け取れない。PC={:#x}",
                machine.pc()
            )
            .into());
        }
        if index > 0 {
            compare_payload_word(machine, banks, index - 1, bytes)?;
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
        compare_payload_word(machine, banks, offset, bytes)?;
    }
    println!(
        "転送完了、tick {}、全{}語一致",
        machine.tick,
        words.len() - 1
    );
    Ok(())
}

fn continuous_setup(
    path: &Path, limit: u64, rate: ContinuousRate,
) -> Result<(Machine, ClockedVm), Box<dyn Error>> {
    let (handoff, receiver) = uart::continuous_machines(path, rate)?;
    Ok((Machine::new(handoff, limit, Loader::Host)?, receiver))
}

fn compare_payload_word(
    machine: &Machine,
    banks: (usize, usize),
    index: usize,
    bytes: [u8; 8],
) -> Result<(), Box<dyn Error>> {
    let address = ENTRY + index as u64 * 8;
    let (bank, width) = if address < 0x50000 { (banks.0, 15) } else { (banks.1, 10) };
    let actual = machine.read_word(bank, address, width)?;
    let expected = u64::from_le_bytes(bytes);
    if actual != expected {
        return Err(format!("tick {}: 転送語 {} の書き込み先 RAM {address:#x} は {actual:#018x}、期待値は {expected:#018x}。", machine.tick, index + 1).into());
    }
    Ok(())
}

fn flash_continuous(
    machine: &mut Machine,
    mut receiver_reference: ClockedVm,
    words: &[[u8; 8]],
    complete: bool,
    rate: ContinuousRate,
) -> Result<(), Box<dyn Error>> {
    let mut receiver = EventVm::new(&receiver_reference)?;
    if receiver.reset() != receiver_reference.reset() {
        return Err("UART 受信器の reset が標準 VM と異なる。".into());
    }
    let mut loader = BootLoader::reset();
    let mut acknowledged = false;
    let mut accepted = 0;
    let mut confirmed = 0;
    let mut expected_writes = VecDeque::new();
    let mut pending_writes = VecDeque::new();
    let banks = (machine.storage_index(15)?, machine.storage_index(10)?);
    let bit_ticks = rate.bit_ticks();
    let waveform_ticks = (uart::BIT_TICKS + bit_ticks * 80 * words.len()) as u64;
    let waveform = std::iter::repeat_n(true, uart::BIT_TICKS)
        .chain(words.iter().copied().flat_map(|bytes| uart::waveform(bytes, rate)))
        .chain(std::iter::repeat_n(true, 96));
    let started = Instant::now();
    let mut wire_elapsed = None;
    let mut last_ram_tick = 0;
    let mut loader_finished = None;
    for rx in waveform {
        let tick = machine.tick + 1;
        let previous = receiver.output();
        if previous[65] {
            return Err(format!("tick {tick}: UART のフレーミングまたは語組み立てでエラー。").into());
        }
        let input = if previous[64] && !acknowledged {
            Receive::Word(uart::word(&previous[..64]))
        } else {
            Receive::Idle
        };
        let frame = loader.step(input);
        let ack = match frame.accepted {
            Acceptance::Header | Acceptance::Payload => {
                let expected = words.get(accepted).copied().map(u64::from_le_bytes);
                if Some(uart::word(&previous[..64])) != expected {
                    return Err(format!("tick {tick}: 受信語 {} が送信データと異なる。", accepted + 1).into());
                }
                accepted += 1;
                true
            }
            Acceptance::Rejected(error) => return Err(format!("tick {tick}: ローダーが拒否した。{error:?}").into()),
            Acceptance::None => false,
        };
        acknowledged = previous[64] && (acknowledged || ack);
        let receiver_input = [rx, ack];
        let output = receiver.step(&receiver_input).map_err(|error| format!("UART 受信器: {error:?}"))?;
        if tick <= STANDARD_TICKS.max(4 * bit_ticks as u64 * 80)
            && receiver_reference.step(&receiver_input).map_err(|error| format!("UART 受信器の標準 VM: {error:?}"))? != output
        {
            return Err(format!("tick {tick}: UART 受信器の公開出力が標準 VM と異なる。").into());
        }

        if output[65] {
            return Err(format!("tick {tick}: UART のフレーミングまたは語組み立てでエラー。").into());
        }
        if frame.cpu_ready && loader_finished.is_none() {
            loader_finished = Some(tick);
        }
        if frame.write {
            let word = frame.bus.ok_or("ローダーの書き込み許可にアドレスとデータがない。")?;
            expected_writes.push_back((tick + 6, word));
        }
        let owned = machine.drive_loader(frame)?;
        let output = machine.event.output();
        if output[193] || output[194] || (owned && output[196]) {
            return Err(format!("tick {tick}: 引き渡し検査の終了前に CPU の発行、退役または書き込みを観測した。").into());
        }
        if output[196] {
            let address = uart::word(&output[64..128]);
            let Some((planned_tick, word)) = expected_writes.pop_front() else {
                return Err(format!("tick {tick}: ローダーが発行していない RAM 書き込み。").into());
            };
            if planned_tick != tick || address != word.address {
                return Err(format!("tick {tick}: RAM への許可の時刻またはアドレスがローダーの予定と異なる。").into());
            }
            pending_writes.push_back((tick + 3, word));
        }
        if expected_writes.front().is_some_and(|&(planned, _)| planned <= tick) {
            return Err(format!("tick {tick}: RAM への書き込み許可が届かない。").into());
        }
        if let Some(&(due, word)) = pending_writes.front()
            && due == tick
        {
            if word.address != ENTRY + 8 * confirmed as u64 || confirmed + 1 >= words.len() {
                return Err(format!("tick {tick}: RAM に確定した語の順序が異なる。").into());
            }
            compare_payload_word(machine, banks, confirmed, words[1 + confirmed])?;
            pending_writes.pop_front();
            confirmed += 1;
            last_ram_tick = tick;
            if confirmed % 1024 == 0 {
                println!("物理 RAM 確定 {confirmed}/{} 語、tick {tick}、経過 {:.3} s", words.len() - 1, started.elapsed().as_secs_f64());
            }
        }
        if owned && (!complete || loader_finished.map(|finished| finished + 8) != Some(tick)
            || confirmed != words.len() - 1 || !pending_writes.is_empty() || last_ram_tick >= tick)
        {
            return Err(format!("tick {tick}: 全語を照合する前に RAM が CPU に引き渡された。").into());
        }
        if tick == waveform_ticks {
            wire_elapsed = Some(started.elapsed());
        }
        if tick >= waveform_ticks && accepted == words.len() && confirmed == words.len() - 1
            && expected_writes.is_empty() && pending_writes.is_empty() && owned == complete
        {
            for (index, &word) in words[1..].iter().enumerate() {
                compare_payload_word(machine, banks, index, word)?;
            }
            let elapsed = started.elapsed().as_secs_f64();
            let wire_seconds = wire_elapsed.ok_or("UART の波形終了時刻がない。")?.as_secs_f64();
            println!("UART の波形終了 {waveform_ticks} tick、{bit_ticks} tick/bit、経過 {wire_seconds:.6} s");
            println!("最後の物理 RAM 確定 {last_ram_tick} tick");
            if let Some(finished) = loader_finished {
                println!("ローダーの完了 {finished} tick、CPU への引き渡し {tick} tick");
            }
            println!(
                "転送と全{}語の照合を終了 {} tick、CPU 所有={}、経過 {elapsed:.6} s、平均 {:.3} µs/tick",
                confirmed, tick, owned, elapsed * 1_000_000.0 / tick as f64,
            );
            return Ok(());
        }
    }
    Err(format!("tick {}: 受信 {accepted} 語、物理 RAM 確定 {confirmed} 語で転送が終了しなかった。", machine.tick).into())
}

/// 連続 RX を単一の回路へ入力し、許可から3 tick 後の RAM を転送語と照合する。
fn flash_circuit(machine: &mut Machine, words: &[[u8; 8]], complete: bool) -> Result<(), Box<dyn Error>> {
    let banks = (machine.storage_index(15)?, machine.storage_index(10)?);
    let payload = &words[1..];
    let rate = ContinuousRate::Fast;
    let idle = uart::BIT_TICKS;
    let waveform_ticks = (idle + words.len() * 80 * rate.bit_ticks()) as u64;
    let waveform = std::iter::repeat_n(true, idle)
        .chain(words.iter().copied().flat_map(|word| uart::waveform(word, rate)))
        .chain(std::iter::repeat_n(true, 256));

    let mut confirmed = 0;
    let mut pending = None;
    let mut last_ram_tick = 0;
    let mut wire_elapsed = None;
    let started = Instant::now();
    for rx in waveform {
        let owned = machine.step(rx)?;
        let tick = machine.tick;
        let output = machine.event.output();
        if output[193] || output[194] {
            return Err(format!("tick {tick}: RAM の引き渡し前に CPU が命令を実行した。").into());
        }
        if pending == Some(tick) {
            compare_payload_word(machine, banks, confirmed, payload[confirmed])?;
            confirmed += 1;
            pending = None;
            last_ram_tick = tick;
            if confirmed % 1024 == 0 {
                println!(
                    "物理 RAM 確定 {confirmed}/{} 語、tick {tick}、経過 {:.3} s",
                    payload.len(),
                    started.elapsed().as_secs_f64(),
                );
            }
        }
        if output[196] {
            let address = uart::word(&output[64..128]);
            let expected = ENTRY + 8 * confirmed as u64;
            if owned || pending.is_some() || confirmed >= payload.len() || address != expected {
                return Err(format!(
                    "tick {tick}: 書き込み先 {address:#x}。次に照合する語は {}、アドレスは {expected:#x}。",
                    confirmed + 1,
                ).into());
            }
            pending = Some(tick + 3);
        }
        if owned && (!complete || confirmed != payload.len() || pending.is_some() || last_ram_tick >= tick) {
            return Err(format!("tick {tick}: 全語の照合を終える前に RAM が CPU に引き渡された。").into());
        }
        if tick == waveform_ticks {
            wire_elapsed = Some(started.elapsed());
        }
        if tick >= waveform_ticks && confirmed == payload.len() && pending.is_none() && owned == complete {
            for (index, &word) in payload.iter().enumerate() {
                compare_payload_word(machine, banks, index, word)?;
            }
            let seconds = wire_elapsed.ok_or("UART の波形終了時刻がない。")?.as_secs_f64();
            println!("UART の波形終了 {waveform_ticks} tick、8 tick/bit、経過 {seconds:.6} s");
            println!("最後の物理 RAM 確定 {last_ram_tick} tick");
            let elapsed = started.elapsed().as_secs_f64();
            println!(
                "転送と全{confirmed}語の照合を終了 {tick} tick、CPU 所有={owned}、経過 {elapsed:.6} s、平均 {:.3} µs/tick",
                elapsed * 1_000_000.0 / tick as f64,
            );
            return Ok(());
        }
    }
    Err(format!(
        "tick {}: 物理 RAM で {confirmed}/{} 語を照合したところで、転送が終了しなかった。",
        machine.tick, payload.len(),
    ).into())
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
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    if let [flag, project, image, count] = arguments.as_slice()
        && (flag == "--measure-continuous" || flag == "--measure-continuous-8" || flag == "--measure-circuit-loader")
    {
        let rate = if flag == "--measure-continuous-8" {
            ContinuousRate::Fast
        } else {
            ContinuousRate::Standard
        };
        let bytes = fs::read(image)?;
        let words = image_words(&bytes)?;
        let count: usize = count.to_str().ok_or("先頭語数を十進整数で指定する。")?.parse()?;
        if count == 0 || count > words.len() - 1 {
            return Err("先頭語数が転送データの範囲外。".into());
        }
        let setup = Instant::now();
        let complete = count == words.len() - 1;
        if flag == "--measure-circuit-loader" {
            let mut machine = Machine::new(uart::circuit_loader_machine(Path::new(project))?, u64::MAX, Loader::Circuit)?;
            println!("回路を構築 {:.6} s", setup.elapsed().as_secs_f64());
            return flash_circuit(&mut machine, &words[..=count], complete);
        }
        let (mut machine, receiver) = continuous_setup(Path::new(project), u64::MAX, rate)?;
        println!("回路を構築 {:.6} s", setup.elapsed().as_secs_f64());
        return flash_continuous(&mut machine, receiver, &words[..=count], complete, rate);
    }
    let rate = match arguments.first() {
        Some(flag) if flag == "--continuous" => Some(ContinuousRate::Standard),
        Some(flag) if flag == "--continuous-8" => Some(ContinuousRate::Fast),
        _ => None,
    };
    let circuit_loader = arguments.first().is_some_and(|flag| flag == "--circuit-loader");
    let args = if rate.is_some() || circuit_loader { &arguments[1..] } else { &arguments[..] };
    let (project, image, log, expected, limit) = match args {
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
    let mut machine = if circuit_loader {
        let setup = Instant::now();
        let mut machine = Machine::new(uart::circuit_loader_machine(Path::new(project))?, limit, Loader::Circuit)?;
        println!("回路を構築 {:.6} s", setup.elapsed().as_secs_f64());
        flash_circuit(&mut machine, words, true)?;
        machine
    } else if let Some(rate) = rate {
        let setup = Instant::now();
        let (mut machine, receiver) = continuous_setup(Path::new(project), limit, rate)?;
        println!("回路を構築 {:.6} s", setup.elapsed().as_secs_f64());
        flash_continuous(&mut machine, receiver, words, true, rate)?;
        machine
    } else {
        let mut machine = Machine::new(uart::machine(Path::new(project))?, limit, Loader::Rom)?;
        flash(&mut machine, words)?;
        machine
    };
    let execution = Instant::now();
    verify_execution(&mut machine, &reference, expected)?;
    println!("転送後の実行に {:.6} s", execution.elapsed().as_secs_f64());
    Ok(())
}
