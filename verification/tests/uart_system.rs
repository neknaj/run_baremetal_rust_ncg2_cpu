//! UART のビット列から書き込んだ機械語を、実際のブート ROM から起動する。

use std::{convert::Infallible, path::Path};

use ncg2_rv64_arithmetic_proof::event_clocked::EventVm;

#[path = "../support/uart.rs"]
mod uart;
use uart::word;

const PROBE: u64 = 0x10020;
const READY: usize = 192;
const ERROR: usize = 193;
const STOPPED: usize = 195;
const BUS_WRITE: usize = 196;

#[test]
fn boots_two_words_sent_as_uart_frames_and_observes_the_application_store() {
    let config = Path::new(env!("CARGO_MANIFEST_DIR")).join("../uart-boot.toml");
    let mut reference = uart::machine(&config).unwrap();
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
    for _ in 0..uart::BIT_TICKS {
        tick(true);
    }
    // 0x10000: LUI t0,0x10; ADDI t1,x0,42; SD t1,32(t0); JAL x0,0。
    for frame in [2_u64, 0x02a0_0313_0001_02b7, 0x0000_006f_0262_b023] {
        assert!(
            uart::send(frame.to_le_bytes(), |rx| Ok::<_, Infallible>(tick(rx).0)).unwrap(),
            "ブート ROM が受信語を消費しなかった",
        );
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
