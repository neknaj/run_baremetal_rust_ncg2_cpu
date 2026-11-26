//! UART ブート用 RAM の書き込み経路を実配線と独立な tick 仕様で照合する。

use circuitgame2_core::{
    netlist::ClockedVm,
    project::{
        ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, check_use_aliases,
        index_use_targets, parse_circuit_project,
    },
};
use std::{
    error::Error,
    io::{self, Write},
};

#[path = "../latch_smt.rs"]
mod latch_smt;
use latch_smt::LatchNetwork;

const MAP_DEPTH: usize = 31;
const REQUEST: usize = MAP_DEPTH + 1;
const ROUTED: usize = REQUEST + 1;

fn lower(module: &str) -> Result<ClockedVm, Box<dyn Error>> {
    let snapshots = [
        ("decode.ncg2", include_str!("../../../circuit/decode.ncg2")),
        (
            "arithmetic.ncg2",
            include_str!("../../../circuit/arithmetic.ncg2"),
        ),
        (
            "execute.ncg2",
            include_str!("../../../circuit/execute-boot.ncg2"),
        ),
        (
            "map-helpers.ncg2",
            include_str!("../../../circuit/selfhost-map.ncg2"),
        ),
        (
            "map.ncg2",
            include_str!("../../../circuit/uart-ram-map.ncg2"),
        ),
        ("ram.ncg2", include_str!("../../../circuit/uart-ram.ncg2")),
    ]
    .map(|(path, text)| SourceSnapshot {
        path,
        text,
        revision: 0,
    });
    lower_snapshots(&snapshots, module)
}

fn lower_snapshots(snapshots: &[SourceSnapshot<'_>], module: &str) -> Result<ClockedVm, Box<dyn Error>> {
    let project = parse_circuit_project(snapshots).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let (file, form) = project.circuits().iter().enumerate().find_map(|(file, circuit)| {
        circuit.names.iter().find(|name| name.name.word == module).map(|name| (file, name.form))
    }).ok_or("UART RAM の検証対象が見つからない。")?;
    let circuit = &project.circuits()[file];
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == form)
        .ok_or("UART RAM の検証対象の索引が見つからない。")?;
    let limits = ClockedModuleVmLimits {
        width: 65_536,
        wiring_bits: 1_048_576,
        gates: 1_048_576,
        state_bits: 8_388_608,
    };
    Ok(aliases
        .lower_clocked_vm(ModuleBodyPosition { file, entry }, limits)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("UART RAM を VM に展開できない。")?
        .vm)
}

/// アドレスを安定させた要求について、次 tick に RAM へ渡す入力を照合する。
fn emit(out: &mut impl Write, vm: &ClockedVm) -> Result<(), Box<dyn Error>> {
    let network = LatchNetwork::read(vm)?;
    if (network.external, network.nor.outputs.len()) != (129, 136) {
        return Err("UART RAM の入力経路が参照仕様の幅と異なる。".into());
    }
    writeln!(out, "(set-logic QF_BV)\n(set-option :timeout 60000)")?;
    writeln!(out, "(declare-const address (_ BitVec 64))")?;
    for tick in 1..=ROUTED {
        for bit in 0..network.external {
            if tick <= REQUEST && bit < 64 {
                writeln!(
                    out,
                    "(define-fun i_{tick}_{bit} () Bool (= ((_ extract {bit} {bit}) address) #b1))"
                )?;
            } else {
                writeln!(out, "(declare-const i_{tick}_{bit} Bool)")?;
            }
        }
    }
    network.emit_state(out, "bus", false)?;
    for tick in 1..=ROUTED {
        network.emit_step(out, "bus", tick)?;
    }
    writeln!(out, "(assert (or")?;
    for (bit, &wire) in network.nor.outputs[..128].iter().enumerate() {
        writeln!(
            out,
            "  (distinct {} i_{REQUEST}_{bit})",
            network.wire(wire, "bus", ROUTED, ROUTED)
        )?;
    }
    for (bank, (start, end)) in [
        (0x10000, 0x50000),
        (0x50000, 0x52000),
        (0xfff00, 0x100000),
        (0x100000, 0x104000),
    ]
    .into_iter()
    .enumerate()
    {
        let selected = format!(
            "(and (= ((_ extract 2 0) address) #b000) (bvuge address (_ bv{start} 64)) (bvult address (_ bv{end} 64)))"
        );
        let write = network.wire(network.nor.outputs[128 + bank], "bus", ROUTED, ROUTED);
        let read = network.wire(network.nor.outputs[132 + bank], "bus", REQUEST, REQUEST);
        writeln!(out, "  (distinct {write} (and i_{REQUEST}_128 {selected}))")?;
        writeln!(out, "  (distinct {read} {selected})")?;
    }
    writeln!(
        out,
        "))\n(check-sat-using (then simplify propagate-values solve-eqs bit-blast sat))"
    )?;
    Ok(())
}

/// 実行設定と同じ演算器・RAMを使い、ローダーから切り替える経路を展開する。
fn lower_handoff(module: &str) -> Result<ClockedVm, Box<dyn Error>> {
    use std::{fs, path::Path};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let config = circuitgame2_project_config::parse(&fs::read(root.join("uart-loader.toml"))?)
        .map_err(|error| format!("{error:?}"))?;
    let texts = config.sources.get_ref().iter()
        .map(|path| fs::read_to_string(root.join(path.get_ref()))).collect::<Result<Vec<_>, _>>()?;
    let snapshots: Vec<_> = config.sources.get_ref().iter().zip(&texts).map(|(path, text)| {
        SourceSnapshot { path: path.get_ref(), text, revision: 0 }
    }).collect();
    lower_snapshots(&snapshots, module)
}

/// 切替前後の配線が同じ tick のアドレス・データ・許可を使い、初回発行が一度だけ通ることを照合する。
fn emit_handoff(out: &mut impl Write, vm: &ClockedVm) -> Result<(), Box<dyn Error>> {
    const TICKS: usize = 64;
    let network = LatchNetwork::read(vm)?;
    if (network.external, network.nor.outputs.len()) != (259, 131) {
        return Err("切替回路の入出力幅が参照仕様と異なる。".into());
    }
    writeln!(out, "(set-logic QF_BV)\n(set-option :timeout 60000)")?;
    // 127 は確認区間に完了通知がない場合を表す。
    writeln!(out, "(declare-const finished (_ BitVec 8))")?;
    writeln!(out, "(assert (or (= finished #x7f) (and (bvuge finished #x18) (bvule finished #x24))))")?;
    network.emit_state(out, "mux", true)?;
    for tick in 1..=TICKS {
        for bit in 0..network.external {
            if bit == 129 {
                writeln!(out, "(define-fun i_{tick}_{bit} () Bool (bvuge (_ bv{tick} 8) finished))")?;
            } else {
                writeln!(out, "(declare-const i_{tick}_{bit} Bool)")?;
            }
        }
        // ローダーの最後の許可は完了通知の2 tick 以上前にある。CPU は発行の前に書き込まない。
        writeln!(out, "(assert (=> i_{tick}_128 (and (bvuge (_ bv{tick} 8) #x10) (bvult (_ bv{} 8) finished))))", tick + 1)?;
        writeln!(out, "(assert (=> i_{tick}_258 (bvuge (_ bv{tick} 8) (bvadd finished #x18))))")?;
        network.emit_step(out, "mux", tick)?;
    }
    writeln!(out, "(assert (or")?;
    for tick in 1..=TICKS {
        let read = |index| network.wire(network.nor.outputs[index], "mux", tick, tick);
        writeln!(out, "  (distinct {} (bvuge (_ bv{tick} 8) (bvadd finished #x08)))", read(129))?;
        writeln!(out, "  (distinct {} (= (_ bv{tick} 8) (bvadd finished #x15)))", read(130))?;
        let write = if let Some(input) = tick.checked_sub(6).filter(|&input| input != 0) {
            format!("(or i_{input}_128 i_{input}_258)")
        } else {
            "false".into()
        };
        writeln!(out, "  (distinct {} {write})", read(128))?;
        // 選択器が切り替わる間は許可が出ない。アドレスとデータは所有者が定まった区間で照合する。
        if tick > 6 {
            let input = tick - 6;
            for bit in 0..128 {
                let bus = read(bit);
                writeln!(out, "  (and (bvult (_ bv{tick} 8) (bvadd finished #x08)) (distinct {bus} i_{input}_{bit}))")?;
                let cpu_bit = 130 + bit;
                writeln!(out, "  (and (bvuge (_ bv{tick} 8) (bvadd finished #x10)) (distinct {bus} i_{input}_{cpu_bit}))")?;
            }
        }
    }
    writeln!(out, "))\n(check-sat-using (then simplify propagate-values solve-eqs bit-blast sat))")?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    if std::env::args().nth(1).as_deref() == Some("--handoff") {
        return emit_handoff(
            &mut io::BufWriter::new(io::stdout().lock()),
            &lower_handoff("uart-loader-route")?,
        );
    }
    emit(
        &mut io::BufWriter::new(io::stdout().lock()),
        &lower("uart-ram-route")?,
    )
}

#[cfg(test)]
mod tests {
    use super::{MAP_DEPTH, lower, lower_handoff};
    use circuitgame2_core::netlist::ClockedVm;

    fn tick(vm: &mut ClockedVm, address: u64, value: u64, write: bool) -> u64 {
        let mut input = [false; 129];
        for bit in 0..64 {
            input[bit] = address & (1_u64 << bit) != 0;
            input[64 + bit] = value & (1_u64 << bit) != 0;
        }
        input[128] = write;
        vm.step(&input)
            .unwrap()
            .iter()
            .enumerate()
            .fold(0, |word, (bit, &value)| word | (u64::from(value) << bit))
    }

    fn read(vm: &mut ClockedVm, address: u64) -> u64 {
        let mut word = 0;
        for _ in 0..MAP_DEPTH + 16 {
            word = tick(vm, address, 0, false);
        }
        word
    }

    fn stored(vm: &ClockedVm, address: u64, address_bits: usize) -> u64 {
        let bank = (0..vm.storage_count())
            .find(|&index| {
                vm.storage_wiring(index)
                    .and_then(|(wiring, _)| wiring.address.as_ref())
                    .is_some_and(|pins| pins.len() == address_bits)
            })
            .unwrap();
        let index = ((address >> 3) as usize) & ((1 << address_bits) - 1);
        vm.ram_memory(bank).unwrap()[index * 64..(index + 1) * 64]
            .iter()
            .enumerate()
            .fold(0, |word, (bit, &value)| word | (u64::from(value) << bit))
    }

    #[test]
    fn pipelined_loader_writes_to_the_same_ncg2_ram_port() {
        use ncg2_uart_loader::{BootLoader, Receive, ENTRY_ADDRESS};
        let mut vm = lower("uart-ram").unwrap();
        let mut loader = BootLoader::reset();
        let words = [0xa6f0_1cde_3227_1917, 0x8777, 0xff04_a1ab_a5d8_77dc];
        let mut sent = 0;
        let mut confirmed = 0;
        let mut credit = 0;
        for cycle in 0..200 {
            let input = if cycle == 0 {
                Receive::Word(words.len() as u64)
            } else if credit != 0 && sent < words.len() {
                let word = words[sent];
                sent += 1;
                Receive::Word(word)
            } else {
                Receive::Idle
            };
            let frame = loader.step(input);
            credit = frame.credit;
            let (address, data) = frame.bus.map_or((0, 0), |word| (word.address, word.data));
            tick(&mut vm, address, data, frame.write);
            if let Some(word) = frame.committed {
                assert_eq!(word.address, ENTRY_ADDRESS + 8 * confirmed as u64);
                assert_eq!(word.data, words[confirmed]);
                confirmed += 1;
            }
            for (i, &word) in words.iter().enumerate() {
                let expected = if i < confirmed { word } else { 0 };
                assert_eq!(stored(&vm, ENTRY_ADDRESS + 8 * i as u64, 15), expected, "tick {cycle}");
            }
            if frame.cpu_ready {
                assert_eq!(confirmed, words.len());
                return;
            }
        }
        panic!("全語を確定して RAM を CPU へ渡すまでの上限を超えた");
    }

    fn continuous_wire(words: &[u64]) -> impl Iterator<Item = bool> + '_ {
        let wire = [words.len() as u64].into_iter().chain(words.iter().copied())
            .flat_map(u64::to_le_bytes).flat_map(|byte| {
                std::iter::once(false)
                    .chain((0..8).map(move |bit| (byte >> bit) & 1 != 0))
                    .chain(std::iter::once(true))
                    .flat_map(|level| std::iter::repeat_n(level, 16))
            });
        std::iter::repeat_n(true, 16).chain(wire).chain(std::iter::repeat(true))
    }

    fn read_word(bits: &[bool]) -> u64 {
        bits.iter().enumerate().fold(0, |word, (bit, &value)| {
            word | (u64::from(value) << bit)
        })
    }

    #[test]
    fn continuous_uart_handoff_starts_the_cpu_after_the_physical_ram_writes() {
        use ncg2_uart_loader::{Acceptance, BootLoader, ENTRY_ADDRESS, Receive};
        let mut vm = lower_handoff("uart-loader-handoff").unwrap();
        let mut receiver = lower_handoff("uart-loader-receive").unwrap();
        let mut receiver_output = receiver.reset();
        let mut loader = BootLoader::reset();
        let mut delivered = false;
        let mut expected_requests = Vec::new();
        let words = [0x02a0_0313_0001_02b7, 0x0000_006f_0262_b023];
        let probe = ENTRY_ADDRESS + 32;
        let mut requests = Vec::new();
        let mut last_address = 0;
        let mut address_ticks = 0;
        let mut finished = None;
        let mut owner_tick = None;
        let mut first_issue = None;
        let mut retirements = Vec::new();

        for (index, rx) in continuous_wire(&words).take(12_000).enumerate() {
            let cycle = index + 1;
            assert!(!receiver_output[65], "tick {cycle}: 受信器がエラーを返した");
            let input = if receiver_output[64] && !delivered {
                Receive::Word(read_word(&receiver_output[..64]))
            } else {
                Receive::Idle
            };
            let step = loader.step(input);
            assert!(!matches!(step.accepted, Acceptance::Rejected(_)));
            let ack = matches!(step.accepted, Acceptance::Header | Acceptance::Payload);
            delivered = receiver_output[64] && (delivered || ack);
            receiver_output = receiver.step(&[rx, ack]).unwrap();
            if step.write { expected_requests.push(cycle + 6); }
            if step.cpu_ready && finished.is_none() { finished = Some(cycle); }
            let (address, data) = step.bus.map_or((0, 0), |word| (word.address, word.data));
            let mut pins = [false; 194];
            for bit in 0..64 {
                pins[bit] = (probe >> bit) & 1 != 0;
                pins[64 + bit] = (address >> bit) & 1 != 0;
                pins[128 + bit] = (data >> bit) & 1 != 0;
            }
            pins[192] = step.write;
            pins[193] = step.cpu_ready;
            let output = vm.step(&pins).unwrap();
            let bus_address = read_word(&output[64..128]);
            if bus_address == last_address {
                address_ticks += 1;
            } else {
                last_address = bus_address;
                address_ticks = 1;
            }
            let owner = output[192];
            let issue = output[193];
            let commit = output[194];
            let stopped = output[195];
            if output[196] {
                assert!(address_ticks >= 32, "tick {cycle}: アドレス保持が足りない");
                if !owner {
                    requests.push((cycle, bus_address));
                }
            }
            assert!(!stopped, "tick {cycle}: 起動中に停止した");
            if !owner {
                assert!(!issue && !commit, "tick {cycle}: RAM 引き渡し前に命令を実行した");
                for (index, &data) in words.iter().enumerate() {
                    let address = ENTRY_ADDRESS + 8 * index as u64;
                    let expected = requests.get(index).filter(|(request, _)| cycle >= request + 3).map_or(0, |_| data);
                    assert_eq!(stored(&vm, address, 15), expected, "tick {cycle}: RAM {address:#x}");
                }
                continue;
            }
            if owner_tick.is_none() {
                owner_tick = Some(cycle);
                assert_eq!(Some(cycle), finished.map(|tick| tick + 8));
                assert!(requests.iter().all(|(request, _)| request + 3 < cycle));
                for (index, &data) in words.iter().enumerate() {
                    assert_eq!(stored(&vm, ENTRY_ADDRESS + 8 * index as u64, 15), data);
                }
            }
            if issue && first_issue.is_none() {
                first_issue = Some(cycle);
                assert_eq!(Some(cycle), owner_tick.map(|tick| tick + 16));
            }
            if commit { retirements.push((cycle, read_word(&output[..64]))); }
            if retirements.len() >= 3 && stored(&vm, probe, 15) == 42 {
                assert_eq!(requests.iter().map(|(tick, _)| *tick).collect::<Vec<_>>(), expected_requests);
                assert_eq!(requests.iter().map(|(_, address)| *address).collect::<Vec<_>>(), [ENTRY_ADDRESS, ENTRY_ADDRESS + 8]);
                assert_eq!(Some(retirements[0].0), first_issue.map(|tick| tick + 1033));
                assert_eq!(retirements.iter().map(|(_, pc)| *pc).collect::<Vec<_>>(), [ENTRY_ADDRESS, ENTRY_ADDRESS + 4, ENTRY_ADDRESS + 8]);
                eprintln!("write {requests:?}, finished {finished:?}, owner {owner_tick:?}, issue {first_issue:?}, retirements {retirements:?}, result tick {cycle}");
                return;
            }
        }
        panic!("ローダーから RAM を引き渡しても、CPU が期待する値を書き込まなかった");
    }

    #[test]
    fn independent_banks_and_wrapped_indices_share_the_cpu_read_and_write_port() {
        let mut vm = lower("uart-ram").unwrap();
        let words = [
            (0x10000, 0x9988, 15),
            (0x3fff8, 0x8877, 15),
            (0x40000, 0x7766, 15),
            (0x50000, 0x6655, 10),
            (0xfff00, 0x5544, 5),
            (0x103ff8, 0x4433, 11),
        ];
        for (address, value, address_bits) in words {
            assert_eq!(read(&mut vm, address), 0);
            tick(&mut vm, address, value, true);
            tick(&mut vm, address, value + 1, true);
            assert_eq!(stored(&vm, address, address_bits), 0);
            // 確定を待つ間にアドレスとデータを変えても、先のフレームを書き込む。
            tick(&mut vm, 0x200010, u64::MAX, false);
            assert_eq!(stored(&vm, address, address_bits), value);
            tick(&mut vm, 0x200010, u64::MAX, false);
            assert_eq!(stored(&vm, address, address_bits), value + 1);
        }
        for invalid in [0xffff, 0x10001, 0x52000, 0x104000, 0x200010] {
            assert_eq!(read(&mut vm, invalid), 0);
            tick(&mut vm, invalid, u64::MAX, true);
        }
        for (address, value, _) in words {
            assert_eq!(read(&mut vm, address), value + 1);
        }
    }
}
