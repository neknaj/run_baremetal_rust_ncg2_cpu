use ncg2_event_queue::{
    fanout::Pin::{Constant, Slot},
    shared_nor::{SharedNorGroup, SharedNorGroupError},
};

#[test]
fn assigns_nonconsecutive_gate_ids_in_the_supplied_order() {
    let shared = Slot(7);
    let gates = [
        (shared, Slot(3)),
        (Constant(true), Constant(false)),
        (Constant(false), shared),
        (shared, shared),
    ];
    let group = SharedNorGroup::new(&gates, &[2, 0, 3], shared).ok().unwrap();

    assert!(matches!(group.shared(), Slot(7)));
    assert!(matches!(group.lanes()[0].other, Constant(false)));
    assert!(matches!(group.lanes()[1].other, Slot(3)));
    assert!(matches!(group.lanes()[2].other, Slot(7)));
    assert_eq!(group.lanes().iter().map(|lane| lane.gate).collect::<Vec<_>>(), [2, 0, 3]);
    assert_eq!(group.evaluate(false, 0b010), 0b101);
    assert_eq!(group.evaluate(true, u64::MAX), 0);
}

#[test]
fn distinguishes_the_first_invalid_lane() {
    let gates = [(Slot(0), Slot(1)), (Slot(1), Slot(2))];
    assert_eq!(SharedNorGroup::new(&gates, &[], Slot(1)).err(), Some(SharedNorGroupError::Width));
    assert_eq!(
        SharedNorGroup::new(&gates, &[0, 1, 0, 2], Slot(1)).err(),
        Some(SharedNorGroupError::DuplicateGate { lane: 2, first: 0 }),
    );
    assert_eq!(
        SharedNorGroup::new(&gates, &[0, usize::MAX], Slot(0)).err(),
        Some(SharedNorGroupError::UnknownGate { lane: 1 }),
    );
    assert_eq!(
        SharedNorGroup::new(&gates, &[0, 1, 0], Slot(0)).err(),
        Some(SharedNorGroupError::SharedPinMissing { lane: 1 }),
    );
}

#[test]
fn keeps_the_last_bit_of_a_full_word() {
    let gates = [(Constant(false), Slot(3)); 64];
    let ids = core::array::from_fn::<_, 64, _>(|lane| lane);
    let group = SharedNorGroup::new(&gates, &ids, Constant(false)).ok().unwrap();
    assert_eq!(group.evaluate(false, u64::MAX >> 1), 1 << 63);
}
