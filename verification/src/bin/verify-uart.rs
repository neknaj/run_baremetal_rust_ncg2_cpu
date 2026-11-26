//! 展開した UART 受信器の公開出力を、RX の中点採取の仕様と照合する。

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

const BIT_TICKS: usize = 16;
const FRAME_TICKS: usize = 10 * BIT_TICKS;
const RESULT: usize = FRAME_TICKS - 1;
const FIRST: usize = BIT_TICKS + 1;
const SECOND: usize = FIRST + FRAME_TICKS;
const END: usize = SECOND + FRAME_TICKS + BIT_TICKS;

fn lower() -> Result<ClockedVm, Box<dyn Error>> {
    let sources = [
        SourceSnapshot {
            path: "basis.ncg2",
            text: "(using nor)",
            revision: 0,
        },
        SourceSnapshot {
            path: "uart-rx.ncg2",
            text: include_str!("../../../circuit/uart-rx.ncg2"),
            revision: 0,
        },
    ];
    let project = parse_circuit_project(&sources).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let circuit = &project.circuits()[1];
    let module = circuit
        .names
        .iter()
        .find(|name| name.name.word == "uart-rx")
        .ok_or("UART 受信器が見つからない。")?;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == module.form)
        .ok_or("UART 受信器の索引が見つからない。")?;
    let limits = ClockedModuleVmLimits {
        width: 64,
        wiring_bits: 1024,
        gates: 64,
        state_bits: 256,
    };
    let compiled = aliases
        .lower_clocked_vm(ModuleBodyPosition { file: 1, entry }, limits)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("UART 受信器を VM に展開できない。")?;
    Ok(compiled.vm)
}

// 連続する二つの開始位置を指定する。開始候補直前の区間以外は、サンプル間も含めて任意。
fn fixed_input(tick: usize) -> Option<bool> {
    match tick {
        0 | FIRST | SECOND => Some(false),
        1..FIRST => Some(true),
        tick if (SECOND - 4..SECOND).contains(&tick) => Some(true),
        _ => None,
    }
}

fn sample(start: usize, bit: usize) -> usize {
    start + bit * BIT_TICKS + BIT_TICKS / 2
}

// 実回路の状態とは独立した、RX のサンプルと報告時刻だけで書く参照仕様。
fn expected(tick: usize, output: usize) -> String {
    let accepted = |start| format!("(not i_{}_0)", sample(start, 0));
    let valid = |start| format!("(and {} i_{}_0)", accepted(start), sample(start, 9));
    let failed = |start| format!("(and {} (not i_{}_0))", accepted(start), sample(start, 9));
    match output {
        0..8 => [FIRST, SECOND]
            .into_iter()
            .filter(|&start| tick >= start + RESULT)
            .fold("false".into(), |held, start| {
                format!(
                    "(ite {} i_{}_0 {held})",
                    valid(start),
                    sample(start, output + 1)
                )
            }),
        8 => [FIRST, SECOND]
            .into_iter()
            .find(|&start| tick == start + RESULT)
            .map_or("false".into(), valid),
        9 => [FIRST, SECOND]
            .into_iter()
            .find(|&start| tick == start + RESULT)
            .map_or("false".into(), failed),
        _ => unreachable!("UART の公開出力は10ビット"),
    }
}

fn emit(out: &mut impl Write, vm: &ClockedVm) -> Result<(), Box<dyn Error>> {
    let network = LatchNetwork::read(vm)?;
    if (network.external, network.nor.outputs.len()) != (1, 10) {
        return Err("UART の入出力幅が参照仕様と異なる。".into());
    }
    writeln!(out, "(set-logic QF_BV)")?;
    writeln!(out, "(set-option :timeout 30000)")?;
    for tick in 0..=END {
        if let Some(value) = fixed_input(tick) {
            writeln!(out, "(define-fun i_{tick}_0 () Bool {value})")?;
        } else {
            writeln!(out, "(declare-const i_{tick}_0 Bool)")?;
        }
    }
    network.emit_state(out, "uart", true)?;
    for tick in 1..=END {
        network.emit_step(out, "uart", tick)?;
    }
    writeln!(out, "(assert (or")?;
    for tick in 0..=END {
        for (index, &wire) in network.nor.outputs.iter().enumerate() {
            let observed = network.wire(wire, "uart", tick, tick);
            let expected = expected(tick, index);
            writeln!(out, "  (distinct {observed} {expected})")?;
        }
    }
    writeln!(out, "))")?;
    writeln!(out, "(check-sat)")?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    emit(&mut io::BufWriter::new(io::stdout().lock()), &lower()?)
}

#[cfg(test)]
mod tests {
    use super::{BIT_TICKS, FRAME_TICKS, RESULT, lower};

    fn idle(rx: &mut Vec<bool>, ticks: usize) {
        rx.extend(std::iter::repeat_n(true, ticks));
    }

    fn send(rx: &mut Vec<bool>, byte: u8, stop: bool) -> usize {
        let report = rx.len() + RESULT + 1;
        for bit in [false]
            .into_iter()
            .chain((0..8).map(|index| byte & (1 << index) != 0))
            .chain([stop])
        {
            rx.extend(std::iter::repeat_n(bit, BIT_TICKS));
        }
        report
    }

    fn check(rx: &[bool], reports: &[(usize, Option<u8>)]) {
        let mut vm = lower().unwrap();
        assert_eq!(vm.reset(), vec![false; 10]);
        let mut held = 0;
        for (index, &bit) in rx.iter().enumerate() {
            let tick = index + 1;
            let event = reports
                .iter()
                .find(|(when, _)| *when == tick)
                .map(|(_, byte)| *byte);
            if let Some(Some(byte)) = event {
                held = byte;
            }
            let expected: Vec<_> = (0..8)
                .map(|index| held & (1 << index) != 0)
                .chain([
                    event.is_some_and(|byte| byte.is_some()),
                    event == Some(None),
                ])
                .collect();
            assert_eq!(vm.step(&[bit]).unwrap(), expected, "tick {tick}");
        }
    }

    #[test]
    fn receives_adjacent_frames_and_holds_the_last_byte() {
        let mut rx = Vec::new();
        idle(&mut rx, BIT_TICKS);
        let first = send(&mut rx, 0x81, true);
        let second = send(&mut rx, 0x42, true);
        assert_eq!(second - first, FRAME_TICKS);
        idle(&mut rx, BIT_TICKS);
        check(&rx, &[(first, Some(0x81)), (second, Some(0x42))]);
    }

    #[test]
    fn ignores_a_falling_edge_after_the_stop_sample() {
        let mut rx = Vec::new();
        idle(&mut rx, BIT_TICKS);
        let start = rx.len();
        let first = send(&mut rx, 0, true);
        rx[start + 9 * BIT_TICKS + BIT_TICKS / 2 + 1] = false;
        let second = send(&mut rx, 0xa5, true);
        idle(&mut rx, BIT_TICKS);
        check(&rx, &[(first, Some(0)), (second, Some(0xa5))]);
    }

    #[test]
    fn rejects_false_start_and_recovers_after_framing_error() {
        let mut rx = Vec::new();
        idle(&mut rx, BIT_TICKS);
        rx.push(false);
        idle(&mut rx, FRAME_TICKS - 1);
        let accepted = send(&mut rx, 0x5a, true);
        let rejected = send(&mut rx, 0xff, false);
        idle(&mut rx, 4);
        let recovered = send(&mut rx, 0x18, true);
        idle(&mut rx, BIT_TICKS);
        check(
            &rx,
            &[
                (accepted, Some(0x5a)),
                (rejected, None),
                (recovered, Some(0x18)),
            ],
        );
    }
}
