//! 物理 PC と RAM フェッチの実 NOR を照合する SMT-LIB を出力する。

use std::error::Error;

use circuitgame2_core::{
    netlist::Wire,
    project::{SourceSnapshot, check_use_aliases, index_use_targets, parse_circuit_project},
};

fn name(wire: Wire) -> String {
    match wire {
        Wire::Input(index) => format!("i{index}"),
        Wire::Gate(index) => format!("g{index}"),
        Wire::Constant(value) => value.to_string(),
    }
}

fn define_bits(symbol: &str, wires: &[Wire]) {
    let vector = wires
        .iter()
        .rev()
        .map(|&wire| format!("(ite {} #b1 #b0)", name(wire)))
        .reduce(|high, bit| format!("(concat {high} {bit})"))
        .unwrap();
    println!(
        "(define-fun {symbol} () (_ BitVec {}) {vector})",
        wires.len()
    );
}

fn main() -> Result<(), Box<dyn Error>> {
    let sources = [
        ("decode.ncg2", include_str!("../../circuit/decode.ncg2")),
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
        (
            "execute-control.ncg2",
            include_str!("../../circuit/execute-control.ncg2"),
        ),
        (
            "execute-alu.ncg2",
            include_str!("../../circuit/execute-alu.ncg2"),
        ),
        (
            "boot-machine.ncg2",
            include_str!("../../circuit/boot-machine.ncg2"),
        ),
        (
            "selfhost-map.ncg2",
            include_str!("../../circuit/selfhost-map.ncg2"),
        ),
        ("uart-pc.ncg2", include_str!("../../circuit/uart-pc.ncg2")),
        (
            "uart-firmware-machine.ncg2",
            include_str!("../../circuit/uart-firmware-machine.ncg2"),
        ),
        ("uart-pc-proof.ncg2", include_str!("uart_pc.ncg2")),
    ];
    let snapshots = sources.map(|(path, text)| SourceSnapshot {
        path,
        text,
        revision: 0,
    });
    let project = parse_circuit_project(&snapshots).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let file = sources.len() - 1;
    let circuit = &project.circuits()[file];
    let form = circuit
        .names
        .iter()
        .find(|item| item.name.word == "prove-uart-pc")
        .ok_or("PC 検証用の関数名を索引から得られない。")?
        .form;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == form)
        .ok_or("PC 検証用の関数を構文から得られない。")?;
    let function = aliases
        .expand_reachable_function((file, entry), 65_536, 1_048_576, 1_048_576)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("PC 検証用の回路を展開できない。")?;
    let net = function.netlist();
    assert_eq!((net.input_count, net.outputs.len()), (224, 355));

    println!("(set-logic QF_BV)");
    for index in 0..net.input_count {
        println!("(declare-const i{index} Bool)");
    }
    let mut depths = Vec::with_capacity(net.gates.len());
    let depth = |wire, known: &[usize]| match wire {
        Wire::Input(_) | Wire::Constant(_) => 0,
        Wire::Gate(index) => known[index],
    };
    for (index, gate) in net.gates.iter().enumerate() {
        println!(
            "(define-fun g{index} () Bool (not (or {} {})))",
            name(gate.left),
            name(gate.right)
        );
        depths.push(1 + depth(gate.left, &depths).max(depth(gate.right, &depths)));
    }

    let inputs: Vec<_> = (0..net.input_count).map(Wire::Input).collect();
    for (symbol, wires) in [
        ("pc", &inputs[..64]),
        ("instruction", &inputs[64..96]),
        ("rs1", &inputs[96..160]),
        ("ram", &inputs[160..224]),
        ("offset", &net.outputs[..64]),
        ("round-trip", &net.outputs[64..128]),
        ("next", &net.outputs[128..192]),
        ("register", &net.outputs[192..256]),
        ("word-address", &net.outputs[258..322]),
        ("fetched", &net.outputs[323..355]),
    ] {
        define_bits(symbol, wires);
    }
    for (symbol, index) in [
        ("valid", 256),
        ("write-register", 257),
        ("fetch-valid", 322),
    ] {
        println!("(define-fun {symbol} () Bool {})", name(net.outputs[index]));
    }
    let maximum = |wires: &[Wire]| {
        wires
            .iter()
            .map(|&wire| depth(wire, &depths))
            .max()
            .unwrap()
    };
    eprintln!(
        "{} NOR。全体{}段、PCの減算{}段、フェッチ有効{}段、語内選択{}段。",
        net.gates.len(),
        maximum(&net.outputs),
        maximum(&net.outputs[..64]),
        depth(net.outputs[322], &depths),
        maximum(&net.outputs[323..355]),
    );
    print!("{}", include_str!("uart_pc.smt2"));
    Ok(())
}
