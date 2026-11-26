//! 実 UART CPU の書き込み配線を調べ、時刻付きのアドレス照合を SMT-LIB に出力する。

use circuitgame2_core::netlist::{ClockedVm, NorNetlist, Wire};
use std::{
    collections::HashSet,
    error::Error,
    io::{self, Write},
    ops::Range,
    path::Path,
};

#[allow(dead_code)]
#[path = "../support/uart.rs"]
mod uart;

type Check<T> = Result<T, Box<dyn Error>>;

/// ラッチの出力と CPU のデータ応答で切った NOR の依存経路。
struct Cone {
    /// 依存先を先に並べたゲート列。順序回路の平坦な添字はこの順とは限らない。
    gates: Vec<usize>,
    /// 各根拠入力から観測する出力までの最大 NOR 段数。
    inputs: Vec<Option<usize>>,
    read_depth: Option<usize>,
}

fn depths(net: &NorNetlist, roots: &[Wire], read_data: &[Wire]) -> Check<Cone> {
    fn visit(
        net: &NorNetlist,
        wire: Wire,
        cut: &[bool],
        marks: &mut [u8],
        order: &mut Vec<usize>,
    ) -> Check<()> {
        let Wire::Gate(index) = wire else {
            return Ok(());
        };
        if cut[index] || marks[index] == 2 {
            return Ok(());
        }
        if marks[index] == 1 {
            return Err("ラッチで区切った経路に NOR の循環がある。".into());
        }
        marks[index] = 1;
        let gate = net.gates[index];
        visit(net, gate.left, cut, marks, order)?;
        visit(net, gate.right, cut, marks, order)?;
        marks[index] = 2;
        order.push(index);
        Ok(())
    }

    let mut cut = vec![false; net.gates.len()];
    for &wire in read_data {
        let Wire::Gate(index) = wire else {
            return Err("データ応答が NOR の出力でない。".into());
        };
        cut[index] = true;
    }
    let mut gates = Vec::new();
    let mut marks = vec![0; net.gates.len()];
    for &wire in roots {
        visit(net, wire, &cut, &mut marks, &mut gates)?;
    }
    let mut ranks = vec![None; net.gates.len()];
    let mut inputs = vec![None; net.input_count];
    let mut read_depth = None;
    let mut propagate = |wire, rank, ranks: &mut [Option<usize>]| {
        let depth = match wire {
            Wire::Constant(_) => return,
            Wire::Input(index) => &mut inputs[index],
            Wire::Gate(index) if cut[index] => &mut read_depth,
            Wire::Gate(index) => &mut ranks[index],
        };
        *depth = (*depth).max(Some(rank));
    };
    for &wire in roots {
        propagate(wire, 0, &mut ranks);
    }
    for &index in gates.iter().rev() {
        let rank = ranks[index].unwrap() + 1;
        propagate(net.gates[index].left, rank, &mut ranks);
        propagate(net.gates[index].right, rank, &mut ranks);
    }
    Ok(Cone {
        gates,
        inputs,
        read_depth,
    })
}

fn latch_range(vm: &ClockedVm, wire: Wire, width: usize) -> Check<Range<usize>> {
    let Wire::Input(index) = wire else {
        return Err("観測した状態が入力配線でない。".into());
    };
    (0..vm.storage_count())
        .filter_map(|slot| vm.storage_wiring(slot))
        .find_map(|(storage, range)| {
            (storage.address.is_none() && range.len() == width && range.contains(&index))
                .then_some(range)
        })
        .ok_or_else(|| format!("期待する {width} ビットのラッチが見つからない。").into())
}

fn address_cut(
    vm: &ClockedVm,
    cone: &Cone,
) -> Check<(Range<usize>, Range<usize>, Range<usize>, usize)> {
    let pc = latch_range(vm, vm.netlist().outputs[0], 64)?;
    let pc_wires = pc.clone().map(Wire::Input);
    if !pc_wires.eq(vm.netlist().outputs[..64].iter().copied()) {
        return Err("公開 PC とラッチのビット配置が異なる。".into());
    }
    let mut instruction = None;
    let mut operand = None;
    let mut fetch = None;
    for slot in 0..vm.storage_count() {
        let (storage, range) = vm.storage_wiring(slot).ok_or("状態素子が見つからない。")?;
        if range == pc || !cone.inputs[range.clone()].iter().any(Option::is_some) {
            continue;
        }
        let selected = match (storage.address.is_none(), range.len()) {
            (true, 1) => &mut fetch,
            (true, 33) => &mut instruction,
            (true, 64) => &mut operand,
            _ => return Err("バスアドレスが想定外の状態素子を読む。".into()),
        };
        if selected.replace(range).is_some() {
            return Err("バスアドレスのラッチを一意に区別できない。".into());
        }
    }
    let fetch = fetch.ok_or("フェッチ選択ラッチがない。")?.start;
    Ok((
        pc,
        instruction.ok_or("命令ラッチがない。")?,
        operand.ok_or("オペランドラッチがない。")?,
        fetch,
    ))
}

fn require_bound(cone: &Cone, range: Range<usize>, bound: usize, label: &str) -> Check<()> {
    let measured = cone.inputs[range].iter().copied().flatten().max();
    if !measured.is_some_and(|depth| depth <= bound) {
        return Err(
            format!("{label} の最大経路が想定外。計測 {measured:?}、上限 {bound}。").into(),
        );
    }
    eprintln!("{label}: {} 段", measured.unwrap());
    Ok(())
}

fn writer_input(vm: &ClockedVm, wire: Wire) -> Check<Wire> {
    let net = vm.netlist();
    let range = latch_range(vm, wire, 1)?;
    let writer = (0..vm.storage_count())
        .filter_map(|slot| vm.storage_wiring(slot))
        .find(|(_, output)| *output == range)
        .ok_or("bus-write のラッチがない。")?
        .0;
    let Wire::Gate(and) = writer.data[0] else {
        return Err("書き込み要求の AND がない。".into());
    };
    let negated = |wire| {
        let Wire::Gate(index) = wire else {
            return None;
        };
        let gate = net.gates[index];
        (gate.left == gate.right).then_some(gate.left)
    };
    let gate = net.gates[and];
    let (Some(retire), Some(qualified)) = (negated(gate.left), negated(gate.right)) else {
        return Err("書き込み要求が退役と許可の論理積でない。".into());
    };
    latch_range(vm, retire, 1)?;
    // 書き込みパルスを記録するラッチの許可は、reset から4 tick 経過後に1を保持する。
    let mut enable = writer.enable;
    for _ in 0..4 {
        let range = latch_range(vm, enable, 1)?;
        let delay = (0..vm.storage_count())
            .filter_map(|slot| vm.storage_wiring(slot))
            .find(|(_, output)| *output == range)
            .ok_or("書き込み許可の初期化ラッチがない。")?
            .0;
        if delay.enable != Wire::Constant(true) {
            return Err("書き込み許可の初期化は定数を遅延させる。".into());
        }
        enable = delay.data[0];
    }
    if enable != Wire::Constant(true) {
        return Err("書き込み許可の初期化が4 tick の遅延と異なる。".into());
    }
    Ok(qualified)
}

fn verify_paths(
    vm: &ClockedVm,
    address: &Cone,
    pc: Range<usize>,
    instruction: Range<usize>,
    operand: Range<usize>,
    fetch: usize,
) -> Check<Wire> {
    let net = vm.netlist();
    let read = &net.outputs[128..192];
    if address.inputs[0].is_some() || address.read_depth.is_some() {
        return Err("RX またはデータ応答が、ラッチを介さずアドレスに届く。".into());
    }
    require_bound(address, 1..65, 16, "probe から bus-address")?;
    require_bound(address, pc.clone(), 4, "PC から bus-address")?;
    require_bound(address, fetch..fetch + 1, 4, "フェッチ選択から bus-address")?;
    require_bound(
        address,
        instruction.start..instruction.start + 32,
        479,
        "命令から bus-address",
    )?;
    if address.inputs[instruction.end - 1].is_some() {
        return Err("フェッチ有効ビットがバスアドレスの組み合わせ経路にある。".into());
    }
    require_bound(address, operand.clone(), 465, "rs1 から bus-address")?;

    let permission = writer_input(vm, net.outputs[196])?;
    let write = depths(net, &[permission], read)?;
    if write.read_depth.is_some() || write.inputs[..65].iter().any(Option::is_some) {
        return Err("書き込み許可が、ラッチを介さず外部入力またはデータ応答に依存する。".into());
    }
    require_bound(&write, pc.clone(), 34, "PC から dispatch の書き込み許可")?;
    require_bound(
        &write,
        instruction.start..instruction.start + 32,
        502,
        "命令から dispatch の書き込み許可",
    )?;
    require_bound(
        &write,
        instruction.end - 1..instruction.end,
        4,
        "フェッチ有効から dispatch の書き込み許可",
    )?;
    require_bound(
        &write,
        operand.clone(),
        490,
        "rs1 から dispatch の書き込み許可",
    )?;
    let mut second_operand = None;
    for (index, depth) in write.inputs.iter().enumerate().skip(65) {
        if depth.is_none()
            || pc.contains(&index)
            || instruction.contains(&index)
            || operand.contains(&index)
        {
            continue;
        }
        let range = latch_range(vm, Wire::Input(index), 64)?;
        if second_operand.as_ref().is_some_and(|found| found != &range) {
            return Err("書き込み許可が複数の追加ラッチに依存する。".into());
        }
        second_operand = Some(range);
    }
    require_bound(
        &write,
        second_operand.ok_or("書き込み許可の rs2 ラッチがない。")?,
        70,
        "rs2 から dispatch の書き込み許可",
    )?;

    // RAM に入る128ビットの要求は、公開アドレスと書き込みデータを並べたもの。
    let mut requests = (0..vm.storage_count())
        .filter_map(|slot| vm.storage_wiring(slot))
        .filter(|(storage, range)| {
            storage.address.is_none()
                && range.len() == 128
                && storage.data[..64] == net.outputs[64..128]
        });
    let (request, _) = requests.next().ok_or("RAM の要求ラッチがない。")?;
    if requests.next().is_some() {
        return Err("RAM の要求ラッチを一意に区別できない。".into());
    }
    let data = depths(net, &request.data[64..], read)?;
    if data.read_depth != Some(8) || data.inputs[..65].iter().any(Option::is_some) {
        return Err("データ応答から部分書き込み値への経路が想定外。".into());
    }
    eprintln!("データ応答から部分書き込み値: 8 段");
    Ok(permission)
}

struct TimedAddress<'a, W> {
    net: &'a NorNetlist,
    out: W,
    varies_with_probe: Vec<bool>,
    emitted: HashSet<(usize, Option<usize>)>,
    probes: HashSet<(usize, usize)>,
    states: HashSet<usize>,
}

impl<W: Write> TimedAddress<'_, W> {
    /// 過去の tick を age で表す。外部線はその tick の入力、ゲートは前 tick の状態を読む。
    fn emit(&mut self, wire: Wire, age: usize) -> io::Result<String> {
        Ok(match wire {
            Wire::Constant(bit) => bit.to_string(),
            Wire::Input(index @ 1..=64) => {
                let name = format!("p{index}_{age}");
                if self.probes.insert((index, age)) {
                    writeln!(self.out, "(declare-const {name} Bool)")?;
                }
                name
            }
            Wire::Input(index) => {
                debug_assert!(index >= 65);
                let name = format!("q{index}");
                if self.states.insert(index) {
                    writeln!(self.out, "(declare-const {name} Bool)")?;
                }
                name
            }
            Wire::Gate(index) => {
                let age = self.varies_with_probe[index].then_some(age);
                let name = match age {
                    Some(tick) => format!("g{index}_{tick}"),
                    None => format!("g{index}"),
                };
                if self.emitted.insert((index, age)) {
                    let gate = self.net.gates[index];
                    let child_age =
                        |wire| age.unwrap_or(0) + usize::from(!matches!(wire, Wire::Input(1..=64)));
                    let left = self.emit(gate.left, child_age(gate.left))?;
                    let right = self.emit(gate.right, child_age(gate.right))?;
                    writeln!(
                        self.out,
                        "(define-fun {name} () Bool (not (or {left} {right})))"
                    )?;
                }
                name
            }
        })
    }

    fn vector(
        &mut self,
        symbol: &str,
        bits: impl DoubleEndedIterator<Item = Wire>,
    ) -> io::Result<()> {
        let mut width = 0;
        let mut expression = String::new();
        for bit in bits.rev() {
            let name = self.emit(bit, 0)?;
            let value = format!("(ite {name} #b1 #b0)");
            expression = if width == 0 {
                value
            } else {
                format!("(concat {expression} {value})")
            };
            width += 1;
        }
        writeln!(
            self.out,
            "(define-fun {symbol} () (_ BitVec {width}) {expression})"
        )
    }
}

fn main() -> Check<()> {
    let vm = uart::machine(Path::new("uart-boot.toml"))?;
    let net = vm.netlist();
    let address = &net.outputs[64..128];
    let cone = depths(net, address, &net.outputs[128..192])?;
    let (pc, instruction, operand, fetch) = address_cut(&vm, &cone)?;
    let permission = verify_paths(&vm, &cone, pc, instruction.clone(), operand.clone(), fetch)?;

    let mut varies_with_probe = vec![false; net.gates.len()];
    let variable = |wire, known: &[bool]| match wire {
        Wire::Input(1..=64) => true,
        Wire::Gate(index) => known[index],
        _ => false,
    };
    for &index in &cone.gates {
        let gate = net.gates[index];
        varies_with_probe[index] =
            variable(gate.left, &varies_with_probe) || variable(gate.right, &varies_with_probe);
    }
    let mut model = TimedAddress {
        net,
        out: io::BufWriter::new(io::stdout().lock()),
        varies_with_probe,
        emitted: HashSet::new(),
        probes: HashSet::new(),
        states: HashSet::new(),
    };
    writeln!(model.out, "(set-logic QF_BV)")?;
    model.vector("bus", address.iter().copied())?;
    model.vector(
        "instruction",
        (instruction.start..instruction.start + 32).map(Wire::Input),
    )?;
    model.vector("rs1", operand.map(Wire::Input))?;
    let fetching = model.emit(Wire::Input(fetch), 0)?;
    let allowed = model.emit(permission, 0)?;
    writeln!(model.out, "(define-fun allowed () Bool {allowed})")?;
    writeln!(model.out, "(assert (not {fetching}))")?;
    write!(model.out, "{}", include_str!("uart_store_address.smt2"))?;
    model.out.flush()?;
    Ok(())
}
