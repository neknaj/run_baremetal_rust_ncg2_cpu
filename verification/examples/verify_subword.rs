//! Core の実展開を、リトルエンディアンの語に対する独立仕様と照合する。

use circuitgame2_core::{
    netlist::{NorNetlist, Wire},
    project::{SourceSnapshot, check_use_aliases, index_use_targets, parse_circuit_project},
};

const SOURCES: &[(&str, &str)] = &[
    ("decode.ncg2", include_str!("../../circuit/decode.ncg2")),
    (
        "arithmetic.ncg2",
        include_str!("../../circuit/arithmetic.ncg2"),
    ),
    (
        "subword-memory.ncg2",
        include_str!("../../circuit/subword-memory.ncg2"),
    ),
];

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
    let joined = bits.iter().skip(1).fold(bits[0].clone(), |prefix, bit| {
        format!("(concat {prefix} {bit})")
    });
    println!("(define-fun {port} () (_ BitVec {}) {joined})", bits.len());
}

fn depth(wire: Wire, depths: &[usize]) -> usize {
    match wire {
        Wire::Input(_) | Wire::Constant(_) => 0,
        Wire::Gate(index) => depths[index],
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let (declaration, expected_shape, depth_limit) = match args.get(1).map(String::as_str) {
        Some("read-byte") => ("read-byte", (68, 64), 14),
        Some("read-halfword") => ("read-halfword", (68, 65), 10),
        Some("merge-store") => ("merge-store", (134, 65), 28),
        _ => {
            return Err(
                "対象の read-byte、read-halfword、merge-store のいずれかを指定する。".into(),
            );
        }
    };
    let snapshots: Vec<_> = SOURCES
        .iter()
        .map(|&(path, text)| SourceSnapshot {
            path,
            revision: 0,
            text,
        })
        .collect();
    let project = parse_circuit_project(&snapshots).map_err(|e| format!("{e:?}"))?;
    let targets = index_use_targets(&project).map_err(|e| format!("{e:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|e| format!("{e:?}"))?;
    let file = SOURCES.len() - 1;
    let circuit = &project.circuits()[file];
    let form = circuit
        .names
        .iter()
        .find(|name| name.name.word == declaration)
        .ok_or("検証対象の func が見つからない。")?
        .form;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == form)
        .ok_or("検証対象の func の索引が見つからない。")?;
    let built = aliases
        .expand_reachable_function((file, entry), 65_536, 1_048_576, 1_048_576)
        .map_err(|e| format!("{e:?}"))?
        .ok_or("実 NOR への展開結果がない。")?;
    let NorNetlist {
        input_count,
        gates,
        outputs,
    } = built.netlist();
    assert_eq!((*input_count, outputs.len()), expected_shape);

    println!("(set-logic QF_BV)");
    for index in 0..*input_count {
        println!("(declare-const i{index} Bool)");
    }
    let mut depths = Vec::with_capacity(gates.len());
    for (index, gate) in gates.iter().enumerate() {
        println!(
            "(define-fun g{index} () Bool (not (or {} {})))",
            name(gate.left),
            name(gate.right),
        );
        depths.push(1 + depth(gate.left, &depths).max(depth(gate.right, &depths)));
    }

    emit_vector("word", (0..64).map(Wire::Input));
    if declaration != "merge-store" {
        emit_vector("offset", (64..67).map(Wire::Input));
        if declaration == "read-byte" {
            emit_vector("actual", outputs.iter().copied());
            print!("{}", include_str!("read_byte.smt2"));
        } else {
            println!("(define-fun aligned () Bool {})", name(outputs[0]));
            emit_vector("actual", outputs[1..].iter().copied());
            print!("{}", include_str!("read_halfword.smt2"));
        }
    } else {
        emit_vector("value", (64..128).map(Wire::Input));
        emit_vector("offset", (128..131).map(Wire::Input));
        emit_vector("funct3", (131..134).map(Wire::Input));
        println!("(define-fun valid () Bool {})", name(outputs[0]));
        emit_vector("actual", outputs[1..].iter().copied());
        print!("{}", include_str!("merge_store.smt2"));
    }
    println!("(check-sat)");

    let max_depth = outputs
        .iter()
        .map(|&wire| depth(wire, &depths))
        .max()
        .unwrap();
    assert!(max_depth <= depth_limit);
    eprintln!("{declaration}: {} NORs, depth {max_depth}", gates.len());
    Ok(())
}
