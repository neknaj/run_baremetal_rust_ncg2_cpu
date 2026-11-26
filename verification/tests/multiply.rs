//! 入力ラッチの操作から MUL の完了と積の観測までを、実際の VM で検査する。

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
    ("multiply.ncg2", include_str!("../../circuit/multiply.ncg2")),
];

#[test]
fn completes_on_the_last_capture_and_accepts_a_new_pair() {
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
    let form = circuit
        .names
        .iter()
        .find(|name| name.name.word == "mul-low")
        .unwrap()
        .form;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == form)
        .unwrap();
    let mut machine = aliases
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
        .unwrap();
    assert_eq!(machine.vm.reset(), [false; 65]);

    // Rust 起動イメージの MUL 命令に渡る値を使い、ワード境界をまたぐ桁も検査する。
    let left: u64 = 0x0000_0001_0000_0001;
    let right: u64 = 0x0000_0002_0000_0003;
    let mut inputs = [false; 130];
    for index in 0..64 {
        inputs[index] = (left >> index) & 1 != 0;
        inputs[64 + index] = (right >> index) & 1 != 0;
    }
    // 各更新の直前までラッチを保持する。完了ビットは捕捉する tick で変化する。
    let mut previous_done = false;
    for (initialize, expected_done) in [(true, false)]
        .into_iter()
        .chain((1..=64).map(|round| (false, round == 64)))
        .chain([(true, false)])
    {
        inputs[128] = initialize;
        inputs[129] = false;
        for _ in 0..457 {
            assert_eq!(machine.vm.step(&inputs).unwrap()[64], previous_done);
        }
        inputs[129] = true;
        let output = machine.vm.step(&inputs).unwrap();
        assert_eq!(output[64], expected_done);
        let product = output[..64]
            .iter()
            .enumerate()
            .map(|(index, &bit)| u64::from(bit) << index)
            .sum::<u64>();
        if initialize {
            assert_eq!(product, 0);
        } else if expected_done {
            assert_eq!(product, 0x0000_0005_0000_0003);
        }
        previous_done = expected_done;
    }
}
