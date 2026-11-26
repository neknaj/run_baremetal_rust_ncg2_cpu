//! ブート ROM が UART のワードを RAM に書き、物理アドレス 0x10000 から実行することを確認する。

use std::{fs, path::Path};

use circuitgame2_core::project::{
    ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, check_use_aliases,
    index_use_targets, parse_circuit_project,
};
use ncg2_rv64_arithmetic_proof::event_clocked::EventVm;

fn word(bits: &[bool]) -> u64 {
    bits.iter().enumerate().fold(0, |value, (offset, &bit)| {
        value | (u64::from(bit) << offset)
    })
}

#[test]
fn boot_transfers_control_to_writable_ram() {
    let config = circuitgame2_project_config::parse(include_bytes!("../../firmware.toml")).unwrap();
    let mut source_paths: Vec<_> = config
        .sources
        .get_ref()
        .iter()
        .map(|path| path.get_ref().as_str())
        .filter(|path| {
            !matches!(
                *path,
                "circuit/firmware-rom.ncg2"
                    | "circuit/firmware-memory.ncg2"
                    | "circuit/firmware-machine.ncg2"
                    | "circuit/firmware-demo.ncg2"
            )
        })
        .collect();
    source_paths.extend([
        "circuit/selfhost-map.ncg2",
        "circuit/uart-boot-rom.ncg2",
        "circuit/uart-boot-fetch.ncg2",
        "circuit/uart-pc.ncg2",
        "circuit/uart-firmware-machine.ncg2",
        "verification/tests/uart_entry.ncg2",
    ]);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let texts: Vec<_> = source_paths
        .iter()
        .map(|path| fs::read_to_string(root.join(path)).unwrap())
        .collect();
    let snapshots: Vec<_> = source_paths
        .iter()
        .zip(&texts)
        .map(|(&path, text)| SourceSnapshot {
            path,
            text,
            revision: 0,
        })
        .collect();
    let project = parse_circuit_project(&snapshots).unwrap();
    let targets = index_use_targets(&project).unwrap();
    let aliases = check_use_aliases(&targets).unwrap();
    let file = source_paths.len() - 1;
    let circuit = &project.circuits()[file];
    let entry_form = circuit
        .names
        .iter()
        .find(|name| name.name.word == "uart-entry-fixture")
        .unwrap()
        .form;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|item| item.form == entry_form)
        .unwrap();
    let mut built = aliases
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
    let mut event = EventVm::new(&built).unwrap();
    let reset = built.reset();
    assert_eq!(event.reset(), reset);
    assert_eq!(word(&reset[..64]), 0);

    // 0x10000: AUIPC t0,0。0x10004: JAL t1,+8。0x1000c: JALR t2,16(t0)。
    // 不正命令のある 0x10008 は飛ばし、0x10010 の JAL x0,0 に到達する。
    let image = [
        0x0080_036f_0000_0297,
        0x0102_83e7_0000_0000,
        0x0000_0000_0000_006f,
    ];
    let stream = [image.len() as u64, image[0], image[1], image[2]];
    let mut consumed = 0;
    let mut written = 0;
    let mut app_commits = Vec::new();
    let mut expected_pc_after_commit = None;
    for tick in 1..=65_000 {
        let ready = consumed < stream.len();
        let data = stream.get(consumed).copied().unwrap_or(0);
        let input: [bool; 129] = std::array::from_fn(|bit| match bit {
            0 => ready,
            1..=64 => data & (1 << (bit - 1)) != 0,
            _ => bit == 65 + 16,
        });
        let output = event.step(&input).unwrap();
        if tick <= 1050 {
            assert_eq!(output, built.step(&input).unwrap(), "tick {tick}");
        }
        let pc = word(&output[..64]);
        if let Some(target) = expected_pc_after_commit.take() {
            assert_eq!(pc, target, "tick {tick}");
        }
        assert!(!output[194], "tick {tick}, PC {pc:#x} で停止");
        if output[195] {
            assert!(ready);
            consumed += 1;
        }
        if output[196] {
            let data = word(&output[261..325]);
            assert_eq!(word(&output[64..128]), 0x10000 + 8 * written as u64);
            assert_eq!(data, image[written]);
            // 同じ語に書くフレームで、RAM ポートは更新前の値を返す。
            assert_eq!(word(&output[197..261]), 0);
            written += 1;
        }
        if output[193] && pc >= 0x10000 {
            assert_eq!(consumed, stream.len());
            assert_eq!(written, image.len());
            let register_value = output[192].then(|| word(&output[128..192]));
            app_commits.push((pc, register_value));
            expected_pc_after_commit = match pc {
                0x10000 => Some(0x10004),
                0x10004 => Some(0x1000c),
                0x1000c => Some(0x10010),
                0x10010 => Some(0x10010),
                _ => panic!("予期しないアプリの PC {pc:#x}"),
            };
            if pc == 0x10010 {
                break;
            }
        }
    }
    assert_eq!(
        &app_commits[..3],
        &[
            (0x10000, Some(0x10000)),
            (0x10004, Some(0x10008)),
            (0x1000c, Some(0x10010))
        ]
    );
    assert_eq!(app_commits.len(), 4);
    assert_eq!(app_commits[3].0, 0x10010);
}
