//! セルフホスト用のアドレス判定と ROM フェッチを展開済み NOR で検証する。

use circuitgame2_core::{
    netlist::Wire,
    project::{SourceSnapshot, check_use_aliases, index_use_targets, parse_circuit_project},
};

const FIXTURE: &str = include_str!("../fixtures/selfhost-rom.ncg2");

fn name(wire: Wire) -> String {
    match wire {
        Wire::Input(index) => format!("i{index}"),
        Wire::Gate(index) => format!("g{index}"),
        Wire::Constant(value) => value.to_string(),
    }
}

fn emit_vector(port: &str, wires: impl Iterator<Item = Wire> + DoubleEndedIterator) {
    let bits: Vec<_> = wires
        .rev()
        .map(|wire| format!("(ite {} #b1 #b0)", name(wire)))
        .collect();
    let folded = bits.iter().skip(1).fold(bits[0].clone(), |prefix, bit| {
        format!("(concat {prefix} {bit})")
    });
    println!("(define-fun {port} () (_ BitVec {}) {folded})", bits.len());
}

/// 定数を除く各入力から出力までの最短・最長の辺数を返す。
fn input_paths(wire: Wire, previous: &[Option<(usize, usize)>]) -> Option<(usize, usize)> {
    match wire {
        Wire::Constant(_) => None,
        Wire::Input(_) => Some((0, 0)),
        Wire::Gate(index) => previous[index],
    }
}

fn depth(wire: Wire, previous: &[usize]) -> usize {
    match wire {
        Wire::Input(_) | Wire::Constant(_) => 0,
        Wire::Gate(index) => previous[index],
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let (selected, rom, measure) =
        match args.as_slice() {
            [mode] if mode == "fetch" => ("firmware-fetch", FIXTURE.to_owned(), false),
            [mode] if mode == "access" => ("firmware-data-access", FIXTURE.to_owned(), false),
            [mode] if mode == "banks" => ("selfhost-outside-banks", FIXTURE.to_owned(), false),
            [mode, path] if mode == "measure-fetch" => {
                ("firmware-fetch", std::fs::read_to_string(path)?, true)
            }
            [mode, path] if mode == "measure-rom" => {
                ("firmware-rom", std::fs::read_to_string(path)?, true)
            }
            _ => return Err(
                "fetch、access、banks、measure-fetch ROM、measure-rom ROM のいずれかを指定する。"
                    .into(),
            ),
        };
    let sources = [
        ("decode.ncg2", include_str!("../../circuit/decode.ncg2")),
        ("boot-rom.ncg2", include_str!("../../circuit/boot-rom.ncg2")),
        (
            "fetch-boot.ncg2",
            include_str!("../../circuit/fetch-boot.ncg2"),
        ),
        (
            "arithmetic.ncg2",
            include_str!("../../circuit/arithmetic.ncg2"),
        ),
        (
            "execute-boot.ncg2",
            include_str!("../../circuit/execute-boot.ncg2"),
        ),
        ("firmware-rom.ncg2", rom.as_str()),
        (
            "selfhost-memory.ncg2",
            include_str!("../../circuit/selfhost-memory.ncg2"),
        ),
    ];
    let snapshots: Vec<_> = sources
        .iter()
        .map(|&(path, text)| SourceSnapshot {
            path,
            revision: 0,
            text,
        })
        .collect();
    let project = parse_circuit_project(&snapshots).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let file = if selected == "firmware-rom" { 5 } else { 6 };
    let circuit = &project.circuits()[file];
    let form = circuit
        .names
        .iter()
        .find(|name| name.name.word == selected)
        .ok_or("検証対象の func が見つからない。")?
        .form;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == form)
        .ok_or("検証対象の func の索引が見つからない。")?;
    let expanded = aliases
        .expand_reachable_function((file, entry), 65_536, 1_048_576, 1_048_576)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("展開済み NOR が得られない。")?;
    let netlist = expanded.netlist();
    let expected = match selected {
        "firmware-fetch" => (64, 33),
        "firmware-data-access" => (64, 2),
        "selfhost-outside-banks" => (74, 4),
        "firmware-rom" => (12, 64),
        _ => unreachable!(),
    };
    assert_eq!((netlist.input_count, netlist.outputs.len()), expected);
    let mut paths = Vec::with_capacity(netlist.gates.len());
    let mut depths = Vec::with_capacity(netlist.gates.len());
    for gate in &netlist.gates {
        depths.push(1 + depth(gate.left, &depths).max(depth(gate.right, &depths)));
        let left = input_paths(gate.left, &paths);
        let right = input_paths(gate.right, &paths);
        let range = match (left, right) {
            (Some((a, b)), Some((c, d))) => Some((a.min(c), b.max(d))),
            _ => left.or(right),
        };
        paths.push(range.map(|(min, max)| (min + 1, max + 1)));
    }
    let output_paths: Vec<_> = netlist
        .outputs
        .iter()
        .map(|&wire| input_paths(wire, &paths))
        .collect();
    let maximum = netlist
        .outputs
        .iter()
        .map(|&wire| depth(wire, &depths))
        .max()
        .unwrap();
    if selected == "selfhost-outside-banks" {
        assert_eq!(output_paths, [Some((14, 14)); 4]);
    }
    eprintln!("{selected}: {} NORs, depth {maximum}", netlist.gates.len());
    if measure {
        return Ok(());
    }
    println!("(set-logic QF_BV)");
    for index in 0..netlist.input_count {
        println!("(declare-const i{index} Bool)");
    }
    for (index, gate) in netlist.gates.iter().enumerate() {
        println!(
            "(define-fun g{index} () Bool (not (or {} {})))",
            name(gate.left),
            name(gate.right)
        );
    }
    emit_vector("address", (0..64).map(Wire::Input));
    if selected == "firmware-fetch" {
        emit_vector("actual", netlist.outputs[..32].iter().copied());
        println!("(define-fun valid () Bool {})", name(netlist.outputs[32]));
        print!("{}", include_str!("selfhost_fetch.smt2"));
    } else {
        print!("{}", include_str!("selfhost_regions.smt2"));
        if selected == "firmware-data-access" {
            println!(
                "(assert (or (distinct {} (or rom bss mailbox stack)) (distinct {} (or bss mailbox stack))))",
                name(netlist.outputs[0]),
                name(netlist.outputs[1])
            );
        } else {
            emit_vector("inverted", (64..74).map(Wire::Input));
            println!("(assert (= inverted (bvnot ((_ extract 17 8) address))))");
            println!("(assert (or");
            for (port, &wire) in ["rom", "bss", "mailbox", "stack"]
                .into_iter()
                .zip(&netlist.outputs)
            {
                println!("(distinct {} (not {port}))", name(wire));
            }
            println!("))");
        }
    }
    println!("(check-sat)");
    Ok(())
}
