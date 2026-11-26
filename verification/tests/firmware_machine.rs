//! 起動 ELF をフェッチし、Core の VM 上で命令とメモリアクセスを照合する。

use circuitgame2_core::netlist::ClockedVm;
use circuitgame2_core::project::{
    ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, check_use_aliases,
    index_use_targets, parse_circuit_project,
};
use ncg2_rv64_arithmetic_proof::event_clocked::EventVm;

fn machine(rom_override: Option<&str>) -> ClockedVm {
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
        "execute-alu",
        "multiply-wide",
        "subword-memory",
        "execute-subword",
        "firmware-dispatch",
        "firmware-machine",
    ];
    let sources: Vec<_> = sources
        .iter()
        .map(|name| {
            let path = format!("../circuit/{name}.ncg2");
            let text = match (rom_override, *name) {
                (Some(rom), "firmware-rom") => rom.to_owned(),
                _ => std::fs::read_to_string(&path).unwrap(),
            };
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
    aliases
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
        .vm
}

#[test]
fn runs_the_rust_elf_to_its_halt_loop() {
    let mut vm = machine(None);
    assert_eq!(vm.reset(), [false; 260]);
    let mut sparse = EventVm::new(&vm).unwrap();
    assert_eq!(sparse.output(), [false; 260]);

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
        assert_eq!(
            sparse.step(&product_address).unwrap(),
            output,
            "tick {tick}"
        );
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
    assert_eq!(sparse.step(&product_address).unwrap(), output);
    for index in 0..vm.storage_count() {
        assert_eq!(sparse.ram_memory(index), vm.ram_memory(index));
    }
    assert_eq!(word(&output[128..192]), 0x2fff8);
    assert_eq!(word(&output[64..128]), 0x0000_0005_0000_0003);
    let status_address = bits(0x2fff0);
    for _ in 0..39 {
        assert_eq!(
            sparse.step(&status_address).unwrap(),
            vm.step(&status_address).unwrap()
        );
    }
    let output = vm.step(&status_address).unwrap();
    assert_eq!(sparse.step(&status_address).unwrap(), output);
    assert_eq!(word(&output[128..192]), 0x2fff0);
    assert_eq!(word(&output[64..128]), 1);
}

#[test]
fn partial_stores_and_extended_alu_retire_through_the_cpu() {
    let instructions: [u32; 18] = [
        0x0001_10b7, // lui x1,0x11
        0x0001_0137, // lui x2,0x10
        0x0601_3183, // ld x3,0x60(x2)
        0x0030_b023, // sd x3,0(x1)
        0xf830_0193, // addi x3,x0,-125
        0x0030_80a3, // sb x3,1(x1)
        0x0010_8203, // lb x4,1(x1)
        0x0010_c283, // lbu x5,1(x1)
        0x0040_9123, // sh x4,2(x1)
        0x0050_a223, // sw x5,4(x1)
        0x0000_b303, // ld x6,0(x1)
        0x43f2_5393, // srai x7,x4,63
        0x0053_8433, // add x8,x7,x5
        0xfff2_c493, // xori x9,x5,-1
        0x0080_b423, // sd x8,8(x1)
        0xfff0_0493, // addi x9,x0,-1
        0x0294_b533, // mulhu x10,x9,x9
        0x0040_90a3, // sh x4,1(x1)
    ];
    let mut words = [0; 32];
    for (index, pair) in instructions.chunks_exact(2).enumerate() {
        words[index] = u64::from(pair[0]) | (u64::from(pair[1]) << 32);
    }
    words[12] = 0xa1b2_c3d4_e5f6_0718;
    let mut vm = machine(Some(&constant_rom(words)));
    vm.reset();
    let input = bits(0x11000);
    let mut committed = Vec::new();
    let mut stopped = false;
    for _ in 0..54_000 {
        let output = vm.step(&input).unwrap();
        let pc = word(&output[..64]);
        if output[258] {
            committed.push(pc);
            match pc {
                0x14 | 0x20 | 0x24 => {
                    assert!(output[257]);
                    assert_eq!(word(&output[128..192]), 0x11000);
                }
                0x18 => assert_eq!(word(&output[192..256]), (-125i64) as u64),
                0x1c => assert_eq!(word(&output[192..256]), 0x83),
                0x28 => assert_eq!(word(&output[192..256]), 0x0000_0083_ff83_8318),
                0x2c => assert_eq!(word(&output[192..256]), u64::MAX),
                0x30 => assert_eq!(word(&output[192..256]), 0x82),
                0x34 => assert_eq!(word(&output[192..256]), !0x83),
                0x38 => {
                    assert!(output[257]);
                    assert_eq!(word(&output[128..192]), 0x11008);
                }
                0x40 => {
                    assert!(output[256]);
                    assert_eq!(word(&output[192..256]), u64::MAX - 1);
                }
                _ => {}
            }
        }
        if output[259] {
            assert_eq!(pc, 0x44);
            stopped = true;
        }
        if stopped {
            assert!(!output[256] && !output[257] && !output[258]);
        }
    }
    assert!(stopped);
    assert_eq!(committed, (0..0x44).step_by(4).collect::<Vec<_>>());
    assert_eq!(
        word(&vm.step(&input).unwrap()[64..128]),
        0x0000_0083_ff83_8318
    );
}

fn constant_rom(words: [u64; 32]) -> String {
    use std::fmt::Write;
    let mut source = String::from(
        "(namespace rv64 (export firmware-rom) (func firmware-rom (in (addr (bits 5))) (out (word (bits 64)))",
    );
    let mut leaves = Vec::new();
    for (index, word) in words.into_iter().enumerate() {
        let name = format!("literal-{index}");
        writeln!(source, "(def {name} #x{word:016x})").unwrap();
        leaves.push(name);
    }
    for bit in 0..5 {
        writeln!(source, "(def s-{bit} (bit addr {bit}))").unwrap();
        leaves = leaves
            .chunks_exact(2)
            .enumerate()
            .map(|(column, pair)| {
                let name = if bit == 4 {
                    "word".to_owned()
                } else {
                    format!("stage-{bit}-{column}")
                };
                writeln!(
                    source,
                    "(def {name} (::rv64::arithmetic::select s-{bit} {} {}))",
                    pair[0], pair[1]
                )
                .unwrap();
                name
            })
            .collect();
    }
    source.push_str("))");
    source
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
