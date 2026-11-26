//! 反復除算の開始、完了、結果の保持を順序回路 VM で調べる。

use circuitgame2_core::{
    netlist::ClockedVm,
    project::{
        ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, check_use_aliases,
        index_use_targets, parse_circuit_project,
    },
};

const SOURCES: &[(&str, &str)] = &[
    ("decode.ncg2", include_str!("../../circuit/decode.ncg2")),
    (
        "arithmetic.ncg2",
        include_str!("../../circuit/arithmetic.ncg2"),
    ),
    ("divide.ncg2", include_str!("../../circuit/divide.ncg2")),
    (
        "divide-operands.ncg2",
        include_str!("../../circuit/divide-operands.ncg2"),
    ),
    (
        "execute-boot.ncg2",
        include_str!("../../circuit/execute-boot.ncg2"),
    ),
    (
        "boot-machine.ncg2",
        include_str!("../../circuit/boot-machine.ncg2"),
    ),
    (
        "firmware-divide.ncg2",
        include_str!("../../circuit/firmware-divide.ncg2"),
    ),
];

fn word(bits: &[bool]) -> u64 {
    bits.iter()
        .enumerate()
        .map(|(index, &bit)| u64::from(bit) << index)
        .sum()
}

fn divider(path: &str, entry_name: &str) -> ClockedVm {
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
    let file = SOURCES.iter().position(|&(name, _)| name == path).unwrap();
    let circuit = &project.circuits()[file];
    let declaration = circuit
        .names
        .iter()
        .find(|name| name.name.word == entry_name)
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
    aliases
        .lower_clocked_vm(ModuleBodyPosition { file, entry }, limits)
        .unwrap()
        .unwrap()
        .vm
}

#[test]
fn divides_on_the_last_capture_and_holds_until_reinitialized() {
    let mut vm = divider("divide.ncg2", "div-wide");
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

#[test]
fn firmware_divider_captures_modes_and_finishes_with_the_corrected_result() {
    let mut vm = divider("firmware-divide.ncg2", "firmware-divide");
    let mut input = [false; 132];
    vm.reset();
    let cases = [
        ((-11i64) as u64, 3, true, false, false, (-3i64) as u64),
        ((-11i64) as u64, 3, true, false, true, (-2i64) as u64),
        (u64::MAX, 3, false, false, false, u64::MAX / 3),
        (u64::MAX - 1, 3, false, false, true, 2),
        (
            0x1234_5678_8000_0000,
            u64::MAX,
            true,
            true,
            false,
            0xffff_ffff_8000_0000,
        ),
        (0x1234_5678_ffff_fff5, 3, true, true, true, (-2i64) as u64),
        (0x1234_5678_ffff_ffff, 1, false, true, false, u64::MAX),
        (
            0x1234_5678_8000_0000,
            0x8765_4321_0000_0000,
            false,
            true,
            true,
            0xffff_ffff_8000_0000,
        ),
    ];
    for (index, (left, right, signed, word_mode, remainder, expected)) in
        cases.into_iter().enumerate()
    {
        input[0] = true;
        for index in 0..64 {
            input[1 + index] = left >> index & 1 != 0;
            input[65 + index] = right >> index & 1 != 0;
        }
        input[129..].copy_from_slice(&[signed, word_mode, remainder]);
        assert!(!vm.step(&input).unwrap()[64]);
        // 開始後は外部オペランドとモードを変更しても捕捉済みの値で演算する。
        input.fill(false);
        input[129..].copy_from_slice(&[!signed, !word_mode, !remainder]);
        for elapsed in 1..=34_184 {
            let output = vm.step(&input).unwrap();
            assert_eq!(output[64], elapsed == 34_184, "開始から {elapsed} tick");
            if elapsed == 34_184 {
                assert_eq!(word(&output[..64]), expected);
            }
        }
        // 最初の完了の次の tick には、待機なしで次の要求を渡す。
        for _ in 0..if index == 0 { 0 } else { 8 } {
            let output = vm.step(&input).unwrap();
            assert!(!output[64]);
            assert_eq!(word(&output[..64]), expected);
        }
    }
}
