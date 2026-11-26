//! 実 UART CPU の書き込み配線を調べ、時刻付きのアドレス照合を SMT-LIB に出力する。

use circuitgame2_core::netlist::{ClockedStorageInputs, ClockedVm, NorNetlist, Wire};
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

fn latch(vm: &ClockedVm, wire: Wire, width: usize) -> Check<(&ClockedStorageInputs, Range<usize>)> {
    let Wire::Input(index) = wire else {
        return Err("観測した状態が入力配線でない。".into());
    };
    (0..vm.storage_count())
        .filter_map(|slot| vm.storage_wiring(slot))
        .find(|(storage, range)| {
            storage.address.is_none() && range.len() == width && range.contains(&index)
        })
        .ok_or_else(|| format!("期待する {width} ビットのラッチが見つからない。").into())
}

fn latch_range(vm: &ClockedVm, wire: Wire, width: usize) -> Check<Range<usize>> {
    Ok(latch(vm, wire, width)?.1)
}

/// リセット後の常時許可ラッチを順にたどり、指定 tick 数だけ前の信号を得る。
fn delay_source(vm: &ClockedVm, mut wire: Wire, ticks: usize) -> Check<Wire> {
    for _ in 0..ticks {
        let (delay, _) = latch(vm, wire, 1)?;
        if delay.enable != Wire::Constant(true) {
            return Err("時刻を決める遅延ラッチが常時許可でない。".into());
        }
        wire = delay.data[0];
    }
    Ok(wire)
}

fn nor_inputs(net: &NorNetlist, wire: Wire) -> Option<[Wire; 2]> {
    match wire {
        Wire::Gate(index) => net.gates.get(index).map(|gate| [gate.left, gate.right]),
        _ => None,
    }
}

fn inverted_input(net: &NorNetlist, wire: Wire) -> Option<Wire> {
    let [left, right] = nor_inputs(net, wire)?;
    (left == right).then_some(left)
}

fn and_inputs(net: &NorNetlist, wire: Wire) -> Option<[Wire; 2]> {
    let [left, right] = nor_inputs(net, wire)?;
    Some([inverted_input(net, left)?, inverted_input(net, right)?])
}

fn or_inputs(net: &NorNetlist, wire: Wire) -> Option<[Wire; 2]> {
    nor_inputs(net, inverted_input(net, wire)?)
}

fn other_input(inputs: [Wire; 2], known: Wire) -> Option<Wire> {
    match inputs {
        [left, right] if left == known && right != known => Some(right),
        [left, right] if right == known && left != known => Some(left),
        _ => None,
    }
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

fn writer_inputs(vm: &ClockedVm, wire: Wire) -> Check<(Wire, Wire)> {
    let net = vm.netlist();
    let (writer, _) = latch(vm, wire, 1)?;
    let [retire, qualified] =
        and_inputs(net, writer.data[0]).ok_or("書き込み要求が退役と許可の論理積でない。")?;
    // 書き込みパルスを記録するラッチの許可は、reset から4 tick 経過後に1を保持する。
    if delay_source(vm, writer.enable, 4)? != Wire::Constant(true) {
        return Err("書き込み許可の初期化が4 tick の遅延と異なる。".into());
    }
    Ok((retire, qualified))
}

/// 完了を決める64ビットのマークに、積やオペランドの値が入らないことを調べる。
fn verify_multiplier_round_marker(vm: &ClockedVm, done: Wire, running: Wire) -> Check<()> {
    let net = vm.netlist();
    let (done_latch, _) = latch(vm, done, 1)?;
    let [initialize, unfinished] =
        nor_inputs(net, done_latch.data[0]).ok_or("乗算の次の完了値が NOR でない。")?;
    let last = nor_inputs(net, unfinished)
        .and_then(|inputs| other_input(inputs, done))
        .ok_or("乗算完了が完了フラグと最終マークを参照していない。")?;
    let (mark, bits) = latch(vm, last, 64)?;
    if inverted_input(net, initialize) != Some(running)
        || last != Wire::Input(bits.end - 1)
        || mark.enable != done_latch.enable
    {
        return Err("ラウンドマークと完了フラグの保持または初期化の配線が異なる。".into());
    }
    for (bit, &next) in mark.data.iter().enumerate() {
        let shifted = if bit == 0 {
            Wire::Constant(false)
        } else {
            Wire::Input(bits.start + bit - 1)
        };
        let seeded = Wire::Constant(bit == 0);
        let [set, advance] = or_inputs(net, next).ok_or("次のラウンドマークが選択回路でない。")?;
        if and_inputs(net, set) != Some([initialize, seeded])
            || and_inputs(net, advance) != Some([running, shifted])
        {
            return Err(
                format!("ラウンドマークのビット{bit}の初期化またはシフトが異なる。").into(),
            );
        }
    }
    let roots: Vec<_> = mark
        .data
        .iter()
        .copied()
        .chain(done_latch.data.iter().copied())
        .collect();
    let cone = depths(net, &roots, &[])?;
    let Wire::Input(running_index) = running else {
        return Err("乗算の実行状態がラッチ出力でない。".into());
    };
    require_bound(
        &cone,
        running_index..running_index + 1,
        5,
        "実行状態から次のマークと完了値",
    )?;
    require_bound(&cone, bits, 4, "マークから次のマークと完了値")?;
    Ok(())
}

/// フェッチとオペランドの取り込み、通常書き込みの遅延を同じ issue 配線へ結び付ける。
fn verify_instruction_controls(
    vm: &ClockedVm,
    pc: usize,
    instruction: usize,
    first: usize,
    second: usize,
    fetch: usize,
    retire: Wire,
) -> Check<()> {
    let net = vm.netlist();
    let (instruction_latch, instruction_range) = latch(vm, Wire::Input(instruction), 33)?;
    let capture = instruction_latch.enable;
    let issue = delay_source(vm, capture, 72)?;

    let (pc_latch, pc_range) = latch(vm, Wire::Input(pc), 64)?;
    let commit = net.outputs[194];
    let (commit_retire, valid) = writer_inputs(vm, commit)?;
    let (issue_latch, _) = latch(vm, issue, 1)?;
    let started = delay_source(vm, issue_latch.enable, 2)?;
    let issue_pulses = or_inputs(net, issue_latch.data[0]).ok_or("命令枠の開始が OR でない。")?;
    let seed = issue_pulses
        .iter()
        .copied()
        .find(|&wire| wire != commit)
        .and_then(|wire| inverted_input(net, wire))
        .ok_or("命令枠の初期パルスがない。")?;
    if pc_latch.enable != commit
        || commit_retire != retire
        || started != Wire::Constant(true)
        || !issue_pulses.contains(&commit)
        || delay_source(vm, seed, 1)? != Wire::Constant(true)
    {
        return Err("PC の保持または初期・次回の命令開始回路が異なる。".into());
    }

    let (first_latch, first_range) = latch(vm, Wire::Input(first), 64)?;
    let (second_latch, second_range) = latch(vm, Wire::Input(second), 64)?;
    if delay_source(vm, first_latch.enable, 96)? != issue
        || delay_source(vm, second_latch.enable, 144)? != issue
        || first_latch.data != second_latch.data
    {
        return Err("オペランドの取り込み時刻またはレジスタの読み出し配線が異なる。".into());
    }

    let (mode, _) = latch(vm, Wire::Input(fetch), 1)?;
    let mode_control = or_inputs(net, mode.enable).ok_or("フェッチ選択の許可が OR でない。")?;
    if !mode_control.contains(&issue)
        || !mode_control.contains(&capture)
        || delay_source(vm, mode.data[0], 2)? != issue
    {
        return Err("フェッチ選択の開始または解除時刻を決める配線が異なる。".into());
    }

    let (retire_latch, _) = latch(vm, retire, 1)?;
    let [simple, long_finished] =
        or_inputs(net, retire_latch.data[0]).ok_or("退役要求が OR でない。")?;
    let [mul_finished, div_finished] =
        or_inputs(net, long_finished).ok_or("乗算と除算の完了通知が OR でない。")?;
    if delay_source(vm, retire_latch.enable, 2)? != Wire::Constant(true) {
        return Err("退役ラッチの初期化時刻が異なる。".into());
    }
    let (simple_latch, _) = latch(vm, simple, 1)?;
    let [ready, simple_operation] =
        and_inputs(net, simple_latch.data[0]).ok_or("通常退役要求が AND でない。")?;
    if delay_source(vm, simple_latch.enable, 4)? != Wire::Constant(true)
        || delay_source(vm, ready, 1024)? != issue
    {
        return Err("通常退役が同じ issue の1024 tick 後を起点とする経路を特定できない。".into());
    }
    let (finished, _) = latch(vm, mul_finished, 1)?;
    let [rising, running] =
        and_inputs(net, finished.data[0]).ok_or("乗算完了通知が AND でない。")?;
    let (run, _) = latch(vm, running, 1)?;
    let initialized = delay_source(vm, run.data[0], 2)?;
    if or_inputs(net, run.enable).and_then(|inputs| other_input(inputs, mul_finished))
        != Some(initialized)
        || delay_source(vm, finished.enable, 4)? != Wire::Constant(true)
    {
        return Err("乗算の開始と完了通知の接続が異なる。".into());
    }
    let start = delay_source(vm, initialized, 8)?;
    let [no_done, older_done] =
        nor_inputs(net, rising).ok_or("乗算完了の立ち上がりが NOR でない。")?;
    let done = inverted_input(net, no_done).ok_or("乗算完了の立ち上がりに done の反転がない。")?;
    if delay_source(vm, older_done, 2)? != done {
        return Err("乗算完了の立ち上がりが同じ done の2 tick 遅延を比較していない。".into());
    }
    verify_multiplier_round_marker(vm, done, running)?;
    let (done_latch, _) = latch(vm, done, 1)?;
    let capture = done_latch.enable;
    let (capture_latch, _) = latch(vm, capture, 1)?;
    let repeat = or_inputs(net, capture_latch.data[0])
        .and_then(|inputs| other_input(inputs, start))
        .ok_or("乗算状態の取り込みが開始か反復の OR でない。")?;
    let (repeat_latch, _) = latch(vm, repeat, 1)?;
    let [request, not_yet] = and_inputs(net, repeat_latch.data[0])
        .ok_or("乗算反復が遅延した要求と実行条件の AND でない。")?;
    if delay_source(vm, capture_latch.enable, 2)? != Wire::Constant(true)
        || delay_source(vm, repeat_latch.enable, 4)? != Wire::Constant(true)
        || delay_source(vm, request, 512)? != capture
        || and_inputs(net, not_yet) != Some([running, no_done])
    {
        return Err("乗算状態の取り込み間隔または反復の抑止条件が異なる。".into());
    }
    let (start_latch, _) = latch(vm, start, 1)?;
    let mul_request = and_inputs(net, start_latch.data[0])
        .and_then(|inputs| other_input(inputs, ready))
        .ok_or("通常退役と乗算で ready を共有していない。")?;
    let multiply = and_inputs(net, mul_request)
        .and_then(|inputs| other_input(inputs, valid))
        .ok_or("乗算の開始要求が commit と同じ命令有効信号を使っていない。")?;
    let divide = nor_inputs(net, simple_operation)
        .and_then(|inputs| other_input(inputs, multiply))
        .ok_or("通常退役は乗算・除算の判定を参照する必要がある。")?;
    if delay_source(vm, start_latch.enable, 4)? != Wire::Constant(true) {
        return Err("乗算の開始ラッチの初期化時刻が異なる。".into());
    }
    // 除算の開始も同じ命令枠と有効信号を使い、補正結果が安定してから完了を通知する。
    let (div_completion, _) = latch(vm, div_finished, 1)?;
    let [corrected, div_running] =
        and_inputs(net, div_completion.data[0]).ok_or("除算の完了通知が AND でない。")?;
    let div_rising = delay_source(vm, corrected, 512)?;
    let [div_no_done, div_older_done] =
        nor_inputs(net, div_rising).ok_or("除算完了の立ち上がりが NOR でない。")?;
    let div_done = inverted_input(net, div_no_done).ok_or("除算完了の反転がない。")?;
    let (div_run, _) = latch(vm, div_running, 1)?;
    let div_initialized = delay_source(vm, div_run.data[0], 2)?;
    let div_first = delay_source(vm, div_initialized, 8)?;
    let div_start = delay_source(vm, div_first, 512)?;
    let (div_start_latch, _) = latch(vm, div_start, 1)?;
    let div_request = and_inputs(net, div_start_latch.data[0])
        .and_then(|inputs| other_input(inputs, ready))
        .ok_or("除算は同じ ready を使う必要がある。")?;
    if delay_source(vm, div_older_done, 2)? != div_done
        || delay_source(vm, div_completion.enable, 4)? != Wire::Constant(true)
        || or_inputs(net, div_run.enable).and_then(|inputs| other_input(inputs, div_finished))
            != Some(div_initialized)
        || and_inputs(net, div_request).and_then(|inputs| other_input(inputs, valid)) != Some(divide)
        || delay_source(vm, div_start_latch.enable, 4)? != Wire::Constant(true)
    {
        return Err("除算の開始または完了の配線が異なる。".into());
    }
    // 2経路の NOR 段数が違うため、同じ命令種別信号を異なる tick に読む。
    // ready が届く前に信号が確定するよう、取り込み済みの命令だけに依存することを調べる。
    let operation_cone = depths(net, &[multiply, divide], &net.outputs[128..192])?;
    if operation_cone.read_depth.is_some()
        || operation_cone
            .inputs
            .iter()
            .enumerate()
            .any(|(index, depth)| depth.is_some() && !instruction_range.contains(&index))
    {
        return Err("乗算・除算の判定が取り込み済みの命令以外に依存する。".into());
    }
    require_bound(
        &operation_cone,
        instruction_range.clone(),
        14,
        "命令から乗算・除算の判定",
    )?;

    // 乗算中に RAM のデータ応答や積が変わっても、退役の有効性は保持値だけで決まる。
    let valid_cone = depths(net, &[valid], &net.outputs[128..192])?;
    let fields = [
        (pc_range, 32, "PC"),
        (
            instruction_range.start..instruction_range.end - 1,
            500,
            "命令",
        ),
        (
            instruction_range.end - 1..instruction_range.end,
            2,
            "フェッチ有効",
        ),
        (first_range, 488, "rs1"),
        (second_range, 68, "rs2"),
    ];
    if valid_cone.read_depth.is_some()
        || valid_cone.inputs.iter().enumerate().any(|(index, depth)| {
            depth.is_some() && !fields.iter().any(|(range, _, _)| range.contains(&index))
        })
    {
        return Err("退役の有効性が保持した命令、PC、オペランド以外に依存する。".into());
    }
    for (range, bound, label) in fields {
        require_bound(
            &valid_cone,
            range,
            bound,
            &format!("{label} から退役の有効性"),
        )?;
    }
    eprintln!("issue から命令・第1・第2取込: 73 / 97 / 145 tick、通常の書き込み: 1033 tick");
    Ok(())
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

    let (retire, permission) = writer_inputs(vm, net.outputs[196])?;
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
    let second_operand = second_operand.ok_or("書き込み許可の rs2 ラッチがない。")?;
    require_bound(
        &write,
        second_operand.clone(),
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
    verify_instruction_controls(
        vm,
        pc.start,
        instruction.start,
        operand.start,
        second_operand.start,
        fetch,
        retire,
    )?;
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
