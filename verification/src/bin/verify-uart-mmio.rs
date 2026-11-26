//! UART MMIO の展開済み配線を、全アドレスの読出しと ACK の仕様に照合する。

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

const STATUS: u64 = 0x200000;
const DATA: u64 = 0x200008;
const ACK: u64 = 0x200010;
const READ_LATENCY: usize = 19;
const ACK_DECODE_LATENCY: usize = 17;
const FLAGS_LATENCY: usize = 4;
const HORIZON: usize = READ_LATENCY + 1;

fn lower(target: &str) -> Result<ClockedVm, Box<dyn Error>> {
    let sources = [
        ("basis.ncg2", "(using nor)"),
        ("rx.ncg2", include_str!("../../../circuit/uart-rx.ncg2")),
        (
            "word.ncg2",
            include_str!("../../../circuit/uart-word-buffer.ncg2"),
        ),
        (
            "mmio.ncg2",
            include_str!("../../../circuit/uart-boot-mmio.ncg2"),
        ),
    ]
    .map(|(path, text)| SourceSnapshot {
        path,
        text,
        revision: 0,
    });
    let project = parse_circuit_project(&sources).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let file = sources.len() - 1;
    let circuit = &project.circuits()[file];
    let module = circuit
        .names
        .iter()
        .find(|name| name.name.word == target)
        .ok_or("UART MMIO の検証対象が見つからない。")?;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == module.form)
        .ok_or("UART MMIO の検証対象の索引が見つからない。")?;
    let limits = ClockedModuleVmLimits {
        width: 64,
        wiring_bits: 32768,
        gates: 2048,
        state_bits: 2048,
    };
    let compiled = aliases
        .lower_clocked_vm(ModuleBodyPosition { file, entry }, limits)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("UART MMIO の検証対象を VM に展開できない。")?;
    Ok(compiled.vm)
}

fn emit_inputs(out: &mut impl Write, tick: usize, width: usize) -> io::Result<()> {
    for input in 0..width {
        if tick == 0 {
            writeln!(out, "(define-fun i_{tick}_{input} () Bool false)")?;
        } else {
            writeln!(out, "(declare-const i_{tick}_{input} Bool)")?;
        }
    }
    let address = (0..64)
        .rev()
        .map(|bit| format!("(ite i_{tick}_{bit} #b1 #b0)"))
        .reduce(|high, low| format!("(concat {high} {low})"))
        .expect("アドレスは64ビット");
    writeln!(
        out,
        "(define-fun address_{tick} () (_ BitVec 64) {address})"
    )
}

// アドレスとの完全一致で、読出し先と ACK を決める。受信値は独立の入力とする。
fn expected(tick: usize, output: usize) -> String {
    let received = tick.saturating_sub(FLAGS_LATENCY);
    match output {
        0..64 if tick > READ_LATENCY => {
            let address = tick - READ_LATENCY;
            let status = match output {
                0 => format!("i_{received}_129"),
                1 => format!("i_{received}_130"),
                _ => "false".into(),
            };
            let data_bit = 65 + output;
            format!(
                "(or (and (= address_{address} #x{STATUS:016x}) {status})                      (and (= address_{address} #x{DATA:016x}) i_{received}_{data_bit}))"
            )
        }
        64 => format!("i_{received}_129"),
        65 => format!("i_{received}_130"),
        66 if tick > ACK_DECODE_LATENCY => {
            let frame = tick - ACK_DECODE_LATENCY;
            format!("(and (= address_{frame} #x{ACK:016x}) i_{frame}_64)")
        }
        _ => "false".into(),
    }
}

fn compare(
    out: &mut impl Write,
    network: &LatchNetwork<'_>,
    prefix: &str,
    tick: usize,
) -> io::Result<()> {
    for (index, &wire) in network.nor.outputs.iter().enumerate() {
        let observed = network.wire(wire, prefix, tick, tick);
        writeln!(out, "  (distinct {observed} {})", expected(tick, index))?;
    }
    Ok(())
}

fn emit(out: &mut impl Write, vm: &ClockedVm) -> Result<(), Box<dyn Error>> {
    let network = LatchNetwork::read(vm)?;
    if (network.external, network.nor.outputs.len()) != (131, 67) {
        return Err("UART MMIO の入出力幅が参照仕様と異なる。".into());
    }
    writeln!(out, "(set-logic QF_BV)")?;
    writeln!(out, "(set-option :timeout 30000)")?;
    for tick in 0..=HORIZON {
        emit_inputs(out, tick, network.external)?;
    }
    network.emit_state(out, "reset", true)?;
    network.emit_state(out, "window", false)?;
    for tick in 1..=HORIZON {
        network.emit_step(out, "reset", tick)?;
        network.emit_step(out, "window", tick)?;
    }
    writeln!(out, "(assert (or")?;
    for tick in 0..HORIZON {
        compare(out, &network, "reset", tick)?;
    }
    // 任意の初期内部状態から20tick進めれば、同じ窓を任意の実行時刻に適用できる。
    compare(out, &network, "window", HORIZON)?;
    writeln!(out, "))")?;
    writeln!(
        out,
        "(check-sat-using (then simplify propagate-values solve-eqs bit-blast sat))"
    )?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    emit(
        &mut io::BufWriter::new(io::stdout().lock()),
        &lower("uart-mmio-port")?,
    )
}

#[cfg(test)]
mod tests {
    use super::{ACK, DATA, READ_LATENCY, STATUS, lower};
    use circuitgame2_core::netlist::ClockedVm;

    const BIT_TICKS: usize = 16;
    const UART_ACK_LATENCY: usize = 35;
    const DRAIN: usize = 40;
    const PAYLOAD: [u8; 8] = [0x03, 0x17, 0x2a, 0xbc, 0x57, 0x69, 0x7c, 0x8f];

    fn frame(vm: &mut ClockedVm, rx: bool, address: u64, write: bool) -> (u64, bool, bool) {
        let input: Vec<_> = [rx]
            .into_iter()
            .chain((0..64).map(|bit| address & (1 << bit) != 0))
            .chain([write])
            .collect();
        let output = vm.step(&input).unwrap();
        let data = output[..64]
            .iter()
            .enumerate()
            .fold(0, |word, (bit, &value)| word | u64::from(value) << bit);
        (data, output[64], output[65])
    }

    fn idle(vm: &mut ClockedVm, ticks: usize, address: u64) -> (u64, bool, bool) {
        let mut result = frame(vm, true, address, false);
        for _ in 1..ticks {
            result = frame(vm, true, address, false);
        }
        result
    }

    fn send(vm: &mut ClockedVm, byte: u8, stop: bool) {
        let data = (0..8).map(|bit| byte & (1 << bit) != 0);
        for rx in [false].into_iter().chain(data).chain([stop]) {
            for _ in 0..BIT_TICKS {
                frame(vm, rx, STATUS, false);
            }
        }
    }

    #[test]
    fn real_uart_data_remains_until_a_store_to_the_exact_ack_address() {
        let mut vm = lower("uart-boot-mmio").unwrap();
        assert_eq!(vm.reset(), vec![false; 66]);
        idle(&mut vm, BIT_TICKS, STATUS);
        for byte in PAYLOAD {
            send(&mut vm, byte, true);
        }
        assert_eq!(idle(&mut vm, DRAIN, STATUS), (1, true, false));

        let expected = u64::from_le_bytes(PAYLOAD);
        assert_eq!(
            idle(&mut vm, READ_LATENCY + 1, DATA),
            (expected, true, false)
        );
        assert_eq!(idle(&mut vm, READ_LATENCY + 1, ACK), (0, true, false));
        for wrong in [STATUS, DATA, ACK + 1, ACK | (1 << 63)] {
            frame(&mut vm, true, ACK, false);
            frame(&mut vm, true, wrong, true);
            frame(&mut vm, true, ACK, false);
            assert_eq!(idle(&mut vm, DRAIN, DATA), (expected, true, false));
        }

        frame(&mut vm, true, ACK, true);
        for since_ack in 1..UART_ACK_LATENCY {
            assert!(frame(&mut vm, true, STATUS, false).1, "ACK+{since_ack}");
        }
        assert_eq!(frame(&mut vm, true, STATUS, false), (0, false, false));
    }

    #[test]
    fn status_keeps_a_real_uart_framing_error_after_ack() {
        let mut vm = lower("uart-boot-mmio").unwrap();
        vm.reset();
        idle(&mut vm, BIT_TICKS, STATUS);
        send(&mut vm, 0x7e, false);
        assert_eq!(idle(&mut vm, DRAIN, STATUS), (2, false, true));
        frame(&mut vm, true, ACK, true);
        assert_eq!(idle(&mut vm, DRAIN, STATUS), (2, false, true));
    }
}
