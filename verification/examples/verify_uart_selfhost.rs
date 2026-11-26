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
        "転送完了、tick {}、全{}語一致",
        machine.tick,
        words.len() - 1
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
    let mut machine = Machine::new(uart::machine(Path::new(project))?, limit)?;
    flash(&mut machine, words)?;
    verify_execution(&mut machine, &reference, expected)
}
