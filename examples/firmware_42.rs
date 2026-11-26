#![no_std]
#![no_main]

//! 固定の結果領域へ計算結果を書き込む、ベアメタル Rust の実行例。

use core::{arch::global_asm, panic::PanicInfo, ptr::write_volatile};

global_asm!(include_str!("../src/start.S"));

#[used]
#[unsafe(link_section = ".result")]
static mut RESULT: [u64; 2] = [0; 2];

fn publish(status: u64, value: u64) {
    let words = &raw mut RESULT;
    // SAFETY: リンカは結果の2語を専用領域に置く。単一ハートで書き込み、参照を作らない。
    unsafe {
        write_volatile(&raw mut (*words)[1], value);
        write_volatile(&raw mut (*words)[0], status);
    }
}

/// 起動コードから呼ばれ、任意の計算結果を結果領域に書く。
#[unsafe(no_mangle)]
extern "C" fn rust_main(_stack_probe: u64) {
    let result = 6 * 7;
    publish(1, result);
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    publish(4, 0);
    loop {
        core::hint::spin_loop();
    }
}
