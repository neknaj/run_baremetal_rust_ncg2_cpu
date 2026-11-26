#![no_std]
#![no_main]

//! RAM から1件の要求を読み、Core の厳格 S 式解析と NOR 評価を実行する。

extern crate alloc;

use alloc::vec;
use circuitgame2_core::netlist::{NorGate, NorNetlist, Wire};
use core::alloc::{GlobalAlloc, Layout};
use core::arch::global_asm;
use core::cell::UnsafeCell;
use core::panic::PanicInfo;
use core::ptr::{addr_of, addr_of_mut, read_volatile, write_volatile};

global_asm!(include_str!("start.S"));

const HEAP_SIZE: usize = 8192;
const SOURCE_CAPACITY: usize = 128;

#[repr(align(64))]
struct Heap {
    bytes: UnsafeCell<[u8; HEAP_SIZE]>,
    used: UnsafeCell<usize>,
}

// SAFETY: 単一ハートの起動コードで使い、割り当てに再入しない。
unsafe impl Sync for Heap {}

unsafe impl GlobalAlloc for Heap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let bytes = self.bytes.get().cast::<u8>();
        let base = bytes.addr();
        let used = self.used.get();
        // SAFETY: 割り当て済みの長さを読み書きするのは単一ハートだけである。
        let offset = unsafe { *used };
        let Some(start) = base.checked_add(offset).and_then(|p| p.checked_add(layout.align() - 1)) else {
            return core::ptr::null_mut();
        };
        let offset = (start & !(layout.align() - 1)) - base;
        let Some(end) = offset.checked_add(layout.size()) else {
            return core::ptr::null_mut();
        };
        if end > HEAP_SIZE {
            return core::ptr::null_mut();
        }
        // SAFETY: 新たな割り当てがヒープ内に収まり、以後の割り当てから除外する。
        unsafe {
            *used = end;
            bytes.add(offset)
        }
    }

    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}

#[global_allocator]
static ALLOCATOR: Heap = Heap { bytes: UnsafeCell::new([0; HEAP_SIZE]), used: UnsafeCell::new(0) };

#[derive(Clone, Copy)]
#[repr(C)]
struct Request {
    source_length: u64,
    inputs: u64,
    source: [u8; SOURCE_CAPACITY],
}

#[repr(C)]
struct ResultWords {
    status: u64,
    information: u64,
}

#[used]
#[unsafe(link_section = ".request")]
static mut REQUEST: Request = Request { source_length: 0, inputs: 0, source: [0; SOURCE_CAPACITY] };

#[used]
#[unsafe(link_section = ".result")]
static mut RESULT: ResultWords = ResultWords { status: 0, information: 0 };

fn publish(status: u64, information: u64) {
    // SAFETY: リンカが結果専用に確保した領域へ、単一ハートが書き込む。
    unsafe {
        write_volatile(addr_of_mut!(RESULT.information), information);
        write_volatile(addr_of_mut!(RESULT.status), status);
    }
}

/// RAM の要求を1回処理する。情報の下位8ビットは AST ノード数、その次のビットは NOR 出力である。
/// 上位48ビットには、この要求の末尾までに予約したヒープのバイト数を格納する。
#[unsafe(no_mangle)]
extern "C" fn rust_main() {
    // SAFETY: リンカの要求領域は RAM にあり、起動前に設定されてから書き換わらない。
    let request = unsafe { read_volatile(addr_of!(REQUEST)) };
    let Some(bytes) = request.source.get(..request.source_length as usize) else {
        publish(2, 0);
        return;
    };
    let Ok(source) = core::str::from_utf8(bytes) else {
        publish(3, 0);
        return;
    };
    let Ok(parsed) = circuitgame2_core::syntax::parse(source) else {
        publish(4, 0);
        return;
    };
    let netlist = NorNetlist {
        input_count: 2,
        gates: vec![NorGate { left: Wire::Input(0), right: Wire::Input(1) }],
        outputs: vec![Wire::Gate(0)],
    };
    let Ok(output) = netlist.evaluate(&[request.inputs & 1 != 0, request.inputs & 2 != 0]) else {
        publish(5, 0);
        return;
    };
    // SAFETY: 割り当て処理と同じ単一ハートから、現在の予約済みサイズを読む。
    let heap_used = unsafe { *ALLOCATOR.used.get() } as u64;
    publish(1, (heap_used << 16) | (u64::from(output[0]) << 8) | parsed.len() as u64);
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    publish(6, 0);
    loop {
        core::hint::spin_loop();
    }
}
