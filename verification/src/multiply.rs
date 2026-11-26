//! MUL の更新回路を独立したビットベクトル仕様と照合する。

use circuitgame2_core::netlist::Wire;

use crate::{name, vector};

/// 初期化と1回の更新の全出力について、参照式と異なる入力を探す。
pub(super) fn emit(outputs: &[Wire]) {
    for (port, start) in [
        ("multiplicand", 0),
        ("multiplier", 64),
        ("accumulated", 128),
        ("mark", 192),
        ("left", 257),
        ("right", 321),
    ] {
        let bits = vector((start..start + 64).map(|index| format!("i{index}")));
        println!("(define-fun {port} () (_ BitVec 64) {bits})");
    }
    for (port, start) in [
        ("next-multiplicand", 0),
        ("next-multiplier", 64),
        ("next-accumulated", 128),
        ("next-mark", 192),
    ] {
        let bits = vector(outputs[start..start + 64].iter().copied().map(name));
        println!("(define-fun {port} () (_ BitVec 64) {bits})");
    }
    println!("(define-fun done () Bool i256)");
    println!("(define-fun initialize () Bool i385)");
    println!("(define-fun next-done () Bool {})", name(outputs[256]));
    print!("{}", include_str!("mul_round.smt2"));
}
