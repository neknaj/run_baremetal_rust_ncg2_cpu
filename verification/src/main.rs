//! NCG2 が展開した回路と参照仕様の差を探す SMT-LIB を出力する。

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
    let Some(selected @ ("add" | "shift-left" | "boot-ram-address")) =
        args.get(1).map(String::as_str)
    else {
        return Err("add、shift-left、boot-ram-address のいずれかを指定する。".into());
    };
    let arithmetic = ["decode.ncg2", "arithmetic.ncg2"];
    let boot = [
        "decode.ncg2",
        "boot-rom.ncg2",
        "fetch-boot.ncg2",
        "arithmetic.ncg2",
        "program-counter.ncg2",
        "register-file.ncg2",
        "execute-boot.ncg2",
        "boot-machine.ncg2",
    ];
    let paths = if selected == "boot-ram-address" {
        &boot[..]
    } else {
        &arithmetic[..]
    };
    let sources = paths
        .iter()
        .map(|path| std::fs::read_to_string(format!("circuit/{path}")))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshots: Vec<_> = paths
        .iter()
        .zip(&sources)
        .map(|(&path, text)| SourceSnapshot {
            path,
            revision: 0,
            text,
        })
        .collect();
    let project = parse_circuit_project(&snapshots).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let file = paths.len() - 1;
    let circuit = &project.circuits()[file];
    let declaration = circuit
        .names
        .iter()
        .find(|n| n.name.word == selected)
        .ok_or("検証対象の回路の宣言が見つからない。")?;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|e| e.form == declaration.form)
        .ok_or("検証対象の回路の索引が見つからない。")?;
    let built = aliases
        .expand_reachable_function((file, entry), 65536, 1_048_576, 1_048_576)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("検証対象の回路の展開結果がない。")?;
    let NorNetlist {
        input_count,
        gates,
        outputs,
    } = built.netlist();
    let expected_shape = match selected {
        "add" => (128, 64),
        "shift-left" => (70, 64),
        "boot-ram-address" => (64, 12),
        _ => unreachable!(),
    };
    assert_eq!((*input_count, outputs.len()), expected_shape);

    println!("(set-logic QF_BV)");
    println!("(set-option :timeout 30000)");
    for index in 0..*input_count {
        println!("(declare-const i{index} Bool)");
    }
    for (index, gate) in gates.iter().enumerate() {
        println!(
            "(define-fun g{index} () Bool (not (or {} {})))",
            name(gate.left),
            name(gate.right)
        );
    }
    let first = vector((0..64).map(|i| format!("i{i}")));
    if selected == "boot-ram-address" {
        let mapped = name(outputs[0]);
        let word = vector(outputs[1..].iter().copied().map(name));
        println!("(define-fun address () (_ BitVec 64) {first})");
        println!(
            "(define-fun in-range () Bool (and (bvuge address #x000000000002c000) (bvult address #x0000000000030000) (= (bvurem address (_ bv8 64)) (_ bv0 64))))"
        );
        println!(
            "(define-fun word () (_ BitVec 64) (bvudiv (bvsub address #x000000000002c000) (_ bv8 64)))"
        );
        println!(
            "(assert (or (distinct {mapped} in-range) (and in-range (distinct ((_ zero_extend 53) {word}) word))))"
        );
    } else {
        let rest = vector((64..*input_count).map(|i| format!("i{i}")));
        let output = vector(outputs.iter().copied().map(name));
        let expected = match selected {
            "add" => format!("(bvadd {first} {rest})"),
            "shift-left" => format!("(bvshl {first} ((_ zero_extend 58) {rest}))"),
            _ => unreachable!(),
        };
        println!("(assert (distinct {output} {expected}))");
    }
    println!("(check-sat)");
    Ok(())
}
