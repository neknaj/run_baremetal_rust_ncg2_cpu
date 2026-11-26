//! NCG2 が展開した算術回路と仕様の差を探す SMT-LIB を出力する。

use circuitgame2_core::{
    netlist::{NorNetlist, Wire},
    project::{SourceSnapshot, check_use_aliases, index_use_targets, parse_circuit_project},
};

fn name(wire: Wire) -> String {
    match wire {
        Wire::Input(at) => format!("i{at}"),
        Wire::Gate(at) => format!("g{at}"),
        Wire::Constant(value) => value.to_string(),
    }
}

fn vector(bits: impl Iterator<Item = String> + DoubleEndedIterator) -> String {
    let mut reversed = bits.rev().map(|bit| format!("(ite {bit} #b1 #b0)"));
    let highest = reversed.next().unwrap();
    reversed.fold(highest, |prefix, bit| format!("(concat {prefix} {bit})"))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let Some(selected @ ("add" | "shift-left")) = args.get(1).map(String::as_str) else {
        return Err("add または shift-left を指定する。".into());
    };
    let source = std::fs::read_to_string("circuit/decode.ncg2")?;
    let arithmetic = std::fs::read_to_string("circuit/arithmetic.ncg2")?;
    let snapshots = [
        SourceSnapshot { path: "decode.ncg2", revision: 0, text: &source },
        SourceSnapshot { path: "arithmetic.ncg2", revision: 0, text: &arithmetic },
    ];
    let project = parse_circuit_project(&snapshots).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let circuit = &project.circuits()[1];
    let declaration = circuit.names.iter().find(|n| n.name.word == selected)
        .ok_or("算術回路の宣言が見つからない。")?;
    let entry = circuit.syntax.entries.iter().position(|e| e.form == declaration.form)
        .ok_or("算術回路の索引が見つからない。")?;
    let built = aliases.expand_reachable_function((1, entry), 65536, 1_048_576, 1_048_576)
        .map_err(|error| format!("{error:?}"))?.ok_or("算術回路の展開結果がない。")?;
    let NorNetlist { input_count, gates, outputs } = built.netlist();
    let expected_inputs = if selected == "add" { 128 } else { 70 };
    assert_eq!(*input_count, expected_inputs);
    assert_eq!(outputs.len(), 64);

    println!("(set-logic QF_BV)");
    println!("(set-option :timeout 30000)");
    for index in 0..*input_count {
        println!("(declare-const i{index} Bool)");
    }
    for (index, gate) in gates.iter().enumerate() {
        println!("(define-fun g{index} () Bool (not (or {} {})))", name(gate.left), name(gate.right));
    }
    let first = vector((0..64).map(|i| format!("i{i}")));
    let rest = vector((64..*input_count).map(|i| format!("i{i}")));
    let output = vector(outputs.iter().copied().map(name));
    let expected = match selected {
        "add" => format!("(bvadd {first} {rest})"),
        "shift-left" => format!("(bvshl {first} ((_ zero_extend 58) {rest}))"),
        _ => unreachable!(),
    };
    println!("(assert (distinct {output} {expected}))");
    println!("(check-sat)");
    Ok(())
}
