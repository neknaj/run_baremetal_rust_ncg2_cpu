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
    let file = snapshots.len() - 1;
    let circuit = &project.circuits()[file];
    let form = circuit
        .names
        .iter()
        .find(|name| name.name.word == module)
        .ok_or("UART RAM の検証対象が見つからない。")?
        .form;
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

fn main() -> Result<(), Box<dyn Error>> {
    emit(
        &mut io::BufWriter::new(io::stdout().lock()),
        &lower("uart-ram-route")?,
    )
}

#[cfg(test)]
mod tests {
    use super::{MAP_DEPTH, lower, lower_snapshots};
    use circuitgame2_core::project::SourceSnapshot;
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

    #[test]
    fn continuous_rx_loads_every_word_without_a_sender_ack() {
        use ncg2_uart_loader::{Acceptance, BootLoader, ENTRY_ADDRESS, Receive};

        let snapshots = [
            ("rx.ncg2", include_str!("../../../circuit/uart-rx.ncg2")),
            ("word.ncg2", include_str!("../../../circuit/uart-word-buffer.ncg2")),
            ("receiver.ncg2", "(using nor)
                (namespace rv64
                  (export raw-receiver)
                  (module raw-receiver (in rx ack) (out (word (bits 64)) ready error)
                    (def (byte valid framing-error) (uart-rx rx))
                    (def (word ready error) (uart-word-buffer byte valid framing-error ack))))"),
        ].map(|(path, text)| SourceSnapshot { path, text, revision: 0 });
        let mut receiver = lower_snapshots(&snapshots, "raw-receiver").unwrap();
        let mut output = receiver.reset();
        let mut ram = lower("uart-ram").unwrap();
        let mut loader = BootLoader::reset();
        let words = [0x7168_c033_14c4_f37a, 0xf0ff_0133_6789_9421, 0x0000_006f_0262_b023];
        let wire = [words.len() as u64].into_iter().chain(words).flat_map(u64::to_le_bytes).flat_map(|byte| {
            std::iter::once(false)
                .chain((0..8).map(move |bit| (byte >> bit) & 1 != 0))
                .chain(std::iter::once(true))
                .flat_map(|level| std::iter::repeat_n(level, 16))
        });

        let mut delivered = false;
        let mut confirmed = 0;
        let mut ready = false;
        for rx in std::iter::repeat_n(true, 16).chain(wire).chain(std::iter::repeat_n(true, 80)) {
            assert!(!output[65]);
            let input = if output[64] && !delivered {
                let word = output[..64].iter().enumerate().fold(0, |word, (bit, &value)| {
                    word | (u64::from(value) << bit)
                });
                Receive::Word(word)
            } else {
                Receive::Idle
            };
            let frame = loader.step(input);
            let ack = matches!(frame.accepted, Acceptance::Header | Acceptance::Payload);
            assert!(!matches!(frame.accepted, Acceptance::Rejected(_)));
            delivered = output[64] && (delivered || ack);
            output = receiver.step(&[rx, ack]).unwrap();

            let (address, data) = frame.bus.map_or((0, 0), |word| (word.address, word.data));
            tick(&mut ram, address, data, frame.write);
            if let Some(word) = frame.committed {
                assert_eq!(word.address, ENTRY_ADDRESS + 8 * confirmed as u64);
                assert_eq!(word.data, words[confirmed]);
                assert_eq!(stored(&ram, word.address, 15), word.data);
                confirmed += 1;
            }
            if frame.cpu_ready {
                assert_eq!(confirmed, words.len());
                ready = true;
            }
        }
        assert!(ready);
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
