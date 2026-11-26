//! 反復除算の開始、完了、結果の保持を順序回路 VM で調べる。

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
    ("divide.ncg2", include_str!("../../circuit/divide.ncg2")),
];

fn word(bits: &[bool]) -> u64 {
    bits.iter()
        .enumerate()
        .map(|(index, &bit)| u64::from(bit) << index)
        .sum()
}

#[test]
fn divides_on_the_last_capture_and_holds_until_reinitialized() {
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
        .find(|name| name.name.word == "div-wide")
        .unwrap();
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == declaration.form)
        .unwrap();
    let limits = ClockedModuleVmLimits {
        width: 65_536,
        wiring_bits: 1_048_576,
        gates: 1_048_576,
        state_bits: 1_048_576,
    };
    let mut vm = aliases
        .lower_clocked_vm(ModuleBodyPosition { file, entry }, limits)
        .unwrap()
        .unwrap()
        .vm;
    assert_eq!(vm.reset(), [false; 129]);
    let mut input = [false; 130];
    let mut was_done = false;
    for _ in 0..480 {
        vm.step(&input).unwrap();
    }
    input[129] = true;
    assert_eq!(vm.step(&input).unwrap(), [false; 129]);

    for (dividend, divisor) in [(u64::MAX, 3), (0x8000000000000001, 0), (17, u64::MAX)] {
        let expected = (
            dividend.checked_div(divisor).unwrap_or(u64::MAX),
            dividend.checked_rem(divisor).unwrap_or(dividend),
        );
        for index in 0..64 {
            input[index] = dividend >> index & 1 != 0;
            input[64 + index] = divisor >> index & 1 != 0;
        }
        for captures in 0..=64 {
            input[128] = captures == 0;
            input[129] = false;
            for _ in 0..480 {
                assert_eq!(vm.step(&input).unwrap()[128], was_done);
            }
            input[129] = true;
            let output = vm.step(&input).unwrap();
            was_done = captures == 64;
            assert_eq!(output[128], was_done);
            if captures == 0 {
                assert_eq!(word(&output[..64]), dividend);
                assert_eq!(word(&output[64..128]), 0);
                // 初期化以降に外部オペランドを変えても、捕捉済みの値を使う。
                input[..128].fill(false);
            } else if was_done {
                assert_eq!((word(&output[..64]), word(&output[64..128])), expected);
            }
        }
        input[129] = false;
        for _ in 0..480 {
            vm.step(&input).unwrap();
        }
        input[129] = true;
        let output = vm.step(&input).unwrap();
        assert_eq!((word(&output[..64]), word(&output[64..128])), expected);
        assert!(output[128]);
    }
}
