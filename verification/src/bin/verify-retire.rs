//! 実際の NOR とラッチを tick ごとに展開し、完了通知から更新要求までを検査する。

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

const HISTORY: usize = 6;

#[derive(Clone, Copy)]
enum Controller {
    Accept,
    Retire,
}

impl Controller {
    fn name(self) -> &'static str {
        match self {
            Self::Accept => "accept-pulse",
            Self::Retire => "firmware-retire",
        }
    }

    fn widths(self) -> (usize, usize) {
        match self {
            Self::Accept => (2, 1),
            Self::Retire => (5, 3),
        }
    }

    // CPU の dispatch は、無効な命令で書き込みを許可しない。
    fn constrain(self, out: &mut impl Write, tick: usize) -> io::Result<()> {
        if let Self::Retire = self {
            writeln!(out, "(assert (=> (or i_{tick}_3 i_{tick}_4) i_{tick}_2))")?;
        }
        Ok(())
    }

    // 回路の内部状態や NOR の段数から独立した、ポート上の期待値を定める。
    fn expected(self, tick: usize, output: usize) -> String {
        match self {
            Self::Accept if tick >= 5 => {
                let request = tick - 2;
                format!("(and i_{request}_0 i_{request}_1)")
            }
            Self::Retire if tick >= 6 => {
                let event = tick - 5;
                let qualifier = tick - 2;
                let completed = format!("(or i_{event}_0 i_{event}_1)");
                if output == 0 {
                    format!("(and {completed} i_{qualifier}_2)")
                } else {
                    let permission = output + 2;
                    format!("(and {completed} i_{qualifier}_2 i_{qualifier}_{permission})")
                }
            }
            _ => "false".into(),
        }
    }
}

fn lower(controller: Controller) -> Result<ClockedVm, Box<dyn Error>> {
    let sources = [
        ("decode.ncg2", include_str!("../../../circuit/decode.ncg2")),
        (
            "arithmetic.ncg2",
            include_str!("../../../circuit/arithmetic.ncg2"),
        ),
        (
            "execute-boot.ncg2",
            include_str!("../../../circuit/execute-boot.ncg2"),
        ),
        (
            "boot-machine.ncg2",
            include_str!("../../../circuit/boot-machine.ncg2"),
        ),
        (
            "firmware-engine.ncg2",
            include_str!("../../../circuit/firmware-engine.ncg2"),
        ),
    ];
    let snapshots = sources.map(|(path, text)| SourceSnapshot {
        path,
        text,
        revision: 0,
    });
    let project = parse_circuit_project(&snapshots).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let file = match controller {
        Controller::Accept => 3,
        Controller::Retire => 4,
    };
    let circuit = &project.circuits()[file];
    let declaration = circuit
        .names
        .iter()
        .find(|name| name.name.word == controller.name())
        .ok_or("検証対象の回路が見つからない。")?;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == declaration.form)
        .ok_or("検証対象の回路の索引が見つからない。")?;
    let limits = ClockedModuleVmLimits {
        width: 64,
        wiring_bits: 128,
        gates: 128,
        state_bits: 128,
    };
    let compiled = aliases
        .lower_clocked_vm(ModuleBodyPosition { file, entry }, limits)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("検証対象を VM に展開できない。")?;
    Ok(compiled.vm)
}

fn emit_differences(
    network: &LatchNetwork<'_>,
    out: &mut impl Write,
    controller: Controller,
    prefix: &str,
    tick: usize,
) -> io::Result<()> {
    for (index, &wire) in network.nor.outputs.iter().enumerate() {
        let observed = network.wire(wire, prefix, tick, tick);
        let expected = controller.expected(tick, index);
        writeln!(out, "    (distinct {observed} {expected})")?;
    }
    Ok(())
}

fn emit(
    out: &mut impl Write,
    controller: Controller,
    vm: &ClockedVm,
) -> Result<(), Box<dyn Error>> {
    let network = LatchNetwork::read(vm)?;
    if (network.external, network.nor.outputs.len()) != controller.widths() {
        return Err("ポート幅が完了制御の参照仕様と一致しない。".into());
    }
    writeln!(out, "(set-logic QF_BV)")?;
    writeln!(out, "(set-option :timeout 30000)")?;
    for tick in 0..=HISTORY {
        for index in 0..network.external {
            if tick == 0 {
                writeln!(out, "(define-fun i_{tick}_{index} () Bool false)")?;
            } else {
                writeln!(out, "(declare-const i_{tick}_{index} Bool)")?;
            }
        }
        controller.constrain(out, tick)?;
    }
    network.emit_state(out, "reset", true)?;
    network.emit_state(out, "arbitrary", false)?;
    for tick in 1..=HISTORY {
        network.emit_step(out, "reset", tick)?;
        network.emit_step(out, "arbitrary", tick)?;
    }
    writeln!(out, "(assert (or")?;
    for tick in 0..HISTORY {
        emit_differences(&network, out, controller, "reset", tick)?;
    }
    // 開始状態を任意にすると、6 tick 経過後は任意の時点へ同じ検査を適用できる。
    emit_differences(&network, out, controller, "arbitrary", HISTORY)?;
    writeln!(out, "))")?;
    writeln!(out, "(check-sat)")?;
    Ok(())
}

/// reset からの反例に含まれる入力を、同じ VM の実行に渡す。
fn replay(mut vm: ClockedVm, frames: &[String]) -> Result<(), Box<dyn Error>> {
    let print = |tick, output: Vec<bool>| {
        let bits: String = output
            .into_iter()
            .map(|bit| if bit { '1' } else { '0' })
            .collect();
        println!("{tick} {bits}");
    };
    print(0, vm.reset());
    for (i, frame) in frames.iter().enumerate() {
        let input = frame
            .bytes()
            .map(|bit| match bit {
                b'0' => Ok(false),
                b'1' => Ok(true),
                _ => Err("入力フレームには 0 と 1 を指定する。"),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let output = vm.step(&input).map_err(|error| format!("{error:?}"))?;
        print(i + 1, output);
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let controller = match args.first().map(String::as_str) {
        Some("accept-pulse") => Controller::Accept,
        Some("firmware-retire") => Controller::Retire,
        _ => return Err("accept-pulse または firmware-retire を指定する。".into()),
    };
    let vm = lower(controller)?;
    match args.as_slice() {
        [_] => emit(&mut io::stdout().lock(), controller, &vm),
        [_, replay_flag, frames @ ..] if replay_flag == "--replay" => replay(vm, frames),
        _ => Err("実行を再生するときは --replay に入力フレームを続ける。".into()),
    }
}
