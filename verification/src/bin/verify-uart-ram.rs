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
    let project = parse_circuit_project(&snapshots).map_err(|error| format!("{error:?}"))?;
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
    use super::{MAP_DEPTH, lower};
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
