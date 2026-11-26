//! 実 NOR のフェッチを GNU binutils の命令列と照合する SMT-LIB を出力する。

use std::{error::Error, fs};

use circuitgame2_core::{
    netlist::Wire,
    project::{SourceSnapshot, check_use_aliases, index_use_targets, parse_circuit_project},
};

const SOURCES: &[(&str, &str)] = &[
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
        "uart-boot-rom.ncg2",
        include_str!("../../circuit/uart-boot-rom.ncg2"),
    ),
    (
        "uart-boot-fetch.ncg2",
        include_str!("../../circuit/uart-boot-fetch.ncg2"),
    ),
];

fn name(wire: Wire) -> String {
    match wire {
        Wire::Input(index) => format!("i{index}"),
        Wire::Gate(index) => format!("g{index}"),
        Wire::Constant(value) => value.to_string(),
    }
}

fn vector(wires: impl Iterator<Item = Wire> + DoubleEndedIterator) -> String {
    let mut bits = wires
        .rev()
        .map(|wire| format!("(ite {} #b1 #b0)", name(wire)));
    let first = bits.next().unwrap();
    bits.fold(first, |joined, bit| format!("(concat {joined} {bit})"))
}

fn depth(wire: Wire, earlier: &[usize]) -> usize {
    match wire {
        Wire::Input(_) | Wire::Constant(_) => 0,
        Wire::Gate(index) => earlier[index],
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let [image] = &std::env::args_os().skip(1).collect::<Vec<_>>()[..] else {
        return Err("GNU objcopy の .text 出力を指定する。".into());
    };
    let binary = fs::read(image)?;
    if binary.is_empty() || binary.len() > 128 || binary.len() % 4 != 0 {
        return Err("ROM の命令列は4バイトから128バイトまでの4の倍数とする。".into());
    }
    let mut padded = [0; 128];
    padded[..binary.len()].copy_from_slice(&binary);
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

    for (file, selected, input_count, output_count) in
        [(4, "uart-boot-rom", 4, 64), (5, "uart-boot-fetch", 64, 33)]
    {
        let circuit = &project.circuits()[file];
        let form = circuit
            .names
            .iter()
            .find(|decl| decl.name.word == selected)
            .ok_or("検証対象の宣言が見つからない。")?
            .form;
        let entry = circuit
            .syntax
            .entries
            .iter()
            .position(|entry| entry.form == form)
            .ok_or("検証対象の索引が見つからない。")?;
        let expanded = aliases
            .expand_reachable_function((file, entry), 65_536, 1_048_576, 1_048_576)
            .map_err(|error| format!("{error:?}"))?
            .ok_or("展開済み NOR が得られない。")?;
        let net = expanded.netlist();
        assert_eq!(
            (net.input_count, net.outputs.len()),
            (input_count, output_count)
        );
        if selected == "uart-boot-rom" {
            for (index, bytes) in padded.chunks_exact(8).enumerate() {
                let input: [bool; 4] = std::array::from_fn(|bit| index & (1 << bit) != 0);
                let output = net.evaluate(&input).map_err(|error| format!("{error:?}"))?;
                let value = output
                    .iter()
                    .enumerate()
                    .fold(0, |acc, (bit, &b)| acc | (u64::from(b) << bit));
                assert_eq!(
                    value,
                    u64::from_le_bytes(bytes.try_into()?),
                    "ROM の語 {index}"
                );
            }
            eprintln!("ROM の全16語が一致。NOR は{}個。", net.gates.len());
            continue;
        }

        println!("(set-logic QF_BV)");
        for input in 0..net.input_count {
            println!("(declare-const i{input} Bool)");
        }
        let mut depths = Vec::with_capacity(net.gates.len());
        for (index, gate) in net.gates.iter().enumerate() {
            println!(
                "(define-fun g{index} () Bool (not (or {} {})))",
                name(gate.left),
                name(gate.right)
            );
            depths.push(1 + depth(gate.left, &depths).max(depth(gate.right, &depths)));
        }
        let maximum = net
            .outputs
            .iter()
            .map(|&wire| depth(wire, &depths))
            .max()
            .unwrap();
        eprintln!("{selected}: {} NORs, depth {maximum}", net.gates.len());
        let pc = vector((0..64).map(Wire::Input));
        let actual = vector(net.outputs[..32].iter().copied());
        let valid = name(net.outputs[32]);
        let mut expected = String::from("#x00000000");
        for (index, word) in padded.chunks_exact(4).enumerate().rev() {
            let address = index * 4;
            let value = u32::from_le_bytes(word.try_into()?);
            expected = format!("(ite (= pc (_ bv{address} 64)) #x{value:08x} {expected})");
        }
        println!("(define-fun pc () (_ BitVec 64) {pc})");
        println!(
            "(define-fun mapped () Bool (and (bvult pc (_ bv128 64)) (= (bvurem pc (_ bv4 64)) (_ bv0 64))))"
        );
        println!(
            "(assert (or (distinct {valid} mapped) (and mapped (distinct {actual} {expected}))))"
        );
        println!("(check-sat)");
    }
    Ok(())
}
