//! Core VM で連続した書き込みと、安定した読み出しの契約を検査する。

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
        "execute-boot.ncg2",
        include_str!("../../circuit/execute-boot.ncg2"),
    ),
    (
        "firmware-rom.ncg2",
        include_str!("../fixtures/selfhost-rom.ncg2"),
    ),
    (
        "selfhost-memory.ncg2",
        include_str!("../../circuit/selfhost-memory.ncg2"),
    ),
];

/// 期待値側では半開区間の基点を引いて、独立に RAM の語番号を計算する。
fn ram(address: u64) -> Option<(usize, usize)> {
    if address % 8 != 0 {
        return None;
    }
    [(0x16000..0x19000), (0x2bf00..0x2c000), (0x2c000..0x30000)]
        .into_iter()
        .enumerate()
        .find_map(|(bank, range)| {
            range
                .contains(&address)
                .then(|| (bank, ((address - range.start) / 8) as usize))
        })
}

fn rom(address: u64) -> Option<u64> {
    (address % 8 == 0 && (0x10000..0x16000).contains(&address)).then(|| {
        let word = (address - 0x10000) / 8;
        ((0x5678_a000 | word) << 32) | (0x1234_5000 | word)
    })
}

fn frame(address: u64, value: Option<u64>) -> [bool; 129] {
    let mut input = [false; 129];
    for bit in 0..64 {
        input[bit] = ((address >> bit) & 1) != 0;
        input[64 + bit] = ((value.unwrap_or_default() >> bit) & 1) != 0;
    }
    input[128] = value.is_some();
    input
}

fn word(wires: &[bool]) -> u64 {
    wires
        .iter()
        .enumerate()
        .map(|(bit, &value)| u64::from(value) << bit)
        .sum()
}

#[test]
fn commits_every_frame_to_the_selected_bank_and_reads_after_settling() {
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
        .find(|name| name.name.word == "firmware-data-memory")
        .unwrap()
        .form;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == form)
        .unwrap();
    let banks = ["bss", "mailbox", "stack"];
    let mut vm = aliases
        .lower_clocked_vram(
            ModuleBodyPosition { file, entry },
            ClockedModuleVmLimits {
                width: 65_536,
                wiring_bits: 1_048_576,
                gates: 1_048_576,
                state_bits: 1_048_576,
            },
            &[
                VramDeclaration {
                    name: banks[0],
                    read_output: "bss-data",
                },
                VramDeclaration {
                    name: banks[1],
                    read_output: "mailbox-data",
                },
                VramDeclaration {
                    name: banks[2],
                    read_output: "stack-data",
                },
            ],
        )
        .unwrap()
        .unwrap();

    vm.reset();
    let sequence = [
        (0x16000, Some(0x0123_4567_89ab_cdef)),
        (0x2bf00, Some(0x80f1_e2d3_c4b5_a697)),
        (0x2c000, Some(0xffff_0000_ffff_0000)),
        (0x15ff8, Some(4)),
        (0x17ff8, Some(5)),
        (0x18000, Some(6)),
        (0x18ff8, Some(7)),
        (0x19000, Some(8)),
        (0x2bff8, Some(9)),
        (0x2c000, Some(10)),
        (0x2c000, Some(11)),
        (0x2c001, Some(12)),
        (0x2bff8, None),
        (0x2fff8, Some(13)),
        (0x30000, Some(14)),
        (0x10000, Some(15)),
        (0x16000 | (1u64 << 63), Some(16)),
        (0x16000, Some(17)),
    ];
    let mut expected = [vec![0; 2048], vec![0; 32], vec![0; 2048]];
    // リセット直後から連続して入力を変え、t+17 のフレームごとに全記憶語を見る。
    for step in 0..sequence.len() + 17 {
        let (address, value) = sequence.get(step).copied().unwrap_or_default();
        vm.step(&frame(address, value)).unwrap();
        if let Some(&(previous_address, Some(previous_value))) =
            step.checked_sub(17).and_then(|t| sequence.get(t))
        {
            if let Some((bank, index)) = ram(previous_address) {
                expected[bank][index] = previous_value;
            }
        }
        for (name, expected) in banks.into_iter().zip(&expected) {
            let values = vm.read_vram(name).unwrap().bits;
            assert_eq!(values.len(), expected.len() * 64);
            for (index, (actual, &expected)) in values.chunks_exact(64).zip(expected).enumerate() {
                assert_eq!(
                    word(actual),
                    expected,
                    "{name} の語 {index}、tick {}",
                    step + 1
                );
            }
        }
    }

    // 各区間の端を読む。アドレスを固定して30 tick 待ってから RAM の値を使う。
    for address in [
        0xfff8, 0x10000, 0x13ff8, 0x14000, 0x15ff8, 0x15fff, 0x16000, 0x17ff8, 0x18000, 0x18ff8,
        0x19000, 0x2bef8, 0x2bf00, 0x2bff8, 0x2c000, 0x2fff8, 0x30000,
    ] {
        let input = frame(address, None);
        for _ in 0..29 {
            vm.step(&input).unwrap();
        }
        let output = vm.step(&input).unwrap();
        let expected_ram = ram(address).map(|(bank, index)| expected[bank][index]);
        let value = expected_ram.or_else(|| rom(address));
        assert_eq!(
            (output[64], output[65]),
            (value.is_some(), expected_ram.is_some()),
            "{address:#x}"
        );
        if let Some(value) = value {
            assert_eq!(word(&output[..64]), value, "{address:#x}");
        }
    }

    // 書き込み要求の30 tick 後には、同じアドレスから新しい値を読める。
    let replacement = 0xfedc_ba98_7654_3210;
    vm.step(&frame(0x2bf00, Some(replacement))).unwrap();
    let mailbox_input = frame(0x2bf00, None);
    for _ in 0..29 {
        vm.step(&mailbox_input).unwrap();
    }
    let output = vm.step(&mailbox_input).unwrap();
    assert_eq!(word(&output[..64]), replacement);

    // 確定前の書き込みをリセットで破棄し、RAM も初期値に戻す。
    vm.step(&frame(0x2bf00, Some(u64::MAX))).unwrap();
    vm.reset();
    for _ in 0..18 {
        vm.step(&frame(0x2bf00, None)).unwrap();
    }
    for name in banks {
        assert!(vm.read_vram(name).unwrap().bits.iter().all(|&bit| !bit));
    }
}
