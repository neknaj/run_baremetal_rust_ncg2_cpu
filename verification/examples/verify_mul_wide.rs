//! 桁上がりを含む1回分の更新を、展開済み NOR と129ビットの仕様で照合する。

use circuitgame2_core::{
    netlist::Wire,
    project::{SourceSnapshot, check_use_aliases, index_use_targets, parse_circuit_project},
};

const SOURCES: &[(&str, &str)] = &[
    ("decode.ncg2", include_str!("../../circuit/decode.ncg2")),
    (
        "arithmetic.ncg2",
        include_str!("../../circuit/arithmetic.ncg2"),
    ),
    (
        "multiply-wide.ncg2",
        include_str!("../../circuit/multiply-wide.ncg2"),
    ),
];

fn name(wire: Wire) -> String {
    match wire {
        Wire::Input(index) => format!("i{index}"),
        Wire::Gate(index) => format!("g{index}"),
        Wire::Constant(value) => value.to_string(),
    }
}

fn emit_word(port: &str, wires: impl Iterator<Item = Wire> + DoubleEndedIterator) {
    let mut bits = wires
        .rev()
        .map(|wire| format!("(ite {} #b1 #b0)", name(wire)));
    let first = bits.next().unwrap();
    let joined = bits.fold(first, |prefix, bit| format!("(concat {prefix} {bit})"));
    println!("(define-fun {port} () (_ BitVec 64) {joined})");
}

fn depth(wire: Wire, depths: &[usize]) -> usize {
    match wire {
        Wire::Input(_) | Wire::Constant(_) => 0,
        Wire::Gate(index) => depths[index],
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let snapshots: Vec<_> = SOURCES
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
    let file = SOURCES.len() - 1;
    let circuit = &project.circuits()[file];
    let declaration = circuit
        .names
        .iter()
        .find(|name| name.name.word == "mul-wide-round")
        .ok_or("mul-wide-round の宣言が見つからない。")?;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == declaration.form)
        .ok_or("mul-wide-round の索引が見つからない。")?;
    let built = aliases
        .expand_reachable_function((file, entry), 65_536, 1_048_576, 1_048_576)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("mul-wide-round を NOR に展開できない。")?;
    let netlist = built.netlist();
    assert_eq!((netlist.input_count, netlist.outputs.len()), (386, 257));

    println!("(set-logic QF_BV)");
    println!("(set-option :timeout 30000)");
    for index in 0..netlist.input_count {
        println!("(declare-const i{index} Bool)");
    }
    let mut depths = Vec::with_capacity(netlist.gates.len());
    for (index, gate) in netlist.gates.iter().enumerate() {
        println!(
            "(define-fun g{index} () Bool (not (or {} {})))",
            name(gate.left),
            name(gate.right),
        );
        depths.push(1 + depth(gate.left, &depths).max(depth(gate.right, &depths)));
    }
    for (port, start) in [
        ("multiplicand", 0),
        ("lower", 64),
        ("upper", 128),
        ("mark", 192),
        ("left", 257),
        ("right", 321),
    ] {
        emit_word(port, (start..start + 64).map(Wire::Input));
    }
    for (port, start) in [
        ("next-multiplicand", 0),
        ("next-lower", 64),
        ("next-upper", 128),
        ("next-mark", 192),
    ] {
        emit_word(port, netlist.outputs[start..start + 64].iter().copied());
    }
    println!("(define-fun done () Bool i256)");
    println!("(define-fun initialize () Bool i385)");
    println!(
        "(define-fun next-done () Bool {})",
        name(netlist.outputs[256])
    );
    print!("{}", include_str!("mul_wide_round.smt2"));

    let maximum = netlist
        .outputs
        .iter()
        .map(|&wire| depth(wire, &depths))
        .max()
        .unwrap();
    eprintln!(
        "mul-wide-round: {} NORs, depth {maximum}",
        netlist.gates.len()
    );
    Ok(())
}
