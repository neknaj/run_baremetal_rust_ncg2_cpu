//! 実行用 VM で起動 ELF の ROM と2つの RAM の接続を検査する。

use circuitgame2_core::project::{
    ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, VramDeclaration, check_use_aliases,
    index_use_targets, parse_circuit_project,
};

const SOURCES: &[(&str, &str)] = &[
    ("decode.ncg2", include_str!("../../circuit/decode.ncg2")),
    (
        "arithmetic.ncg2",
        include_str!("../../circuit/arithmetic.ncg2"),
    ),
    ("boot-rom.ncg2", include_str!("../../circuit/boot-rom.ncg2")),
    (
        "fetch-boot.ncg2",
        include_str!("../../circuit/fetch-boot.ncg2"),
    ),
    (
        "execute-boot.ncg2",
        include_str!("../../circuit/execute-boot.ncg2"),
    ),
    (
        "firmware-rom.ncg2",
        include_str!("../../circuit/firmware-rom.ncg2"),
    ),
    (
        "firmware-memory.ncg2",
        include_str!("../../circuit/firmware-memory.ncg2"),
    ),
];

#[test]
fn load_store_and_reset_use_independent_memory_banks() {
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
        .find(|name| name.name.word == "firmware-data-memory")
        .unwrap()
        .form;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == entry_form)
        .unwrap();
    let mut memory = aliases
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
                    name: "bss",
                    read_output: "bss-data",
                },
                VramDeclaration {
                    name: "stack",
                    read_output: "stack-data",
                },
            ],
        )
        .unwrap()
        .unwrap();

    let bss_address = 0x11000;
    let stack_address = 0x2c000;
    let rodata_address = 0x100d8;
    let rodata_word = 0x0000_0001_0000_0001;
    let bss_value = 0x0123_4567_89ab_cdef;
    let stack_value = 0xfedc_ba98_7654_3210;

    // リセット直後から毎 tick バンクを切り替えて要求し、確定した記憶部を直接読む。
    memory.reset();
    let sequence = [
        (bss_address, 1),
        (0x8, 2),
        (stack_address, 3),
        (bss_address + 1, 4),
        (bss_address + 8, 5),
        (rodata_address, 6),
        (stack_address + 8, 7),
        (stack_address | (1 << 63), 8),
        (bss_address, 9),
    ];
    let mut expected = [0; 4];
    for tick in 1..=sequence.len() + 15 {
        let input = sequence.get(tick - 1).map_or_else(
            || frame(0, None),
            |&(address, data)| frame(address, Some(data)),
        );
        memory.step(&input).unwrap();
        match tick {
            16 => expected[0] = 1,
            18 => expected[2] = 3,
            20 => expected[1] = 5,
            22 => expected[3] = 7,
            24 => expected[0] = 9,
            _ => {}
        }
        let bss = memory.read_vram("bss").unwrap().bits;
        let stack = memory.read_vram("stack").unwrap().bits;
        assert_eq!(
            [
                word(&bss[..64]),
                word(&bss[64..128]),
                word(&stack[..64]),
                word(&stack[64..128])
            ],
            expected
        );
        assert!(bss[128..].iter().chain(&stack[128..]).all(|&bit| !bit));
    }

    // 同じ添字の BSS とスタックを使い、2つの RAM が独立していることを確かめる。
    memory.reset();
    {
        let mut access = |address, value: Option<u64>| {
            let read = frame(address, None);
            for _ in 0..32 {
                memory.step(&read).unwrap();
            }
            if value.is_some() {
                memory.step(&frame(address, value)).unwrap();
            }
            for _ in 0..31 {
                memory.step(&read).unwrap();
            }
            let output = memory.step(&read).unwrap();
            (word(&output[..64]), output[64], output[65])
        };
        assert_eq!(access(bss_address, None), (0, true, true));
        assert_eq!(access(stack_address, None), (0, true, true));
        assert_eq!(
            access(bss_address, Some(bss_value)),
            (bss_value, true, true)
        );
        assert_eq!(
            access(stack_address, Some(stack_value)),
            (stack_value, true, true)
        );
        assert_eq!(
            access(rodata_address, Some(u64::MAX)),
            (rodata_word, true, false)
        );
        assert_eq!(access(0x12000, Some(u64::MAX)).1, false);
        assert_eq!(access(bss_address + 1, Some(u64::MAX)).2, false);
        assert_eq!(access(bss_address, None), (bss_value, true, true));
        assert_eq!(access(bss_address + 8, None), (0, true, true));
        assert_eq!(access(stack_address, None), (stack_value, true, true));
        assert_eq!(access(stack_address + 8, None), (0, true, true));
    }
    memory.reset();
    let input = frame(bss_address, None);
    for _ in 0..31 {
        memory.step(&input).unwrap();
    }
    assert_eq!(memory.step(&input).unwrap()[..64], [false; 64]);
}

fn frame(address: u64, value: Option<u64>) -> [bool; 129] {
    let mut input = [false; 129];
    for bit in 0..64 {
        input[bit] = ((address >> bit) & 1) != 0;
        input[64 + bit] = ((value.unwrap_or(0) >> bit) & 1) != 0;
    }
    input[128] = value.is_some();
    input
}

fn word(bits: &[bool]) -> u64 {
    bits.iter()
        .enumerate()
        .map(|(bit, &value)| u64::from(value) << bit)
        .sum()
}
