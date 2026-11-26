//! 展開した ROM の全添字でファイル由来の値と一致することを検査する。

use circuitgame2_core::project::{
    SourceSnapshot, check_use_aliases, index_use_targets, parse_circuit_project,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [source_path, image_path] = args.as_slice() else {
        return Err("ROM の NCG2 ソースと、ELF から変換したバイナリイメージを指定する。".into());
    };
    let source = std::fs::read_to_string(source_path)?;
    let image = std::fs::read(image_path)?;
    let snapshots = [
        SourceSnapshot {
            path: "basis.ncg2",
            revision: 0,
            text: "(using nor)",
        },
        SourceSnapshot {
            path: "rom.ncg2",
            revision: 0,
            text: &source,
        },
    ];
    let project = parse_circuit_project(&snapshots).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let circuit = &project.circuits()[1];
    let declaration = circuit
        .names
        .iter()
        .find(|name| name.name.word == "firmware-rom")
        .ok_or("firmware-rom が見つからない。")?;
    let entry = circuit
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == declaration.form)
        .ok_or("firmware-rom の索引が見つからない。")?;
    let compiled = aliases
        .expand_reachable_function((1, entry), 65_536, 1_048_576, 1_048_576)
        .map_err(|error| format!("{error:?}"))?
        .ok_or("firmware-rom を展開できない。")?;
    let rom = compiled.netlist();
    let width = rom.input_count;
    if !(1..=14).contains(&width) || rom.outputs.len() != 64 {
        return Err("ROM の入力幅は1〜14、出力幅は64である必要がある。".into());
    }
    let count = 1 << width;
    let size = count * 8;
    if image.len() != 128 * 1024 || image[size..].iter().any(|&byte| byte != 0) {
        return Err("イメージは128 KiBであり、ROM より後ろに非ゼロの初期値を含められない。".into());
    }

    // 添字の全ビット列を列挙するため、すべての64ビット語を照合できる。
    for (index, chunk) in image[..size].chunks_exact(8).enumerate() {
        let input: [bool; 14] = std::array::from_fn(|i| index & (1 << i) != 0);
        let output = rom
            .evaluate(&input[..width])
            .map_err(|error| format!("{error:?}"))?;
        let actual = output
            .iter()
            .enumerate()
            .fold(0, |word, (bit, &state)| word | (u64::from(state) << bit));
        let expected = u64::from_le_bytes(chunk.try_into()?);
        if actual != expected {
            return Err(format!(
                "語 {index} の値が異なる (ROM {actual:#018x}, ELF {expected:#018x})。"
            )
            .into());
        }
    }
    println!(
        "{count} 語の全入力が一致した。NOR は{}個。",
        rom.gates.len()
    );
    Ok(())
}
