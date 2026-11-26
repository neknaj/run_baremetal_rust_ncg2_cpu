//! 疎実行器と Core の公開フレーム、RAM の確定内容を比較する。

use circuitgame2_core::netlist::{
    ClockedNor, ClockedStorageInputs, ClockedVm, NorGate, NorNetlist, TickInputError, Wire,
};
use ncg2_rv64_arithmetic_proof::event_clocked::EventVm;

#[test]
fn writes_become_visible_on_the_next_read_first_tick_with_no_input_changes() {
    let network = ClockedNor::check(NorNetlist {
        input_count: 1,
        gates: vec![],
        outputs: vec![Wire::Input(0), Wire::Constant(true)],
    })
    .unwrap();
    let mut core = ClockedVm::new(
        network,
        0,
        vec![ClockedStorageInputs {
            address: Some(vec![Wire::Constant(false)]),
            data: vec![Wire::Constant(true)],
            enable: Wire::Constant(true),
        }],
        3,
    )
    .unwrap();
    let mut sparse = EventVm::new(&core).unwrap();
    for _ in 0..2 {
        assert_eq!(sparse.reset(), core.reset());
        assert_eq!(sparse.output(), [false, true]);
        assert_eq!(sparse.ram_memory(0), Some([false, false].as_slice()));
        for (evaluation_count, read) in [(1, false), (1, true), (0, true)] {
            assert_eq!(sparse.step_reusing_inputs(), core.step(&[]).unwrap());
            assert_eq!(sparse.output(), [read, true]);
            assert_eq!(sparse.ram_memory(0), Some([true, false].as_slice()));
            assert_eq!(sparse.evaluated_last_tick(), evaluation_count);
        }
    }

    let from_layout = EventVm::new(&core).unwrap();
    assert_eq!(from_layout.output(), [false, true]);
    assert_eq!(from_layout.ram_memory(0), Some([false, false].as_slice()));
}

#[test]
fn mixes_ram_feedback_latch_and_direct_output_in_the_same_old_state() {
    use Wire::{Constant, Gate, Input};

    let circuit = ClockedNor::check(NorNetlist {
        input_count: 7,
        gates: vec![
            NorGate {
                left: Input(5),
                right: Input(6),
            },
            NorGate {
                left: Gate(0),
                right: Input(3),
            },
            NorGate {
                left: Gate(2),
                right: Gate(2),
            },
        ],
        outputs: vec![
            Input(3),
            Input(4),
            Input(5),
            Input(6),
            Gate(0),
            Gate(1),
            Gate(2),
            Constant(true),
            Input(3),
        ],
    })
    .unwrap();
    let mut core = ClockedVm::new(
        circuit,
        4,
        vec![
            ClockedStorageInputs {
                address: Some(vec![Input(0)]),
                data: vec![Input(1)],
                enable: Input(2),
            },
            ClockedStorageInputs {
                address: Some(vec![Input(0)]),
                data: vec![Input(4)],
                enable: Gate(1),
            },
            ClockedStorageInputs {
                address: None,
                data: vec![Gate(0)],
                enable: Input(3),
            },
        ],
        7,
    )
    .unwrap();
    let mut sparse = EventVm::new(&core).unwrap();
    let mut seed = 0x63e2a174u32;
    for iteration in 0..2 {
        assert_eq!(sparse.reset(), core.reset());
        let mut input = [false; 4];
        for tick in 1..=2048 {
            let prior = input;
            if tick % 4 == 1 {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                input = std::array::from_fn(|bit| seed & (1 << bit) != 0);
            }
            let previous = sparse.output().to_vec();
            if tick % 121 == 0 {
                assert_eq!(sparse.step(&[false]), Err(TickInputError::InputCount));
                assert_eq!(core.step(&[false]), Err(TickInputError::InputCount));
                assert_eq!(sparse.output(), previous);
            }
            let output = if input == prior {
                sparse.step_reusing_inputs()
            } else {
                sparse.step(&input).unwrap()
            };
            assert_eq!(output, core.step(&input).unwrap(), "反復 {iteration} の tick {tick}");
            assert_eq!(sparse.output()[0], input[3]);
            assert_eq!(sparse.output()[6], tick % 2 == 1);
            for i in 0..=core.storage_count() {
                assert_eq!(
                    sparse.ram_memory(i),
                    core.ram_memory(i),
                    "状態素子 {i} の tick {tick}"
                );
            }
        }
    }
}

#[test]
fn grouped_latch_lanes_match_scalar_shift_at_each_tick() {
    use Wire::{Constant, Input};

    let circuit = ClockedNor::check(NorNetlist {
        input_count: 65,
        gates: vec![],
        outputs: (1..=64).map(Input).collect(),
    })
    .unwrap();
    let wiring = ClockedStorageInputs {
        address: None,
        data: (0..64).map(Input).collect(),
        enable: Constant(true),
    };
    let mut core = ClockedVm::new(circuit, 1, vec![wiring], 64).unwrap();
    let mut grouped = EventVm::new_with_latch_lanes(&core).unwrap();
    let mut shifted = EventVm::new_with_shift_latches(&core).unwrap();

    for _ in 0..2 {
        assert_eq!(grouped.reset(), core.reset());
        assert_eq!(shifted.reset(), grouped.output());
        let mut prior = false;
        for tick in 1..=145 {
            let value = [1, 3, 7, 64, 67].contains(&tick);
            assert_eq!(
                grouped.step(&[value]).unwrap(),
                core.step(&[value]).unwrap(),
                "tick {tick}",
            );
            let incremental = if value == prior {
                shifted.step_reusing_inputs()
            } else {
                shifted.step(&[value]).unwrap()
            };
            assert_eq!(incremental, grouped.output(), "tick {tick}");
            prior = value;
        }
    }
}
