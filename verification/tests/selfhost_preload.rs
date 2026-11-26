//! 起動前の全入力語と、CPU のデータポートへ切り替える tick を検査する。

use circuitgame2_core::project::{
    ClockedModuleVmLimits, ModuleBodyPosition, SourceSnapshot, check_use_aliases,
    index_use_targets, parse_circuit_project,
};

#[path = "../fixtures/preload.rs"]
mod fixture;

const REQUEST_BASE: u64 = 0x2bf00;
const REQUEST_WORDS: usize = 18;
const REQUEST_SOURCE: &[u8] = b"(using nor)\n(func invert (in a) (out b) (def b (nor a a)))";

/// RAM 入力のバイト仕様から、独立にリトルエンディアンの語を得る。
fn request_values() -> [u64; REQUEST_WORDS] {
    let mut request = [0; REQUEST_WORDS * 8];
    assert_eq!(REQUEST_SOURCE.len(), 58);
    request[..8].copy_from_slice(&(REQUEST_SOURCE.len() as u64).to_le_bytes());
    request[16..16 + REQUEST_SOURCE.len()].copy_from_slice(REQUEST_SOURCE);
    std::array::from_fn(|index| {
        let start = index * 8;
        u64::from_le_bytes(request[start..start + 8].try_into().unwrap())
    })
}

fn word(bits: &[bool]) -> u64 {
    bits.iter()
        .enumerate()
        .map(|(bit, &value)| u64::from(value) << bit)
        .sum()
}

fn signal(address: u64, write: Option<u64>) -> [bool; 129] {
    std::array::from_fn(|bit| match bit {
        0..64 => (address >> bit) & 1 != 0,
        64..128 => (write.unwrap_or_default() >> (bit - 64)) & 1 != 0,
        _ => write.is_some(),
    })
}

#[test]
fn preloads_all_words_and_accepts_cpu_writes_only_after_ready() {
    let snapshots: Vec<_> = fixture::SOURCES
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
    let file = snapshots.len() - 1;
    let circuit = &project.circuits()[file];
    let entry = |name| {
        let form = circuit
            .names
            .iter()
            .find(|decl| decl.name.word == name)
            .unwrap()
            .form;
        circuit
            .syntax
            .entries
            .iter()
            .position(|entry| entry.form == form)
            .unwrap()
    };
    let limits = ClockedModuleVmLimits {
        width: 65_536,
        wiring_bits: 1_048_576,
        gates: 1_048_576,
        state_bits: 1_048_576,
    };
    let values = request_values();

    let compiled = aliases
        .expand_reachable_function(
            (file, entry("selfhost-request-word")),
            limits.width,
            limits.wiring_bits,
            limits.gates,
        )
        .unwrap()
        .unwrap();
    // 添字は5ビットなので、受理と拒否を含め、すべての入力を確認できる。
    for index in 0..32 {
        let input: [bool; 5] = std::array::from_fn(|bit| index & (1 << bit) != 0);
        let output = compiled.netlist().evaluate(&input).unwrap();
        assert_eq!(word(&output[..64]), REQUEST_BASE + 8 * index as u64);
        assert_eq!(
            word(&output[64..128]),
            values.get(index).copied().unwrap_or(0)
        );
        assert_eq!(word(&output[128..133]), ((index + 1) % 32) as u64);
        assert_eq!(output[133], index < REQUEST_WORDS);
    }

    let mut loader = aliases
        .lower_clocked_vm(
            ModuleBodyPosition {
                file,
                entry: entry("selfhost-preload"),
            },
            limits,
        )
        .unwrap()
        .unwrap()
        .vm;
    assert_eq!(&loader.reset()[128..], [false, false]);
    let mut requests = Vec::new();
    let mut finished = None;
    for tick in 1..=820 {
        let output = loader.step(&[]).unwrap();
        if output[128] {
            assert!(!output[129]);
            requests.push((tick, word(&output[..64]), word(&output[64..128])));
        }
        if output[129] {
            finished.get_or_insert(tick);
        }
        assert_eq!(
            output[129],
            finished.is_some(),
            "tick {tick}, first finished {finished:?}, requests {requests:?}"
        );
    }
    assert_eq!(requests.len(), REQUEST_WORDS);
    for (index, &(tick, address, value)) in requests.iter().enumerate() {
        assert_eq!(tick, 42 + 37 * index);
        assert_eq!(
            (address, value),
            (REQUEST_BASE + index as u64 * 8, values[index])
        );
    }
    let finished = finished.unwrap();
    assert_eq!(finished, 715);
    assert!(finished >= requests.last().unwrap().0 + 22);

    let mut memory = aliases
        .lower_clocked_vm(
            ModuleBodyPosition {
                file,
                entry: entry("selfhost-preloaded-memory"),
            },
            limits,
        )
        .unwrap()
        .unwrap()
        .vm;
    assert!(!memory.reset()[66]);
    // ロード中の CPU が書き込みを要求しても、固定入力と未使用領域のどちらも上書きしない。
    let unused_address = REQUEST_BASE + 31 * 8;
    let blocked = signal(unused_address, Some(u64::MAX));
    for tick in 1..=finished + 8 {
        assert_eq!(memory.step(&blocked).unwrap()[66], tick == finished + 8);
    }
    assert!(finished + 8 <= 800);
    for index in 0..32 {
        let read = signal(REQUEST_BASE + index * 8, None);
        for _ in 0..52 {
            assert!(memory.step(&read).unwrap()[66]);
        }
        let output = memory.step(&read).unwrap();
        let expected = values.get(index as usize).copied().unwrap_or(0);
        assert_eq!(word(&output[..64]), expected, "要求領域の語 {index}");
        assert_eq!(&output[64..], [true, true, true]);
    }

    let replacement = 0x0123_4567_89ab_cdef;
    memory
        .step(&signal(unused_address, Some(replacement)))
        .unwrap();
    let read = signal(unused_address, None);
    for _ in 0..33 {
        memory.step(&read).unwrap();
    }
    assert_eq!(word(&memory.step(&read).unwrap()[..64]), replacement);

    assert!(!memory.reset()[66]);
    for _ in 0..finished + 8 {
        memory.step(&signal(REQUEST_BASE, None)).unwrap();
    }
    for _ in 0..53 {
        memory.step(&signal(REQUEST_BASE, None)).unwrap();
    }
    let output = memory.step(&signal(REQUEST_BASE, None)).unwrap();
    assert_eq!(word(&output[..64]), 58);
    assert_eq!(&output[64..], [true, true, true]);
}
