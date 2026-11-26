//! UART のビット列から書き込んだ機械語を、実際のブート ROM から起動する。

use std::{fs, path::Path};

use circuitgame2_core::{
    netlist::ClockedVm,
    project::{
        ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, check_use_aliases,
        index_use_targets, parse_circuit_project,
    },
};
use ncg2_rv64_arithmetic_proof::event_clocked::EventVm;

const PROBE: u64 = 0x10020;
const READY: usize = 192;
const ERROR: usize = 193;
const STOPPED: usize = 195;
const BUS_WRITE: usize = 196;
const TICKS_PER_BIT: usize = 16;

fn lower_uart_system() -> ClockedVm {
    let config =
        circuitgame2_project_config::parse(include_bytes!("../../uart-boot.toml")).unwrap();
    let entry_name = config.entry.as_ref().unwrap().get_ref();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let paths = config.sources.get_ref();
    let texts: Vec<_> = paths
        .iter()
        .map(|path| fs::read_to_string(root.join(path.get_ref())).unwrap())
        .collect();
    let sources: Vec<_> = paths
        .iter()
        .zip(&texts)
        .map(|(path, text)| SourceSnapshot {
            path: path.get_ref(),
            text,
            revision: 0,
        })
        .collect();
    let project = parse_circuit_project(&sources).unwrap();
    let targets = index_use_targets(&project).unwrap();
    let aliases = check_use_aliases(&targets).unwrap();
    let file = sources.len() - 1;
    let circuit = &project.circuits()[file];
    let entry_form = circuit
        .names
        .iter()
        .find(|item| {
            item.namespace.as_ref().is_some_and(|namespace| {
                format!("::{}::{}", namespace.word, item.name.word) == *entry_name
            })
        })
        .unwrap()
        .form;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|item| item.form == entry_form)
        .unwrap();
    aliases
        .lower_clocked_vm(
            ModuleBodyPosition { file, entry },
            ClockedModuleVmLimits {
                width: 65_536,
                wiring_bits: 1_048_576,
                gates: 1_048_576,
                state_bits: 8_388_608,
            },
        )
        .unwrap()
        .unwrap()
        .vm
}

fn word(bits: &[bool]) -> u64 {
    bits.iter().enumerate().fold(0, |value, (offset, &bit)| {
        value | (u64::from(bit) << offset)
    })
}

#[test]
fn boots_two_words_sent_as_uart_frames_and_observes_the_application_store() {
    let mut reference = lower_uart_system();
    let mut event = EventVm::new(&reference).unwrap();
    assert_eq!(event.reset(), reference.reset());
    let mut input = [false; 65];
    for bit in 0..64 {
        input[bit + 1] = (PROBE >> bit) & 1 != 0;
    }
    let mut ticks = 0;
    let mut last_address = 0;
    let mut address_ticks = 0;
    let mut tick = |rx: bool| {
        ticks += 1;
        input[0] = rx;
        let output = event.step(&input).unwrap();
        assert_eq!(output, reference.step(&input).unwrap(), "tick {ticks}");
        let address = word(&output[64..128]);
        if address == last_address {
            address_ticks += 1;
        } else {
            last_address = address;
            address_ticks = 1;
        }
        if output[BUS_WRITE] {
            assert!(address_ticks >= 32, "tick {ticks}, 書き込み先 {address:#x}");
        }
        assert!(
            !output[ERROR] && !output[STOPPED],
            "tick {ticks}, PC {:#x}",
            word(&output[..64])
        );
        (
            output[READY],
            word(&output[..64]),
            address,
            word(&output[128..192]),
            address_ticks,
        )
    };
    for _ in 0..TICKS_PER_BIT {
        tick(true);
    }
    // 0x10000: LUI t0,0x10; ADDI t1,x0,42; SD t1,32(t0); JAL x0,0。
    for frame in [2_u64, 0x02a0_0313_0001_02b7, 0x0000_006f_0262_b023] {
        for byte in frame.to_le_bytes() {
            for level in std::iter::once(false)
                .chain((0..8).map(|bit| (byte >> bit) & 1 != 0))
                .chain(std::iter::once(true))
            {
                for _ in 0..TICKS_PER_BIT {
                    tick(level);
                }
            }
        }
        let mut was_ready = false;
        let mut acknowledged = false;
        for _ in 0..20_000 {
            let (ready, ..) = tick(true);
            if was_ready && !ready {
                acknowledged = true;
                break;
            }
            was_ready |= ready;
        }
        assert!(acknowledged, "ブート ROM が受信語を消費しなかった");
    }
    assert!(
        (0..20_000).any(|_| {
            let (_, pc, address, value, stable) = tick(true);
            pc == 0x1000c && address == PROBE && stable >= 52 && value == 42
        }),
        "RAM から起動したアプリの値が観測位置に現れなかった"
    );
    let bank = (0..reference.storage_count())
        .find(|&index| {
            reference
                .storage_wiring(index)
                .and_then(|(wiring, _)| wiring.address.as_ref())
                .is_some_and(|pins| pins.len() == 15)
        })
        .unwrap();
    let standard_ram = reference.ram_memory(bank).unwrap();
    assert_eq!(event.ram_memory(bank).unwrap(), standard_ram);
    let start = ((PROBE as usize >> 3) & 0x7fff) * 64;
    assert_eq!(word(&standard_ram[start..start + 64]), 42);
    eprintln!("RAM から起動して結果を読み出した tick: {ticks}");
}
