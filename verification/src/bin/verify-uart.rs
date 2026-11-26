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

const RESET_IDLE: usize = 16;
const FIRST: usize = RESET_IDLE + 1;

#[derive(Clone, Copy)]
struct Timing {
    module: &'static str,
    bit_ticks: usize,
    report_offset: usize,
}

impl Timing {
    const STANDARD: Self = Self { module: "uart-rx", bit_ticks: 16, report_offset: 159 };
    const FASTER: Self = Self { module: "uart-rx-8", bit_ticks: 8, report_offset: 87 };

    fn frame_ticks(self) -> usize {
        10 * self.bit_ticks
    }

    fn sample(self, start: usize, bit: usize) -> usize {
        start + bit * self.bit_ticks + self.bit_ticks / 2
    }
}

fn lower(timing: Timing) -> Result<ClockedVm, Box<dyn Error>> {
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
        .find(|name| name.name.word == timing.module)
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

// 二つの開始位置を指定する。各候補の直前を除き、RX のサンプルは任意である。
fn fixed_input(tick: usize, second: usize) -> Option<bool> {
    match tick {
        0 | FIRST => Some(false),
        1..FIRST => Some(true),
        tick if tick == second => Some(false),
        tick if (second - 4..second).contains(&tick) => Some(true),
        _ => None,
    }
}

// 実回路の状態とは独立した、RX のサンプルと報告時刻だけで書く参照仕様。
fn expected(timing: Timing, tick: usize, output: usize, second: usize) -> String {
    let accepted = |start| format!("(not i_{}_0)", timing.sample(start, 0));
    let valid = |start| format!("(and {} i_{}_0)", accepted(start), timing.sample(start, 9));
    let failed = |start| format!("(and {} (not i_{}_0))", accepted(start), timing.sample(start, 9));
    match output {
        0..8 => [FIRST, second]
            .into_iter()
            .filter(|&start| tick >= start + timing.report_offset)
            .fold("false".into(), |held, start| {
                format!(
                    "(ite {} i_{}_0 {held})",
                    valid(start),
                    timing.sample(start, output + 1)
                )
            }),
        8 => [FIRST, second]
            .into_iter()
            .find(|&start| tick == start + timing.report_offset)
            .map_or("false".into(), valid),
        9 => [FIRST, second]
            .into_iter()
            .find(|&start| tick == start + timing.report_offset)
            .map_or("false".into(), failed),
        _ => unreachable!("UART の公開出力は10ビット"),
    }
}

fn emit(out: &mut impl Write, vm: &ClockedVm, timing: Timing, gap: usize) -> Result<(), Box<dyn Error>> {
    let second = FIRST + timing.frame_ticks() + gap;
    let end = second + timing.frame_ticks() + timing.bit_ticks;
    let network = LatchNetwork::read(vm)?;
    if (network.external, network.nor.outputs.len()) != (1, 10) {
        return Err("UART の入出力幅が参照仕様と異なる。".into());
    }
    writeln!(out, "(set-logic QF_BV)")?;
    writeln!(out, "(set-option :timeout 30000)")?;
    for tick in 0..=end {
        if let Some(value) = fixed_input(tick, second) {
            writeln!(out, "(define-fun i_{tick}_0 () Bool {value})")?;
        } else {
            writeln!(out, "(declare-const i_{tick}_0 Bool)")?;
        }
    }
    network.emit_state(out, "uart", true)?;
    for tick in 1..=end {
        network.emit_step(out, "uart", tick)?;
    }
    writeln!(out, "(assert (or")?;
    for tick in 0..=end {
        for (index, &wire) in network.nor.outputs.iter().enumerate() {
            let observed = network.wire(wire, "uart", tick, tick);
            let expected = expected(timing, tick, index, second);
            writeln!(out, "  (distinct {observed} {expected})")?;
        }
    }
    writeln!(out, "))")?;
    writeln!(out, "(check-sat)")?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let (timing, gap) = match args.as_slice() {
        [] => (Timing::STANDARD, 0),
        [option] if option == "--8-tick" => (Timing::FASTER, 0),
        [option] if option == "--8-tick-recovery" => (Timing::FASTER, 4),
        _ => return Err("設定なし、--8-tick、または --8-tick-recovery を指定する。".into()),
    };
    emit(&mut io::BufWriter::new(io::stdout().lock()), &lower(timing)?, timing, gap)
}

#[cfg(test)]
mod tests {
    use super::{RESET_IDLE, Timing, lower};

    fn idle(rx: &mut Vec<bool>, ticks: usize) {
        rx.extend(std::iter::repeat_n(true, ticks));
    }

    fn send(rx: &mut Vec<bool>, timing: Timing, byte: u8, stop: bool) -> usize {
        let report = rx.len() + timing.report_offset + 1;
        for bit in [false]
            .into_iter()
            .chain((0..8).map(|index| byte & (1 << index) != 0))
            .chain([stop])
        {
            rx.extend(std::iter::repeat_n(bit, timing.bit_ticks));
        }
        report
    }

    fn check(timing: Timing, rx: &[bool], reports: &[(usize, Option<u8>)]) {
        let mut vm = lower(timing).unwrap();
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
            assert_eq!(vm.step(&[bit]).unwrap(), expected, "tick {tick}, rate {}", timing.bit_ticks);
        }
    }

    #[test]
    fn receives_adjacent_frames_and_holds_the_last_byte() {
        for timing in [Timing::STANDARD, Timing::FASTER] {
            let mut rx = Vec::new();
            idle(&mut rx, RESET_IDLE);
            let first = send(&mut rx, timing, 0x81, true);
            let second = send(&mut rx, timing, 0x42, true);
            assert_eq!(second - first, timing.frame_ticks());
            idle(&mut rx, timing.bit_ticks);
            check(timing, &rx, &[(first, Some(0x81)), (second, Some(0x42))]);
        }
    }

    #[test]
    fn ignores_a_falling_edge_after_the_stop_sample() {
        for timing in [Timing::STANDARD, Timing::FASTER] {
            let mut rx = Vec::new();
            idle(&mut rx, RESET_IDLE);
            let start = rx.len();
            let first = send(&mut rx, timing, 0, true);
            let noise = timing.sample(start, 9) + 1;
            rx[noise] = false;
            // 次の開始候補には直前の High が4 tick 必要である。
            let gap = (noise + 5).saturating_sub(rx.len());
            idle(&mut rx, gap);
            let second = send(&mut rx, timing, 0xa5, true);
            idle(&mut rx, timing.bit_ticks);
            check(timing, &rx, &[(first, Some(0)), (second, Some(0xa5))]);
        }
    }

    #[test]
    fn rejects_false_start_and_recovers_after_framing_error() {
        for timing in [Timing::STANDARD, Timing::FASTER] {
            let mut rx = Vec::new();
            idle(&mut rx, RESET_IDLE);
            rx.push(false);
            idle(&mut rx, timing.frame_ticks() - 1);
            let accepted = send(&mut rx, timing, 0x5a, true);
            let rejected = send(&mut rx, timing, 0xff, false);
            idle(&mut rx, 4);
            let recovered = send(&mut rx, timing, 0x18, true);
            idle(&mut rx, timing.bit_ticks);
            check(
                timing,
                &rx,
                &[
                    (accepted, Some(0x5a)),
                    (rejected, None),
                    (recovered, Some(0x18)),
                ],
            );
        }
    }
}
