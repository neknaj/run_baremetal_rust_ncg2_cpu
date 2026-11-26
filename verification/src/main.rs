//! NCG2 が展開した回路と参照仕様の差を探す SMT-LIB を出力する。

use circuitgame2_core::{
    netlist::{NorNetlist, Wire},
    project::{SourceSnapshot, check_use_aliases, index_use_targets, parse_circuit_project},
};
mod firmware;
mod multiply;

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

/// 固定入力が出力へ届くまでの上界を、展開した NOR の依存経路から求める。
fn maximum_depth(netlist: &NorNetlist) -> usize {
    let mut depths = Vec::with_capacity(netlist.gates.len());
    let depth = |wire, depths: &[usize]| match wire {
        Wire::Input(_) | Wire::Constant(_) => 0,
        Wire::Gate(index) => depths[index],
    };
    for gate in &netlist.gates {
        depths.push(depth(gate.left, &depths).max(depth(gate.right, &depths)) + 1);
    }
    netlist
        .outputs
        .iter()
        .map(|&wire| depth(wire, &depths))
        .max()
        .unwrap_or(0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let Some(
        selected @ ("add"
        | "shift-left"
        | "boot-ram-address"
        | "execute-boot"
        | "firmware-rom"
        | "firmware-fetch"
        | "firmware-data-map"
        | "execute-control"
        | "xor"
        | "compare-unsigned"
        | "less-than-unsigned"
        | "execute-load"
        | "execute-logic"
        | "mul-round"
        | "div-round"
        | "div-wide-next"
        | "divide-prepare"
        | "divide-result"
        | "firmware-dispatch"
        | "firmware-dispatch-core"
        | "firmware-dispatch-uart"
        | "selfhost-data-map"
        | "uart-ram-map"),
    ) = args.get(1).map(String::as_str)
    else {
        return Err("検証対象の NCG2 関数名を指定する。対象は README と各設計文書を参照。".into());
    };
    let arithmetic = ["decode.ncg2", "arithmetic.ncg2"];
    let core_profile = selected == "firmware-dispatch-core";
    let uart_profile = selected == "firmware-dispatch-uart";
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
    let memory: &[&str] = match selected {
        "firmware-dispatch-core" => &["firmware-rom.ncg2", "selfhost-memory.ncg2"],
        "firmware-dispatch-uart" => &[
            "selfhost-map.ncg2",
            "uart-ram-map.ncg2",
            "uart-boot-mmio.ncg2",
            "uart-access.ncg2",
        ],
        _ => &["firmware-rom.ncg2", "firmware-memory.ncg2"],
    };
    let dispatch_sources: Vec<_> = [
        "decode.ncg2",
        "arithmetic.ncg2",
        "boot-rom.ncg2",
        "fetch-boot.ncg2",
        "execute-boot.ncg2",
        "execute-control.ncg2",
    ]
    .into_iter()
    .chain(memory.iter().copied())
    .chain([
        "execute-load.ncg2",
        "execute-logic.ncg2",
        "subword-memory.ncg2",
        "execute-subword.ncg2",
        "execute-alu.ncg2",
        "firmware-dispatch.ncg2",
    ])
    .collect();
    let paths = match selected {
        "boot-ram-address" => &boot[..],
        "execute-boot" => &["decode.ncg2", "arithmetic.ncg2", "execute-boot.ncg2"],
        "execute-control" => &[
            "decode.ncg2",
            "arithmetic.ncg2",
            "execute-boot.ncg2",
            "execute-control.ncg2",
        ],
        "execute-logic" => &[
            "decode.ncg2",
            "arithmetic.ncg2",
            "execute-boot.ncg2",
            "execute-logic.ncg2",
        ],
        "mul-round" => &["decode.ncg2", "arithmetic.ncg2", "multiply.ncg2"],
        "div-round" | "div-wide-next" => &["decode.ncg2", "arithmetic.ncg2", "divide.ncg2"],
        "divide-prepare" | "divide-result" => {
            &["decode.ncg2", "arithmetic.ncg2", "divide-operands.ncg2"]
        }
        "firmware-dispatch" | "firmware-dispatch-core" | "firmware-dispatch-uart" => {
            &dispatch_sources
        }
        "execute-load" => &[
            "decode.ncg2",
            "arithmetic.ncg2",
            "boot-rom.ncg2",
            "fetch-boot.ncg2",
            "execute-boot.ncg2",
            "firmware-rom.ncg2",
            "firmware-memory.ncg2",
            "execute-load.ncg2",
        ],
        "firmware-rom" => &["decode.ncg2", "arithmetic.ncg2", "firmware-rom.ncg2"],
        "selfhost-data-map" => &[
            "decode.ncg2",
            "arithmetic.ncg2",
            "execute-boot.ncg2",
            "selfhost-map.ncg2",
        ],
        "uart-ram-map" => &[
            "decode.ncg2",
            "arithmetic.ncg2",
            "execute-boot.ncg2",
            "selfhost-map.ncg2",
            "uart-ram-map.ncg2",
        ],
        "firmware-fetch" | "firmware-data-map" => &[
            "decode.ncg2",
            "arithmetic.ncg2",
            "boot-rom.ncg2",
            "fetch-boot.ncg2",
            "execute-boot.ncg2",
            "firmware-rom.ncg2",
            "firmware-memory.ncg2",
        ],
        _ => &arithmetic[..],
    };
    let sources = paths
        .iter()
        .map(|path| {
            if core_profile && *path == "firmware-rom.ncg2" {
                // 命令判定は ROM の値を入力として受け取る。ここでは配線に必要な幅だけをそろえる。
                Ok(include_str!("../fixtures/selfhost-rom.ncg2").to_owned())
            } else {
                std::fs::read_to_string(format!("circuit/{path}"))
            }
        })
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
    let function = if core_profile || uart_profile {
        "firmware-dispatch"
    } else {
        selected
    };
    let declaration = circuit
        .names
        .iter()
        .find(|n| n.name.word == function)
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
        "add" | "xor" => (128, 64),
        "compare-unsigned" => (128, 2),
        "less-than-unsigned" => (128, 1),
        "shift-left" => (70, 64),
        "boot-ram-address" => (64, 12),
        "execute-boot" => (224, 264),
        "execute-control" => (224, 136),
        "execute-load" => (96, 71),
        "execute-logic" => (160, 67),
        "mul-round" => (386, 257),
        "div-round" => (129, 65),
        "div-wide-next" => (387, 258),
        "divide-prepare" => (130, 128),
        "divide-result" => (258, 128),
        "firmware-dispatch" | "firmware-dispatch-core" | "firmware-dispatch-uart" => (417, 266),
        "firmware-rom" => (5, 64),
        "firmware-fetch" => (64, 33),
        "firmware-data-map" => (64, 28),
        "selfhost-data-map" => (64, 43),
        "uart-ram-map" => (64, 45),
        _ => unreachable!(),
    };
    assert_eq!((*input_count, outputs.len()), expected_shape);
    let depth_limit = match selected {
        "firmware-fetch" => Some(24),
        "execute-boot" => Some(475),
        "execute-control" => Some(493),
        "execute-load" => Some(468),
        "execute-logic" => Some(31),
        "mul-round" => Some(457),
        "div-round" => Some(462),
        "div-wide-next" => Some(480),
        "divide-prepare" => Some(457),
        "divide-result" => Some(469),
        "firmware-dispatch" | "firmware-dispatch-core" | "firmware-dispatch-uart" => Some(505),
        "uart-ram-map" => Some(31),
        _ => None,
    };
    if let Some(limit) = depth_limit {
        let depth = maximum_depth(built.netlist());
        eprintln!("{selected}: {} NORs, depth {depth}", gates.len());
        assert!(depth <= limit);
    }
    if selected == "firmware-data-map" {
        let bank_form = circuit
            .names
            .iter()
            .find(|n| n.name.word == "firmware-unmatched-banks")
            .ok_or("バンク判定の宣言が見つからない。")?
            .form;
        let bank = circuit
            .syntax
            .entries
            .iter()
            .position(|e| e.form == bank_form)
            .ok_or("バンク判定の索引が見つからない。")?;
        let built = aliases
            .expand_reachable_function((file, bank), 65536, 1_048_576, 1_048_576)
            .map_err(|error| format!("{error:?}"))?
            .ok_or("バンク判定の展開結果がない。")?;
        firmware::verify_bank_delay(built.netlist());
    }

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
    if matches!(selected, "firmware-rom" | "firmware-fetch") {
        let path = args
            .get(2)
            .ok_or("ELF から変換したバイナリイメージを指定する。")?;
        firmware::emit_rom(selected, outputs, &std::fs::read(path)?)?;
    } else if selected == "firmware-data-map" {
        firmware::emit_data_map(outputs);
    } else if selected == "selfhost-data-map" {
        firmware::emit_selfhost_data_map(outputs);
    } else if selected == "uart-ram-map" {
        firmware::emit_uart_ram_map(outputs);
    } else if matches!(selected, "divide-prepare" | "divide-result") {
        let prepare = selected == "divide-prepare";
        for (port, start) in [("dividend", 0), ("divisor", 64)] {
            let value = vector((start..start + 64).map(|index| format!("i{index}")));
            println!("(define-fun {port} () (_ BitVec 64) {value})");
        }
        if !prepare {
            for (port, start) in [("quotient-magnitude", 128), ("remainder-magnitude", 192)] {
                let value = vector((start..start + 64).map(|index| format!("i{index}")));
                println!("(define-fun {port} () (_ BitVec 64) {value})");
            }
        }
        println!("(define-fun signed-mode () Bool i{})", input_count - 2);
        println!("(define-fun word-mode () Bool i{})", input_count - 1);
        let names = if prepare {
            ["prepared-dividend", "prepared-divisor"]
        } else {
            ["quotient", "remainder"]
        };
        for (port, bits) in names.into_iter().zip(outputs.chunks_exact(64)) {
            let value = vector(bits.iter().copied().map(name));
            println!("(define-fun {port} () (_ BitVec 64) {value})");
        }
        let specification = if prepare {
            include_str!("divide_prepare.smt2")
        } else {
            include_str!("divide_result.smt2")
        };
        print!("{specification}");
    } else if selected == "div-round" {
        let remainder = vector((0..64).map(|index| format!("i{index}")));
        let divisor = vector((64..128).map(|index| format!("i{index}")));
        let next_remainder = vector(outputs[..64].iter().copied().map(name));
        println!("(define-fun remainder () (_ BitVec 64) {remainder})");
        println!("(define-fun divisor () (_ BitVec 64) {divisor})");
        println!("(define-fun next-bit () (_ BitVec 1) (ite i128 #b1 #b0))");
        println!("(define-fun next-remainder () (_ BitVec 64) {next_remainder})");
        println!("(define-fun quotient-bit () Bool {})", name(outputs[64]));
        print!("{}", include_str!("div_round.smt2"));
    } else if selected == "div-wide-next" {
        for (port, start) in [
            ("dividend", 0), ("input-divisor", 64), ("quotient", 129), ("remainder", 193),
            ("divisor", 257), ("mark", 321),
        ] {
            let word = vector((start..start + 64).map(|index| format!("i{index}")));
            println!("(define-fun {port} () (_ BitVec 64) {word})");
        }
        for (port, index) in [("initialize", 128), ("running", 385), ("done", 386)] {
            println!("(define-fun {port} () Bool i{index})");
        }
        for (port, start) in [
            ("next-quotient", 0), ("next-remainder", 64), ("next-divisor", 128), ("next-mark", 192),
        ] {
            let word = vector(outputs[start..start + 64].iter().copied().map(name));
            println!("(define-fun {port} () (_ BitVec 64) {word})");
        }
        println!("(define-fun next-running () Bool {})", name(outputs[256]));
        println!("(define-fun next-done () Bool {})", name(outputs[257]));
        print!("{}", include_str!("div_wide_next.smt2"));
    } else if selected == "mul-round" {
        multiply::emit(outputs);
    } else if matches!(
        selected,
        "firmware-dispatch" | "firmware-dispatch-core" | "firmware-dispatch-uart"
    ) {
        firmware::emit_dispatch(outputs, core_profile, uart_profile);
    } else if matches!(selected, "execute-load" | "execute-logic") {
        let instruction = vector((0..32).map(|i| format!("i{i}")));
        let rs1 = vector((32..96).map(|i| format!("i{i}")));
        println!("(define-fun instruction () (_ BitVec 32) {instruction})");
        println!("(define-fun rs1 () (_ BitVec 64) {rs1})");
        if selected == "execute-load" {
            let address = vector(outputs[2..66].iter().copied().map(name));
            let rd = vector(outputs[66..71].iter().copied().map(name));
            println!("(define-fun is-ld () Bool {})", name(outputs[0]));
            println!("(define-fun valid () Bool {})", name(outputs[1]));
            println!("(define-fun address () (_ BitVec 64) {address})");
            println!("(define-fun register-index () (_ BitVec 5) {rd})");
            print!("{}", include_str!("execute_load.smt2"));
        } else {
            let rs2 = vector((96..160).map(|i| format!("i{i}")));
            let result = vector(outputs[2..66].iter().copied().map(name));
            println!("(define-fun rs2 () (_ BitVec 64) {rs2})");
            println!("(define-fun recognized () Bool {})", name(outputs[0]));
            println!("(define-fun multiply () Bool {})", name(outputs[1]));
            println!("(define-fun high-product () Bool {})", name(outputs[66]));
            println!("(define-fun result () (_ BitVec 64) {result})");
            print!("{}", include_str!("execute_logic.smt2"));
        }
    } else if matches!(selected, "execute-boot" | "execute-control") {
        for (port, start, stop) in [
            ("instruction", 0, 32),
            ("pc", 32, 96),
            ("rs1", 96, 160),
            ("rs2", 160, 224),
        ] {
            let bits = vector((start..stop).map(|i| format!("i{i}")));
            println!("(define-fun {port} () (_ BitVec {}) {bits})", stop - start);
        }
        let flags: &[(&str, usize)] = if selected == "execute-boot" {
            &[("valid", 0), ("register-write", 134), ("store-write", 263)]
        } else {
            &[("recognized", 0), ("valid", 1), ("register-write", 135)]
        };
        for &(port, bit) in flags {
            println!("(define-fun {port} () Bool {})", name(outputs[bit]));
        }
        let words: &[(&str, usize, usize)] = if selected == "execute-boot" {
            &[
                ("next-pc", 1, 65),
                ("register-index", 65, 70),
                ("register-data", 70, 134),
                ("store-address", 135, 199),
                ("store-data", 199, 263),
            ]
        } else {
            &[
                ("next-pc", 2, 66),
                ("register-index", 66, 71),
                ("register-data", 71, 135),
            ]
        };
        for &(port, start, stop) in words {
            let bits = vector(outputs[start..stop].iter().copied().map(name));
            println!("(define-fun {port} () (_ BitVec {}) {bits})", stop - start);
        }
        let specification = if selected == "execute-boot" {
            include_str!("execute_boot.smt2")
        } else {
            include_str!("execute_control.smt2")
        };
        print!("{specification}");
    } else if selected == "boot-ram-address" {
        let first = vector((0..64).map(|i| format!("i{i}")));
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
    } else if matches!(selected, "compare-unsigned" | "less-than-unsigned") {
        let left = vector((0..64).map(|i| format!("i{i}")));
        let right = vector((64..128).map(|i| format!("i{i}")));
        let less = name(outputs[outputs.len() - 1]);
        if selected == "compare-unsigned" {
            let equal = name(outputs[0]);
            println!(
                "(assert (or (distinct {equal} (= {left} {right})) (distinct {less} (bvult {left} {right}))))"
            );
        } else {
            println!("(assert (distinct {less} (bvult {left} {right})))");
        }
    } else {
        let first = vector((0..64).map(|i| format!("i{i}")));
        let rest = vector((64..*input_count).map(|i| format!("i{i}")));
        let output = vector(outputs.iter().copied().map(name));
        let expected = match selected {
            "add" => format!("(bvadd {first} {rest})"),
            "xor" => format!("(bvxor {first} {rest})"),
            "shift-left" => format!("(bvshl {first} ((_ zero_extend 58) {rest}))"),
            _ => unreachable!(),
        };
        println!("(assert (distinct {output} {expected}))");
    }
    println!("(check-sat)");
    Ok(())
}
