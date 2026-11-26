//! 実際の NOR とラッチを tick ごとに展開し、完了通知から更新要求までを検査する。

use circuitgame2_core::{
    netlist::{ClockedStorageInputs, ClockedVm, NorNetlist, Wire},
    project::{
        ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, check_use_aliases,
        index_use_targets, parse_circuit_project,
    },
};
use std::{
    error::Error,
    io::{self, Write},
    ops::Range,
};

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
            "firmware-machine.ncg2",
            include_str!("../../../circuit/firmware-machine.ncg2"),
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

/// 増補入力の先頭に外部入力を、続いてラッチの登録出力を置く借用配線。
struct LatchNetwork<'a> {
    nor: &'a NorNetlist,
    external: usize,
    latches: Vec<(&'a ClockedStorageInputs, Range<usize>)>,
}

impl<'a> LatchNetwork<'a> {
    fn read(vm: &'a ClockedVm, controller: Controller) -> Result<Self, &'static str> {
        let nor = vm.netlist();
        let external = vm.external_input_count();
        if (external, nor.outputs.len()) != controller.widths() {
            return Err("ポート幅が完了制御の参照仕様と一致しない。");
        }
        let mut latches = Vec::with_capacity(vm.storage_count());
        let mut start = external;
        for index in 0..vm.storage_count() {
            let (wiring, range) = vm
                .storage_wiring(index)
                .ok_or("状態素子の配線が見つからない。")?;
            if wiring.address.is_some() {
                return Err("この検証器はラッチ専用で、RAM を展開できない。");
            }
            if range.start != start || range.len() != wiring.data.len() {
                return Err("ラッチ出力の順序または幅が増補入力と一致しない。");
            }
            start = range.end;
            latches.push((wiring, range));
        }
        if start != nor.input_count {
            return Err("外部入力とラッチ出力が増補入力の全域を覆わない。");
        }
        Ok(Self {
            nor,
            external,
            latches,
        })
    }

    // 更新式では開始時の状態、公開出力では確定後の状態を指定する。
    fn wire(&self, wire: Wire, prefix: &str, frame: usize, state: usize) -> String {
        match wire {
            Wire::Input(index) if index < self.external => format!("i_{frame}_{index}"),
            Wire::Input(index) => format!("{prefix}_q_{state}_{index}"),
            Wire::Gate(index) => format!("{prefix}_g_{state}_{index}"),
            Wire::Constant(value) => value.to_string(),
        }
    }

    fn emit_state(&self, out: &mut impl Write, prefix: &str, from_reset: bool) -> io::Result<()> {
        for (kind, indices) in [
            ("g", 0..self.nor.gates.len()),
            ("q", self.external..self.nor.input_count),
        ] {
            for index in indices {
                let symbol = format!("{prefix}_{kind}_0_{index}");
                if from_reset {
                    writeln!(out, "(define-fun {symbol} () Bool false)")?;
                } else {
                    writeln!(out, "(declare-const {symbol} Bool)")?;
                }
            }
        }
        Ok(())
    }

    fn emit_step(&self, out: &mut impl Write, prefix: &str, frame: usize) -> io::Result<()> {
        let prev = frame - 1;
        let wire = |source| self.wire(source, prefix, frame, prev);
        for (index, gate) in self.nor.gates.iter().enumerate() {
            writeln!(
                out,
                "(define-fun {prefix}_g_{frame}_{index} () Bool (not (or {} {})))",
                wire(gate.left),
                wire(gate.right)
            )?;
        }
        for (wiring, range) in &self.latches {
            let enable = wire(wiring.enable);
            for (index, &data) in range.clone().zip(&wiring.data) {
                writeln!(
                    out,
                    "(define-fun {prefix}_q_{frame}_{index} () Bool (ite {enable} {} {prefix}_q_{prev}_{index}))",
                    wire(data)
                )?;
            }
        }
        Ok(())
    }

    fn emit_differences(
        &self,
        out: &mut impl Write,
        controller: Controller,
        prefix: &str,
        tick: usize,
    ) -> io::Result<()> {
        for (index, &wire) in self.nor.outputs.iter().enumerate() {
            let observed = self.wire(wire, prefix, tick, tick);
            let expected = controller.expected(tick, index);
            writeln!(out, "    (distinct {observed} {expected})")?;
        }
        Ok(())
    }
}

fn emit(
    out: &mut impl Write,
    controller: Controller,
    vm: &ClockedVm,
) -> Result<(), Box<dyn Error>> {
    let network = LatchNetwork::read(vm, controller)?;
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
        network.emit_differences(out, controller, "reset", tick)?;
    }
    // 開始状態を任意にすると、6 tick 経過後は任意の時点へ同じ検査を適用できる。
    network.emit_differences(out, controller, "arbitrary", HISTORY)?;
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
