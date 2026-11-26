//! 起動 ELF をフェッチし、Core の VM 上で命令とメモリアクセスを照合する。

use circuitgame2_core::project::{
    ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, check_use_aliases,
    index_use_targets, parse_circuit_project,
};

#[test]
fn runs_the_rust_elf_to_its_halt_loop() {
    let sources = [
        "decode",
        "boot-rom",
        "fetch-boot",
        "arithmetic",
        "program-counter",
        "register-file",
        "execute-boot",
        "boot-machine",
        "execute-control",
        "firmware-rom",
        "firmware-memory",
        "execute-load",
        "execute-logic",
        "multiply",
        "firmware-machine",
    ];
    let sources: Vec<_> = sources
        .iter()
        .map(|name| {
            let path = format!("../circuit/{name}.ncg2");
            let text = std::fs::read_to_string(&path).unwrap();
            (format!("{name}.ncg2"), text)
        })
        .collect();
    let snapshots: Vec<_> = sources
        .iter()
        .map(|(path, text)| SourceSnapshot {
            path,
            text,
            revision: 0,
        })
        .collect();
    let project = parse_circuit_project(&snapshots).unwrap();
    let targets = index_use_targets(&project).unwrap();
    let aliases = check_use_aliases(&targets).unwrap();
    let file = snapshots.len() - 1;
    let circuit = &project.circuits()[file];
    let form = circuit
        .names
        .iter()
        .find(|name| name.name.word == "firmware-machine")
        .unwrap()
        .form;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == form)
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
    assert_eq!(vm.reset(), [false; 260]);

    // BSS 走査を1周し、Rust の結果を書き出し、終了後の自己ループへ戻る。
    let expected = [
        0x00, 0x04, 0x08, 0x0c, 0x10, 0x14, 0x18, 0x1c, 0x20, 0x24, 0x28, 0x2c, 0x20, 0x30, 0x34,
        0x38, 0x3c, 0x40, 0x44, 0x48, 0x4c, 0x50, 0x54, 0x58, 0x5c, 0x68, 0x6c, 0x70, 0x74, 0x78,
        0x7c, 0x80, 0x84, 0x88, 0x8c, 0x90, 0xa4, 0xa8, 0xac, 0xb0, 0xb4, 0xb8, 0xbc, 0xc0, 0xc4,
        0xc8, 0xcc, 0x60, 0x64, 0x64,
    ];
    let mut commits = Vec::new();
    let mut loads = Vec::new();
    let mut stores = Vec::new();
    let product_address = bits(0x2fff8);
    let mut previous_commit = 0;
    for tick in 1..=100_000 {
        let output = vm.step(&product_address).unwrap();
        let pc = word(&output[..64]);
        assert!(
            !output[259],
            "停止診断が出た。tick {tick}, PC offset {pc:#x}"
        );
        if output[258] {
            let address = word(&output[128..192]);
            let register_data = word(&output[192..256]);
            if output[257] {
                stores.push((pc, address));
            }
            if matches!(pc, 0x58 | 0x70 | 0x74 | 0x84 | 0x88) {
                assert!(output[256]);
                loads.push((pc, address, register_data));
            }
            if pc == 0x78 {
                assert!(tick - previous_commit >= 64 * 457);
                assert!(output[256]);
                assert_eq!(register_data, 0x0000_0005_0000_0003);
            }
            assert_eq!(pc, expected[commits.len()], "tick {tick}");
            commits.push(pc);
            previous_commit = tick;
            if commits.len() == expected.len() {
                break;
            }
        }
    }
    assert_eq!(commits, expected);
    assert_eq!(
        stores,
        [
            (0x24, 0x11000),
            (0x54, 0x2ffe8),
            (0xc4, 0x2fff8),
            (0xc8, 0x2fff0),
        ]
    );
    assert_eq!(
        loads,
        [
            (0x58, 0x2ffe8, 0x7a65_0c69_1bd3_482f),
            (0x70, 0x100d8, 0x0000_0001_0000_0001),
            (0x74, 0x100e0, 0x0000_0002_0000_0003),
            (0x84, 0x100d0, 0x7a65_0c69_1bd3_482f),
            (0x88, 0x11000, 0),
        ]
    );
    let output = vm.step(&product_address).unwrap();
    assert_eq!(word(&output[128..192]), 0x2fff8);
    assert_eq!(word(&output[64..128]), 0x0000_0005_0000_0003);
    let status_address = bits(0x2fff0);
    for _ in 0..39 {
        vm.step(&status_address).unwrap();
    }
    let output = vm.step(&status_address).unwrap();
    assert_eq!(word(&output[128..192]), 0x2fff0);
    assert_eq!(word(&output[64..128]), 1);
}

fn bits(value: u64) -> [bool; 64] {
    std::array::from_fn(|bit| ((value >> bit) & 1) != 0)
}

fn word(bits: &[bool]) -> u64 {
    bits.iter()
        .enumerate()
        .map(|(bit, &value)| u64::from(value) << bit)
        .sum()
}
