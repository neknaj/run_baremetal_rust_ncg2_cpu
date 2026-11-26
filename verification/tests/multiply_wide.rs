//! 桁上がりを含む積と完了の境界を、実際の順序回路 VM で検査する。

use circuitgame2_core::project::{
    ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, check_use_aliases,
    index_use_targets, parse_circuit_project,
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

fn word(bits: &[bool]) -> u64 {
    bits.iter()
        .enumerate()
        .map(|(index, &bit)| u64::from(bit) << index)
        .sum()
}

#[test]
fn captures_both_halves_and_completes_on_the_last_round() {
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
    let declaration = circuit
        .names
        .iter()
        .find(|name| name.name.word == "mul-wide")
        .unwrap();
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == declaration.form)
        .unwrap();
    let mut vm = aliases
        .lower_clocked_vm(
            ModuleBodyPosition { file, entry },
            ClockedModuleVmLimits {
                width: 65_536,
                wiring_bits: 1_048_576,
                gates: 1_048_576,
                state_bits: 1_048_576,
            },
        )
        .unwrap()
        .unwrap()
        .vm;
    assert_eq!(vm.reset(), [false; 129]);

    // 最大値の積では上位の桁上がりを、実 ELF の32×32では再初期化後の結果を調べる。
    let mut previous_done = false;
    for (left, right) in [(u64::MAX, u64::MAX), (32, 32)] {
        let expected = u128::from(left) * u128::from(right);
        let mut inputs = [false; 130];
        for index in 0..64 {
            inputs[index] = (left >> index) & 1 != 0;
            inputs[64 + index] = (right >> index) & 1 != 0;
        }
        for round in 0..=64 {
            inputs[128] = round == 0;
            inputs[129] = false;
            for _ in 0..460 {
                assert_eq!(vm.step(&inputs).unwrap()[128], previous_done);
            }
            inputs[129] = true;
            let output = vm.step(&inputs).unwrap();
            previous_done = round == 64;
            assert_eq!(output[128], previous_done, "更新 {round} の完了状態");
            if round == 0 {
                assert_eq!(word(&output[..64]), right);
                assert_eq!(word(&output[64..128]), 0);
            } else if previous_done {
                assert_eq!(word(&output[..64]), expected as u64);
                assert_eq!(word(&output[64..128]), (expected >> 64) as u64);
            }
        }
        // capture が0なら、確定した積と完了ビットを保持する。
        inputs[129] = false;
        let output = vm.step(&inputs).unwrap();
        assert_eq!(word(&output[..64]), expected as u64);
        assert_eq!(word(&output[64..128]), (expected >> 64) as u64);
        assert!(output[128]);
    }
}
