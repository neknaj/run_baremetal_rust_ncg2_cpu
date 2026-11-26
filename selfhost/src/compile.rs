//! 回路ソースを Core で解析し、関数を NOR へ展開して評価する。

use circuitgame2_core::project::{
    SourceSnapshot, parse_circuit_project, prepare_definition_analysis,
};

/// 1入力・1出力の `invert` を実行し、構文ノード数と出力を返す。
#[inline(always)]
pub(super) fn evaluate(source: &str, inputs: u64) -> Result<(usize, bool), u64> {
    let snapshots = [SourceSnapshot {
        path: "request.ncg2",
        revision: 0,
        text: source,
    }];
    let project = parse_circuit_project(&snapshots).map_err(|_| 4_u64)?;
    let prepared = prepare_definition_analysis(&project).map_err(|_| 7_u64)?;
    let checked = prepared.resolve().map_err(|_| 8_u64)?;
    let Some(circuit) = project.circuits().first() else {
        return Err(9);
    };
    let Some(invert) = circuit
        .names
        .iter()
        .find(|entry| entry.name.word == "invert")
    else {
        return Err(9);
    };
    let Some(entry) = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == invert.form)
    else {
        return Err(9);
    };

    let Ok(Some(compiled)) =
        checked
            .aliases()
            .expand_reachable_function((0, entry), 64, 1024, 1024)
    else {
        return Err(10);
    };
    let output = compiled
        .netlist()
        .evaluate(&[inputs & 1 != 0])
        .map_err(|_| 5_u64)?;
    let [bit] = output.as_slice() else {
        return Err(11);
    };
    Ok((circuit.syntax.forms.len(), *bit))
}
