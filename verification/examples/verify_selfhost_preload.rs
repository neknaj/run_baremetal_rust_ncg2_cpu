//! 起動前後の書き込み選択を独立仕様と照合し、同一フレームにそろうことを確認する。

use circuitgame2_core::{
    netlist::Wire,
    project::{SourceSnapshot, check_use_aliases, index_use_targets, parse_circuit_project},
};

#[path = "../fixtures/preload.rs"]
mod fixture;

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

/// 対象の入力から出力までの最短経路と最長経路を求める。
fn input_paths(
    wire: Wire,
    input_limit: usize,
    earlier: &[Option<(usize, usize)>],
) -> Option<(usize, usize)> {
    match wire {
        Wire::Input(index) => (index < input_limit).then_some((0, 0)),
        Wire::Gate(index) => earlier[index],
        Wire::Constant(_) => None,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let snapshots: Vec<_> = fixture::SOURCES
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
    let file = snapshots.len() - 1;
    let circuit = &project.circuits()[file];

    for (selected, active_inputs) in [
        ("selfhost-request-word", 5),
        ("selfhost-route-request", 258),
    ] {
        let form = circuit
            .names
            .iter()
            .find(|entry| entry.name.word == selected)
            .ok_or("検証対象の宣言が見つからない。")?
            .form;
        let entry = circuit
            .syntax
            .entries
            .iter()
            .position(|entry| entry.form == form)
            .ok_or("検証対象の索引が見つからない。")?;
        let built = aliases
            .expand_reachable_function((file, entry), 65_536, 1_048_576, 1_048_576)
            .map_err(|error| format!("{error:?}"))?
            .ok_or("展開結果を得られない。")?;
        let netlist = built.netlist();
        let mut paths = Vec::with_capacity(netlist.gates.len());
        for gate in &netlist.gates {
            let left = input_paths(gate.left, active_inputs, &paths);
            let right = input_paths(gate.right, active_inputs, &paths);
            let combined = match (left, right) {
                (Some((a, b)), Some((c, d))) => Some((a.min(c), b.max(d))),
                _ => left.or(right),
            };
            paths.push(combined.map(|(min, max)| (min + 1, max + 1)));
        }
        let output_paths: Vec<_> = netlist
            .outputs
            .iter()
            .map(|&wire| input_paths(wire, active_inputs, &paths))
            .collect();
        if selected == "selfhost-request-word" {
            assert_eq!((netlist.input_count, netlist.outputs.len()), (5, 134));
            // 15→16 のように添字の複数ビットが変わっても停止判定は同じ添字を見る。
            assert_eq!(output_paths[133], Some((7, 7)));
            let maximum = output_paths
                .iter()
                .flatten()
                .map(|&(_, max)| max)
                .max()
                .unwrap();
            eprintln!("{selected}: {} NORs, depth {maximum}", netlist.gates.len());
        } else {
            assert_eq!((netlist.input_count, netlist.outputs.len()), (260, 129));
            // 選択状態が確定した間、各要求のアドレス、値、許可の全経路は4段である。
            assert_eq!(output_paths, [Some((4, 4)); 129]);
            eprintln!(
                "{selected}: {} NORs, all frame paths have depth 4",
                netlist.gates.len()
            );
            println!("(set-logic QF_BV)");
            for input in 0..netlist.input_count {
                println!("(declare-const i{input} Bool)");
            }
            for (index, gate) in netlist.gates.iter().enumerate() {
                println!(
                    "(define-fun g{index} () Bool (not (or {} {})))",
                    name(gate.left),
                    name(gate.right)
                );
            }
            let cpu = vector((0..128).map(Wire::Input));
            let request = vector((129..257).map(Wire::Input));
            let actual = vector(netlist.outputs[..128].iter().copied());
            let actual_write = name(netlist.outputs[128]);
            println!("(define-fun cpu () (_ BitVec 128) {cpu})");
            println!("(define-fun preload () (_ BitVec 128) {request})");
            println!("(assert (or (distinct {actual} (ite i258 cpu preload))");
            println!("   (distinct {actual_write} (or (and (not i258) i257) (and i259 i128)))))");
            println!("(check-sat)");
        }
    }
    Ok(())
}
