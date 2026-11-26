#![no_std]
#![no_main]

//! RV64IM の起動と RAM の読み書きを検査するファームウェア。

use core::arch::global_asm;
use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

global_asm!(include_str!("start.S"));

const EXPECTED: u64 = 0x0000_0005_0000_0003;
const STACK_PROBE: u64 = 0x7a65_0c69_1bd3_482f;

#[repr(u64)]
enum Status {
    Passed = 1,
    WrongProduct = 2,
    UninitializedBss = 3,
    Panic = 4,
    WrongStackProbe = 5,
}

#[repr(C)]
struct BootResult {
    status: u64,
    product: u64,
}

#[used]
#[unsafe(link_section = ".result")]
static mut RESULT: BootResult = BootResult { status: 0, product: 0 };

static LEFT: u64 = 0x0000_0001_0000_0001;
static RIGHT: u64 = 0x0000_0002_0000_0003;
static mut ZERO: u64 = 0;

fn publish(status: Status, product: u64) {
    // SAFETY: リンカがこの領域を独立に確保する。単一ハートだけが書き込み、他の参照を作らない。
    unsafe {
        write_volatile(addr_of_mut!(RESULT.product), product);
        write_volatile(addr_of_mut!(RESULT.status), status as u64);
    }
}

/// 初期化したスタック上で64ビットの乗算を行い、結果を固定アドレスに残す。
#[unsafe(no_mangle)]
extern "C" fn rust_main(stack_probe: u64) {
    // SAFETY: 入力値と初期化の確認用変数は静的領域にあり、単一ハートで読む。
    let (left, right, zero) = unsafe {
        (read_volatile(addr_of!(LEFT)), read_volatile(addr_of!(RIGHT)), read_volatile(addr_of!(ZERO)))
    };
    let product = left.wrapping_mul(right);
    let status = if stack_probe != STACK_PROBE {
        Status::WrongStackProbe
    } else if zero != 0 {
        Status::UninitializedBss
    } else if product != EXPECTED {
        Status::WrongProduct
    } else {
        Status::Passed
    };
    publish(status, product);
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    publish(Status::Panic, 0);
    loop {
        core::hint::spin_loop();
    }
}
