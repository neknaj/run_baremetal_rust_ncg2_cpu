//! 起動 ROM の命令列を Core の順序回路 VM で実行する。

use circuitgame2_core::project::{
    ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, VramDeclaration, check_use_aliases,
    index_use_targets, parse_circuit_project,
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
        "program-counter.ncg2",
        include_str!("../../circuit/program-counter.ncg2"),
    ),
    (
        "register-file.ncg2",
        include_str!("../../circuit/register-file.ncg2"),
    ),
    (
        "execute-boot.ncg2",
        include_str!("../../circuit/execute-boot.ncg2"),
    ),
    (
        "boot-machine.ncg2",
        include_str!("../../circuit/boot-machine.ncg2"),
    ),
];

fn value(bits: &[bool]) -> u64 {
    bits.iter()
        .enumerate()
        .map(|(offset, &bit)| u64::from(bit) << offset)
        .sum()
}

#[test]
fn retires_rom_and_stores_the_result() {
    let snapshots: Vec<_> = SOURCES
        .iter()
        .map(|&(path, text)| SourceSnapshot {
            path,
            revision: 0,
            text,
        })
        .collect();
    let project = parse_circuit_project(&snapshots).unwrap();
    let targets = index_use_targets(&project).unwrap();
    let aliases = check_use_aliases(&targets).unwrap();
    let file = SOURCES.len() - 1;
    let circuit = &project.circuits()[file];
    let entry_form = circuit
        .names
        .iter()
        .find(|name| name.name.word == "boot-machine")
        .unwrap()
        .form;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == entry_form)
        .unwrap();
    let mut machine = aliases
        .lower_clocked_vram(
            ModuleBodyPosition { file, entry },
            ClockedModuleVmLimits {
                width: 65_536,
                wiring_bits: 1_048_576,
                gates: 1_048_576,
                state_bits: 1_048_576,
            },
            &[VramDeclaration {
                name: "boot-ram",
                read_output: "probe-data",
            }],
        )
        .unwrap()
        .unwrap();
    assert_eq!(machine.reset(), [false; 141]);

    // RAM の最後の語は物理アドレス 0x2fff8 に対応する。
    let result_address = [true; 11];
    let expected = [
        (1031, 0),
        (2061, 4),
        (3091, 8),
        (4121, 12),
        (5151, 16),
        (6181, 20),
    ];
    let mut commits = Vec::new();
    let result_word = 0x0000_0001_0000_0003;
    let mut previous_result = 0;
    let mut previous_pc = 0;
    for tick in 1..=6182 {
        let output = machine.step(&result_address).unwrap();
        let pc = value(&output[..64]);
        assert!(!output[140], "停止信号は tick {tick} で0である。");
        if output[139] {
            assert_eq!(pc, previous_pc);
            commits.push((tick, pc));
        }
        previous_pc = pc;
        let response = value(&output[128..139]);
        if response == 2047 {
            assert_eq!(value(&output[64..128]), previous_result);
        }
        let memory = machine.read_vram("boot-ram").unwrap();
        let (other_words, result) = memory.bits.split_at(2047 * 64);
        assert!(other_words.iter().all(|&bit| !bit));
        let expected_result = if tick <= 5151 { 0 } else { result_word };
        assert_eq!(
            value(result),
            expected_result,
            "RAM を更新した tick を検査する。"
        );
        previous_result = expected_result;
    }
    assert_eq!(commits, expected);
    assert_eq!(previous_pc, 20);

    // JAL は PC を20に保つ。アドレスを切り替えても RAM の値は保持される。
    for _ in 0..16 {
        let output = machine.step(&[false; 11]).unwrap();
        assert_eq!(value(&output[..64]), 20);
    }
    let output = machine.step(&[false; 11]).unwrap();
    assert_eq!(value(&output[128..139]), 0);
    assert_eq!(value(&output[64..128]), 0);
    for _ in 0..16 {
        let output = machine.step(&result_address).unwrap();
        assert_eq!(value(&output[..64]), 20);
    }
    let output = machine.step(&result_address).unwrap();
    assert_eq!(value(&output[128..139]), 2047);
    assert_eq!(value(&output[64..128]), result_word);
}
