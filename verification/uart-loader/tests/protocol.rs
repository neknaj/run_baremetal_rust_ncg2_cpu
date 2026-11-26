use ncg2_uart_loader::{Acceptance, BootError, BootLoader, MAX_WORDS, Receive};

#[test]
fn checks_header_bounds_and_rejects_a_word_without_credit() {
    for invalid in [0, MAX_WORDS as u64 + 1] {
        let mut loader = BootLoader::reset();
        let frame = loader.step(Receive::Word(invalid));
        assert_eq!(frame.accepted, Acceptance::Rejected(BootError::HeaderCount));
        assert!(!frame.cpu_ready && !frame.write);
    }

    let mut loader = BootLoader::reset();
    assert_eq!(loader.step(Receive::Word(MAX_WORDS as u64)).credit, 2);
    assert_eq!(loader.step(Receive::Word(1)).credit, 1);
    assert_eq!(loader.step(Receive::Word(2)).credit, 0);
    let rejected = loader.step(Receive::Word(3));
    assert_eq!(rejected.accepted, Acceptance::Rejected(BootError::NoCredit));
    for _ in 0..40 {
        let frame = loader.step(Receive::Idle);
        assert!(!frame.write && !frame.cpu_ready && frame.committed.is_none());
    }
}

#[test]
fn an_earlier_write_can_finish_after_an_error_without_releasing_the_cpu() {
    let mut loader = BootLoader::reset();
    loader.step(Receive::Word(1));
    let mut frame = loader.step(Receive::Word(0x54));
    for _ in 0..40 {
        if frame.write {
            break;
        }
        frame = loader.step(Receive::Idle);
    }
    assert!(frame.write);
    let requested = frame.bus;
    let failed = loader.step(Receive::Error);
    assert_eq!(failed.accepted, Acceptance::Rejected(BootError::Receiver));
    assert!(!failed.write && !failed.cpu_ready && failed.committed.is_none());
    let drained = loader.step(Receive::Idle);
    assert_eq!(drained.committed, requested);
    assert!(!drained.write && !drained.cpu_ready);
    assert_eq!(loader.error(), Some(BootError::Receiver));
}
