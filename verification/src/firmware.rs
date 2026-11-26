//! バイナリイメージと物理アドレスの区間から、メモリ回路の参照仕様を出力する。

use circuitgame2_core::netlist::{NorNetlist, Wire};

use super::{name, vector};

/// バンク判定の各出力へ入力が到達する経路を、展開済みの NOR 列から調べる。
///
/// 定数を除き、最短と最長がともに12段なら、フレームが途中で混ざらない。
pub(super) fn verify_bank_delay(netlist: &NorNetlist) {
    let mut arrivals = Vec::new();
    let delay = |wire, arrivals: &[Option<(usize, usize)>]| match wire {
        Wire::Input(_) => Some((0, 0)),
        Wire::Constant(_) => None,
        Wire::Gate(index) => arrivals[index],
    };
    for gate in &netlist.gates {
        let shortest = [gate.left, gate.right]
            .into_iter()
            .filter_map(|wire| delay(wire, &arrivals))
            .map(|(min, _)| min + 1)
            .min();
        let longest = [gate.left, gate.right]
            .into_iter()
            .filter_map(|wire| delay(wire, &arrivals))
            .map(|(_, max)| max + 1)
            .max();
        arrivals.push(shortest.zip(longest));
    }
    for &output in &netlist.outputs {
        assert_eq!(delay(output, &arrivals), Some((12, 12)));
    }
}

/// リトルエンディアンのイメージから各読み出しアドレスの期待値を求める。
///
/// ROM の語読み出しは全32添字、フェッチは全64ビットの PC オフセットを検査する。
/// 読み出せないフェッチ先では有効ビットを検査する。
pub(super) fn emit_rom(selected: &str, outputs: &[Wire], image: &[u8]) -> Result<(), &'static str> {
    let rom = image.get(..256).ok_or("イメージの先頭256バイトが必要。")?;
    if image.len() != 128 * 1024 || image[256..].iter().any(|&byte| byte != 0) {
        return Err("イメージは128 KiBで、先頭256バイトより後ろの値は0である必要がある。");
    }
    let fetch = selected == "firmware-fetch";
    let address_width = if fetch { 64 } else { 5 };
    let bytes = if fetch { 4 } else { 8 };
    let address = vector((0..address_width).map(|i| format!("i{i}")));
    println!("(define-fun address () (_ BitVec {address_width}) {address})");
    let mut table = format!("(_ bv0 {})", bytes * 8);
    for (index, chunk) in rom.chunks_exact(bytes).enumerate().rev() {
        let mut little_endian = [0; 8];
        little_endian[..bytes].copy_from_slice(chunk);
        let value = u64::from_le_bytes(little_endian);
        let address = if fetch { index * bytes } else { index };
        table = format!(
            "(ite (= address (_ bv{address} {address_width})) (_ bv{value} {}) {table})",
            bytes * 8,
        );
    }
    let output = vector(outputs[..bytes * 8].iter().copied().map(name));
    if fetch {
        println!(
            "(define-fun valid () Bool (and (bvult address (_ bv256 64)) (= (bvurem address (_ bv4 64)) (_ bv0 64))))"
        );
        println!(
            "(assert (or (distinct {} valid) (and valid (distinct {output} {table}))))",
            name(outputs[32])
        );
    } else {
        println!("(assert (distinct {output} {table}))");
    }
    Ok(())
}

/// 選択ビットと各有効区間の語添字を、絶対アドレスから求めた値と比較する。
pub(super) fn emit_data_map(outputs: &[Wire]) {
    let address = vector((0..64).map(|i| format!("i{i}")));
    println!("(define-fun address () (_ BitVec 64) {address})");
    println!("(define-fun aligned () Bool (= (bvurem address (_ bv8 64)) (_ bv0 64)))");
    let mut differences = Vec::new();
    for (bank, (start, stop, word_bits, out_first)) in [
        (0x10000, 0x10100, 5, 3),
        (0x11000, 0x12000, 9, 8),
        (0x2c000, 0x30000, 11, 17),
    ]
    .into_iter()
    .enumerate()
    {
        println!(
            "(define-fun bank-{bank} () Bool (and aligned (bvuge address (_ bv{start} 64)) (bvult address (_ bv{stop} 64))))"
        );
        let word = vector(
            outputs[out_first..out_first + word_bits]
                .iter()
                .copied()
                .map(name),
        );
        differences.push(format!("(distinct {} bank-{bank})", name(outputs[bank])));
        differences.push(format!(
            "(and bank-{bank} (distinct ((_ zero_extend {}) {word}) (bvudiv (bvsub address (_ bv{start} 64)) (_ bv8 64))))",
            64 - word_bits,
        ));
    }
    println!("(assert (or {}))", differences.join(" "));
}
