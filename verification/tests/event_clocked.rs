//! 疎実行器と Core の公開フレーム、RAM の確定内容を比較する。

use circuitgame2_core::netlist::{
    ClockedNor, ClockedStorageInputs, ClockedVm, NorGate, NorNetlist, TickInputError, Wire,
};
use ncg2_rv64_arithmetic_proof::event_clocked::EventVm;

#[test]
fn delay_chains_stop_at_observations_and_exclude_branches_and_cycles() {
    use Wire::{Constant, Input};

    let network = ClockedNor::check(NorNetlist {
        input_count: 9,
        gates: vec![],
        outputs: vec![Input(2), Input(3), Input(7), Input(8)],
    })
    .unwrap();
    let stages = [0, 1, 2, 5, 4, 0, 6, 6]
        .into_iter()
        .map(|input| ClockedStorageInputs {
            address: None,
            data: vec![Input(input)],
            enable: Constant(true),
        })
        .collect();
    let core = ClockedVm::new(network, 1, stages, 8).unwrap();
    let event = EventVm::new(&core).unwrap();

    assert_eq!(event.linear_latch_chains(), vec![vec![0, 1]]);
}

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
            assert_eq!(sparse.step(&[]).unwrap(), core.step(&[]).unwrap());
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
            assert_eq!(
                sparse.step(&input).unwrap(),
                core.step(&input).unwrap(),
                "反復 {iteration} の tick {tick}"
            );
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
