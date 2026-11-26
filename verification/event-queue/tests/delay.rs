use ncg2_event_queue::delay::{DelayHistory, DelayHistoryError};

#[test]
fn observes_each_stage_and_the_endpoint_change_boundary() {
    let mut stages = vec![true, false, true, false];
    let mut history = DelayHistory::new(stages.clone()).unwrap();
    history.extend_through(4, true).unwrap();
    history.extend_through(7, false).unwrap();
    history.extend_through(16, true).unwrap();

    assert_eq!(history.extend_through(16, false), Err(DelayHistoryError::PastTick));
    assert_eq!(history.endpoint_unchanged_through(4, 16), Some(7));
    assert_eq!(history.endpoint_unchanged_through(8, 16), Some(10));
    assert_eq!(history.endpoint_unchanged_through(11, 20), Some(19));
    assert_eq!(history.endpoint_unchanged_through(20, 20), None);

    for tick in 0..=16 {
        if tick != 0 {
            stages.rotate_right(1);
            stages[0] = tick <= 4 || tick >= 8;
        }
        for (stage, &expected) in stages.iter().enumerate() {
            assert_eq!(history.stage(stage, tick), Some(expected));
        }
    }
}

#[test]
fn limits_the_last_known_change_without_overflow() {
    let mut history = DelayHistory::new(vec![false, false]).unwrap();
    history.extend_through(u64::MAX, true).unwrap();
    assert_eq!(history.endpoint_unchanged_through(2, u64::MAX), Some(u64::MAX));
    assert_eq!(history.stage(1, u64::MAX), Some(true));
}
