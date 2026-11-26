use ncg2_event_queue::{
    fanout::Pin::{Constant, Slot},
    nor_groups::{classify_nor_group, evaluate_nor_group, update_nor_word},
};

#[test]
fn one_circuit_uses_different_pins_per_lane_across_word_boundaries() {
    let gates: Vec<_> = (0..64)
        .map(|lane| (Slot(lane + 3), Slot(lane + 63)))
        .collect();
    let wiring = [0x81aa_c63f_a388_5ac3, 0x7c50_d017_5df8_c2ba];
    let read = |slot: usize| (wiring[slot / 64] >> (slot % 64)) & 1 != 0;
    let group = classify_nor_group(&gates).unwrap();
    let output = evaluate_nor_group(group, &wiring, 127).unwrap();
    for lane in 0..64 {
        assert_eq!((output >> lane) & 1 != 0, !(read(lane + 3) || read(lane + 63)));
    }
    assert_eq!(evaluate_nor_group(group, &wiring[..1], 127), None);
    assert_eq!(evaluate_nor_group(group, &wiring, 126), None);
}

#[test]
fn shared_inputs_and_constants_work_and_irregular_gates_fall_back() {
    let gates = [(Slot(2), Constant(false)); 32];
    let group = classify_nor_group(&gates).unwrap();
    assert_eq!(evaluate_nor_group(group, &[4], 3).unwrap() & u32::MAX as u64, 0);
    assert_eq!(evaluate_nor_group(group, &[0], 3).unwrap() & u32::MAX as u64, u32::MAX as u64);
    assert_eq!(
        classify_nor_group(&[(Slot(2), Constant(false)), (Slot(4), Constant(false))]),
        None,
    );
}

#[test]
fn dirty_mask_retains_the_other_gates_and_reports_only_changes() {
    let (values, changes) = update_nor_word(0b1100, 0b0010, 0b1110, 0b0101);
    assert_eq!(values, 0b1011);
    assert_eq!(changes, 0b0101);
}
