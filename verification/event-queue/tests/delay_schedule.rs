use ncg2_event_queue::delay_schedule::{
    DelayEndUpdate, DelayInputEdge, DelayScheduleError, schedule_delay_edge,
};

#[test]
fn distinguishes_input_phase_and_time_limit() {
    let external = DelayInputEdge::External { tick: 7, value: true };
    let committed = DelayInputEdge::Committed { tick: 7, value: false };
    assert_eq!(
        schedule_delay_edge(4, external, 10),
        Ok(DelayEndUpdate { sample_tick: 7, output_tick: 10, value: true }),
    );
    assert_eq!(
        schedule_delay_edge(4, committed, 11),
        Ok(DelayEndUpdate { sample_tick: 8, output_tick: 11, value: false }),
    );
    assert_eq!(schedule_delay_edge(4, committed, 10), Err(DelayScheduleError::TimeLimit));
    assert_eq!(schedule_delay_edge(0, external, 0), Err(DelayScheduleError::EmptyChain));
    assert_eq!(
        schedule_delay_edge(1, DelayInputEdge::External { tick: 0, value: true }, 0),
        Err(DelayScheduleError::InputBeforeFirstTick),
    );
}

#[test]
fn accepts_the_last_representable_tick_without_wrapping() {
    let last = u64::MAX;
    let external = DelayInputEdge::External { tick: last, value: false };
    assert_eq!(
        schedule_delay_edge(1, external, last),
        Ok(DelayEndUpdate { sample_tick: last, output_tick: last, value: false }),
    );
    assert_eq!(schedule_delay_edge(2, external, last), Err(DelayScheduleError::TimeLimit));
    assert_eq!(
        schedule_delay_edge(1, DelayInputEdge::Committed { tick: last, value: false }, last),
        Err(DelayScheduleError::TimeLimit),
    );
    assert_eq!(
        schedule_delay_edge(1, DelayInputEdge::Committed { tick: 0, value: true }, 1),
        Ok(DelayEndUpdate { sample_tick: 1, output_tick: 1, value: true }),
    );
}
