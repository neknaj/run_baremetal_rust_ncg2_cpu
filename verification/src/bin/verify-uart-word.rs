//! 展開した UART 受信語の回路を、バイト列と ACK から定める状態遷移と照合する。

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

const LATENCY: usize = 12;
const BYTE_SPACING: usize = 16;
const SLOTS: usize = 24;
const END: usize = SLOTS * BYTE_SPACING + LATENCY + 1;

fn lower() -> Result<ClockedVm, Box<dyn Error>> {
    let sources = [
        SourceSnapshot {
            path: "basis.ncg2",
            text: "(using nor)",
            revision: 0,
        },
        SourceSnapshot {
            path: "uart-word-buffer.ncg2",
            text: include_str!("../../../circuit/uart-word-buffer.ncg2"),
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
        .find(|name| name.name.word == "uart-word-buffer")
        .ok_or("UART 受信語の回路が見つからない。")?;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == module.form)
        .ok_or("UART 受信語の回路の索引が見つからない。")?;
    let limits = ClockedModuleVmLimits {
        width: 64,
        wiring_bits: 1024,
        gates: 128,
        state_bits: 256,
    };
    let compiled = aliases
        .lower_clocked_vm(ModuleBodyPosition { file: 1, entry }, limits)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("UART 受信語の回路を VM に展開できない。")?;
    Ok(compiled.vm)
}

fn declare_inputs(out: &mut impl Write, tick: usize) -> io::Result<()> {
    let candidate = tick > 0 && tick <= SLOTS * BYTE_SPACING && tick % BYTE_SPACING == 0;
    for bit in 0..11 {
        if bit == 8 {
            writeln!(out, "(define-fun i_{tick}_{bit} () Bool {candidate})")?;
        } else if candidate || (tick > 0 && bit >= 9) {
            writeln!(out, "(declare-const i_{tick}_{bit} Bool)")?;
        } else {
            writeln!(out, "(define-fun i_{tick}_{bit} () Bool false)")?;
        }
    }
    Ok(())
}

// 仕様は8バイトを1組とし、完成した語の登録、旧語の消費、失敗の保持を決める。
fn emit_reference(out: &mut impl Write, tick: usize) -> io::Result<()> {
    let prev = tick - 1;
    let bit = |index: usize| format!("(ite i_{tick}_{index} #b1 #b0)");
    let incoming = (0..8)
        .rev()
        .map(bit)
        .reduce(|higher, lower| format!("(concat {higher} {lower})"))
        .expect("1バイトは8ビット");
    let valid = format!("i_{tick}_8");
    let framing = format!("i_{tick}_9");
    let public = prev.saturating_sub(LATENCY);
    let ack = format!("(and i_{tick}_10 pending_{public})");
    writeln!(
        out,
        "(define-fun full_{tick} () Bool (and {valid} (= count_{prev} #b111)))"
    )?;
    for (name, kind, value) in [
        (
            "count",
            "(_ BitVec 3)",
            format!("(ite {valid} (bvadd count_{prev} #b001) count_{prev})"),
        ),
        (
            "bytes",
            "(_ BitVec 64)",
            format!(
                "(ite {valid} (concat {incoming} ((_ extract 63 8) bytes_{prev})) bytes_{prev})"
            ),
        ),
        (
            "pending",
            "Bool",
            format!("(or full_{tick} (and pending_{prev} (not {ack})))"),
        ),
        (
            "value",
            "(_ BitVec 64)",
            format!(
                "(ite (and full_{tick} (or (not pending_{prev}) {ack})) bytes_{tick} value_{prev})"
            ),
        ),
        (
            "failed",
            "Bool",
            format!("(or failed_{prev} {framing} (and full_{tick} pending_{prev} (not {ack})))"),
        ),
    ] {
        writeln!(out, "(declare-const {name}_{tick} {kind})")?;
        writeln!(out, "(assert (= {name}_{tick} {value}))")?;
    }
    Ok(())
}

fn emit(out: &mut impl Write, vm: &ClockedVm) -> Result<(), Box<dyn Error>> {
    let network = LatchNetwork::read(vm)?;
    if (network.external, network.nor.outputs.len()) != (11, 66) {
        return Err("UART 受信語の入出力幅が参照仕様と異なる。".into());
    }
    writeln!(out, "(set-logic QF_BV)")?;
    writeln!(out, "(set-option :timeout 90000)")?;
    network.emit_state(out, "word", true)?;
    writeln!(out, "(define-fun count_0 () (_ BitVec 3) #b000)")?;
    writeln!(out, "(define-fun bytes_0 () (_ BitVec 64) (_ bv0 64))")?;
    writeln!(out, "(define-fun value_0 () (_ BitVec 64) (_ bv0 64))")?;
    writeln!(out, "(define-fun pending_0 () Bool false)")?;
    writeln!(out, "(define-fun failed_0 () Bool false)")?;
    declare_inputs(out, 0)?;
    for tick in 1..=END {
        declare_inputs(out, tick)?;
        network.emit_step(out, "word", tick)?;
        emit_reference(out, tick)?;
    }
    writeln!(out, "(assert (or")?;
    for tick in 0..=END {
        let reference = tick.saturating_sub(LATENCY);
        for (index, &wire) in network.nor.outputs.iter().enumerate() {
            let actual = network.wire(wire, "word", tick, tick);
            let expected = match index {
                64 => format!("pending_{reference}"),
                65 => format!("failed_{reference}"),
                _ => format!("(= ((_ extract {index} {index}) value_{reference}) #b1)"),
            };
            writeln!(out, "  (distinct {actual} {expected})")?;
        }
    }
    writeln!(out, "))")?;
    writeln!(
        out,
        "(check-sat-using (then simplify propagate-values solve-eqs bit-blast sat))"
    )?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    emit(&mut io::BufWriter::new(io::stdout().lock()), &lower()?)
}

#[cfg(test)]
mod tests {
    use super::{BYTE_SPACING, LATENCY, lower};

    const FIRST: [u8; 8] = [0x01, 0x12, 0x23, 0x34, 0x45, 0x56, 0x67, 0x78];
    const SECOND: [u8; 8] = [0x82, 0x93, 0xa4, 0xb5, 0xc6, 0xd7, 0xe8, 0xf9];
    const FIRST_LAST: usize = FIRST.len() * BYTE_SPACING;
    const SECOND_LAST: usize = FIRST_LAST + SECOND.len() * BYTE_SPACING;

    fn check(ack: &[usize], framing: &[usize], expected: &[(usize, (u64, bool, bool))]) {
        let mut vm = lower().unwrap();
        assert_eq!(vm.reset(), vec![false; 66]);
        let mut output = (0, false, false);
        let bytes = FIRST.into_iter().chain(SECOND).collect::<Vec<_>>();
        for tick in 1..=SECOND_LAST + LATENCY + BYTE_SPACING {
            let source = if tick % BYTE_SPACING == 0 {
                bytes.get(tick / BYTE_SPACING - 1).copied()
            } else {
                None
            };
            let input: Vec<_> = (0..8)
                .map(|bit| source.unwrap_or(0) & (1 << bit) != 0)
                .chain([
                    source.is_some(),
                    framing.contains(&tick),
                    ack.contains(&tick),
                ])
                .collect();
            if let Some((_, next)) = expected.iter().find(|(when, _)| *when == tick) {
                output = *next;
            }
            let frame = vm.step(&input).unwrap();
            let word = frame[..64]
                .iter()
                .enumerate()
                .fold(0, |word, (bit, &value)| word | u64::from(value) << bit);
            assert_eq!((word, frame[64], frame[65]), output, "tick {tick}");
        }
    }

    #[test]
    fn simultaneous_ack_replaces_the_word_and_empty_ack_has_no_effect() {
        let old = u64::from_le_bytes(FIRST);
        let new = u64::from_le_bytes(SECOND);
        check(
            &[
                1,
                FIRST_LAST,
                FIRST_LAST + 1,
                SECOND_LAST,
                SECOND_LAST + LATENCY + 1,
            ],
            &[],
            &[
                (FIRST_LAST + LATENCY, (old, true, false)),
                (SECOND_LAST + LATENCY, (new, true, false)),
                (SECOND_LAST + 2 * LATENCY + 1, (new, false, false)),
            ],
        );
    }

    #[test]
    fn overrun_keeps_the_old_word_and_remains_after_ack() {
        let old = u64::from_le_bytes(FIRST);
        check(
            &[SECOND_LAST + 1],
            &[],
            &[
                (FIRST_LAST + LATENCY, (old, true, false)),
                (SECOND_LAST + LATENCY, (old, true, true)),
                (SECOND_LAST + LATENCY + 1, (old, false, true)),
            ],
        );
    }

    #[test]
    fn framing_error_before_the_first_byte_is_sticky() {
        let old = u64::from_le_bytes(FIRST);
        let new = u64::from_le_bytes(SECOND);
        check(
            &[SECOND_LAST - 1],
            &[1],
            &[
                (LATENCY + 1, (0, false, true)),
                (FIRST_LAST + LATENCY, (old, true, true)),
                (SECOND_LAST + LATENCY - 1, (old, false, true)),
                (SECOND_LAST + LATENCY, (new, true, true)),
            ],
        );
    }
}
