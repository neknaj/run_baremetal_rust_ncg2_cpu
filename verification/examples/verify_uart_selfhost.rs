//! Core を含む ELF の転送、実行、QEMU との照合を行う。

use std::{error::Error, fs, path::Path};

use circuitgame2_core::netlist::ClockedVm;
use ncg2_rv64_arithmetic_proof::event_clocked::EventVm;

#[path = "../support/uart.rs"]
mod uart;

const ENTRY: u64 = 0x10000;
const HALT: u64 = 0x10068;
const STATUS: u64 = 0x103ff0;
const INFORMATION: u64 = 0x103ff8;
const MAX_WORDS: usize = 0x8400;
const STANDARD_TICKS: u64 = 1600;

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
    fn new(mut reference: ClockedVm, limit: u64) -> Result<Self, Box<dyn Error>> {
        let mut event = EventVm::new(&reference)?;
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
            .step(&self.frame)
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

    fn storage(&self, address_bits: usize) -> Result<&[bool], Box<dyn Error>> {
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
        let memory = matches
            .next()
            .and_then(|index| self.event.ram_memory(index))
            .ok_or("照合先の RAM が見つからない。")?;
        if matches.next().is_some() {
            return Err("照合先と同じアドレス幅の RAM が複数ある。".into());
        }
        Ok(memory)
    }
}

fn memory_word(bank: &[bool], address: u64, address_bits: usize) -> Result<u64, Box<dyn Error>> {
    let start = ((address as usize >> 3) & ((1 << address_bits) - 1)) * 64;
    let bits = bank
        .get(start..start + 64)
        .ok_or_else(|| format!("RAM の語 {address:#x} は範囲外。"))?;
    Ok(uart::word(bits))
}

fn image_words(image: &[u8]) -> Result<impl Iterator<Item = [u8; 8]> + '_, Box<dyn Error>> {
    let (words, tail) = image.as_chunks::<8>();
    let (header, payload) = words.split_first().ok_or("転送データに語数がない。")?;
    if !tail.is_empty()
        || !(1..=MAX_WORDS).contains(&payload.len())
        || u64::from_le_bytes(*header) != payload.len() as u64
    {
        return Err("転送ヘッダの語数とデータ長が一致しないか、容量を超える。".into());
    }
    Ok(words.iter().copied())
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

fn flash(machine: &mut Machine, image: &[u8]) -> Result<(), Box<dyn Error>> {
    for _ in 0..uart::BIT_TICKS {
        machine.step(true)?;
    }
    for (index, bytes) in image_words(image)?.enumerate() {
        if !uart::send(bytes, |rx| -> Result<bool, Box<dyn Error>> {
            let ready = machine.step(rx)?;
            if machine.event.output()[194] && machine.pc() >= ENTRY {
                return Err("転送中にアプリの命令が退役した。".into());
            }
            Ok(ready)
        })? {
            return Err(format!(
                "転送語 {index} の ACK を受け取れない。PC={:#x}",
                machine.pc()
            )
            .into());
        }
        if index > 0 && index % 256 == 0 {
            println!("転送 {index} 語、tick {}", machine.tick);
        }
    }
    if machine.pc() >= ENTRY {
        return Err("転送内容の照合前にアプリが開始した。".into());
    }
    let program = machine.storage(15)?;
    let tail = machine.storage(10)?;
    for (offset, bytes) in image_words(image)?.skip(1).enumerate() {
        let address = ENTRY + offset as u64 * 8;
        let (bank, width) = if address < 0x50000 {
            (program, 15)
        } else {
            (tail, 10)
        };
        if memory_word(bank, address, width)? != u64::from_le_bytes(bytes) {
            return Err(format!("転送結果が RAM の {address:#x} で異なる。").into());
        }
    }
    println!(
        "転送完了、tick {}、全{}語一致",
        machine.tick,
        image.len() / 8 - 1
    );
    Ok(())
}

fn verify_execution(
    machine: &mut Machine,
    reference: &[u64],
    info: u64,
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
    let stack = machine.storage(11)?;
    let status = memory_word(stack, STATUS, 11)?;
    let information = memory_word(stack, INFORMATION, 11)?;
    if (status, information) != (1, info) {
        return Err(format!(
            "RAM の結果が QEMU と異なる。status={status:#x} information={information:#x}。"
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
    let [project, image, log, info, limit] = args.as_slice() else {
        return Err(
            "実行設定、UART 転送データ、QEMU ログ、期待結果（16進）、tick 上限を指定する。".into(),
        );
    };
    let info = u64::from_str_radix(
        info.to_str()
            .ok_or("結果には16進整数を指定する。")?
            .trim_start_matches("0x"),
        16,
    )?;
    let limit = limit
        .to_str()
        .ok_or("tick 上限には十進整数を指定する。")?
        .parse()?;
    let image = fs::read(image)?;
    let reference = reference_pcs(Path::new(log))?;
    let mut machine = Machine::new(uart::machine(Path::new(project))?, limit)?;
    flash(&mut machine, &image)?;
    verify_execution(&mut machine, &reference, info)
}
