use circuitgame2_core::netlist::{ClockedStorageInputs, ClockedVm, NorNetlist, Wire};
use std::{
    io::{self, Write},
    ops::Range,
};

/// 増補入力の先頭に外部入力を、続いてラッチの登録出力を置く借用配線。
pub(crate) struct LatchNetwork<'a> {
    /// Core が展開した NOR と公開出力。
    pub(crate) nor: &'a NorNetlist,
    /// 増補入力のうち、外部ポートのビット数。
    pub(crate) external: usize,
    latches: Vec<(&'a ClockedStorageInputs, Range<usize>)>,
}

impl<'a> LatchNetwork<'a> {
    /// ラッチの出力順と幅を確認し、配線を借用する。
    pub(crate) fn read(vm: &'a ClockedVm) -> Result<Self, &'static str> {
        let nor = vm.netlist();
        let external = vm.external_input_count();
        let mut latches = Vec::with_capacity(vm.storage_count());
        let mut start = external;
        for index in 0..vm.storage_count() {
            let (wiring, range) = vm
                .storage_wiring(index)
                .ok_or("状態素子の配線が見つからない。")?;
            if wiring.address.is_some() {
                return Err("この検証器はラッチ専用で、RAM を展開できない。");
            }
            if range.start != start || range.len() != wiring.data.len() {
                return Err("ラッチ出力の順序または幅が増補入力と一致しない。");
            }
            start = range.end;
            latches.push((wiring, range));
        }
        if start != nor.input_count {
            return Err("外部入力とラッチ出力が増補入力の全域を覆わない。");
        }
        Ok(Self {
            nor,
            external,
            latches,
        })
    }

    /// 更新式では開始時の状態、公開出力では確定後の状態を指定する。
    pub(crate) fn wire(&self, wire: Wire, prefix: &str, frame: usize, state: usize) -> String {
        match wire {
            Wire::Input(index) if index < self.external => format!("i_{frame}_{index}"),
            Wire::Input(index) => format!("{prefix}_q_{state}_{index}"),
            Wire::Gate(index) => format!("{prefix}_g_{state}_{index}"),
            Wire::Constant(value) => value.to_string(),
        }
    }

    /// tick 0 の NOR とラッチを reset 値または任意の値に設定する。
    pub(crate) fn emit_state(
        &self,
        out: &mut impl Write,
        prefix: &str,
        from_reset: bool,
    ) -> io::Result<()> {
        for (kind, indices) in [
            ("g", 0..self.nor.gates.len()),
            ("q", self.external..self.nor.input_count),
        ] {
            for index in indices {
                let symbol = format!("{prefix}_{kind}_0_{index}");
                if from_reset {
                    writeln!(out, "(define-fun {symbol} () Bool false)")?;
                } else {
                    writeln!(out, "(declare-const {symbol} Bool)")?;
                }
            }
        }
        Ok(())
    }

    /// 旧状態と入力から同時に更新する1 tick の方程式を出力する。
    pub(crate) fn emit_step(
        &self,
        out: &mut impl Write,
        prefix: &str,
        frame: usize,
    ) -> io::Result<()> {
        let prev = frame - 1;
        let wire = |source| self.wire(source, prefix, frame, prev);
        for (index, gate) in self.nor.gates.iter().enumerate() {
            writeln!(out, "(declare-const {prefix}_g_{frame}_{index} Bool)")?;
            writeln!(
                out,
                "(assert (= {prefix}_g_{frame}_{index} (not (or {} {}))))",
                wire(gate.left),
                wire(gate.right)
            )?;
        }
        for (wiring, range) in &self.latches {
            let enable = wire(wiring.enable);
            for (index, &data) in range.clone().zip(&wiring.data) {
                writeln!(out, "(declare-const {prefix}_q_{frame}_{index} Bool)")?;
                writeln!(
                    out,
                    "(assert (= {prefix}_q_{frame}_{index} (ite {enable} {} {prefix}_q_{prev}_{index})))",
                    wire(data)
                )?;
            }
        }
        Ok(())
    }
}
