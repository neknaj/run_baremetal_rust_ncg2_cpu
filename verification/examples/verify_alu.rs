//! 整数命令の実 NOR を、命令ビットと64ビット演算で定めた仕様と照合する。

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
        "execute-alu.ncg2",
        include_str!("../../circuit/execute-alu.ncg2"),
    ),
];

fn name(wire: Wire) -> String {
    match wire {
        Wire::Input(index) => format!("i{index}"),
        Wire::Gate(index) => format!("g{index}"),
        Wire::Constant(value) => value.to_string(),
    }
}

fn vector(bits: impl Iterator<Item = Wire> + DoubleEndedIterator) -> String {
    let mut reversed = bits.rev().map(|bit| format!("(ite {} #b1 #b0)", name(bit)));
    let first = reversed.next().unwrap();
    reversed.fold(first, |prefix, bit| format!("(concat {prefix} {bit})"))
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
        .find(|name| name.name.word == "execute-alu")
        .ok_or("execute-alu の宣言が見つからない。")?;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == declaration.form)
        .ok_or("execute-alu の索引が見つからない。")?;
    let built = aliases
        .expand_reachable_function((file, entry), 65_536, 1_048_576, 1_048_576)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("execute-alu を NOR に展開できない。")?;
    let netlist = built.netlist();
    assert_eq!((netlist.input_count, netlist.outputs.len()), (160, 65));

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
    for (port, first, end) in [("instruction", 0, 32), ("rs1", 32, 96), ("rs2", 96, 160)] {
        let bits = vector((first..end).map(Wire::Input));
        println!("(define-fun {port} () (_ BitVec {}) {bits})", end - first);
    }
    println!("(define-fun valid () Bool {})", name(netlist.outputs[0]));
    let bits = vector(netlist.outputs[1..].iter().copied());
    println!("(define-fun result () (_ BitVec 64) {bits})");
    print!("{}", include_str!("integer_alu.smt2"));

    let maximum = netlist
        .outputs
        .iter()
        .map(|&wire| depth(wire, &depths))
        .max()
        .unwrap();
    assert!(maximum <= 469);
    eprintln!("execute-alu: {} NORs, depth {maximum}", netlist.gates.len());
    Ok(())
}
