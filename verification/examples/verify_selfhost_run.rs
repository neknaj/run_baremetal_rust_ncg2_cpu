//! 展開したセルフホスト用回路を実行し、QEMU の PC 遷移と RAM 結果を照合する。

use std::{error::Error, fs, path::Path};

use circuitgame2_core::{
    netlist::ClockedVm,
    project::{
        ClockedModuleVmLimits, ModuleBodyPosition, NameKey, SourceSnapshot, check_use_aliases,
        index_use_targets, parse_circuit_project,
    },
};
use ncg2_rv64_arithmetic_proof::event_clocked::EventVm;

const ENTRY: u64 = 0x10000;
const HALT: u64 = 0x10068;
const INFORMATION: u64 = 0x07e0_0113;
const CROSS_CHECK_TICKS: u64 = 14_500;

fn machine(path: &Path) -> Result<ClockedVm, Box<dyn Error>> {
    let config = circuitgame2_project_config::parse(&fs::read(path)?)
        .map_err(|error| format!("実行設定: {error:?}"))?;
    if config.entry.as_ref().map(|name| name.get_ref().as_str()) != Some("::rv64::firmware-demo")
        || !config.device.is_empty()
        || !config.vram.is_empty()
    {
        return Err("外部デバイスや VRAM 設定のない ::rv64::firmware-demo を選ぶ。".into());
    }
    let directory = path.parent().unwrap_or(Path::new("."));
    let paths = config.sources.get_ref();
    let texts = paths
        .iter()
        .map(|path| fs::read_to_string(directory.join(path.get_ref())))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshots = paths
        .iter()
        .zip(&texts)
        .map(|(path, text)| SourceSnapshot {
            path: path.get_ref(),
            revision: 0,
            text,
        })
        .collect::<Vec<_>>();
    let project = parse_circuit_project(&snapshots).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let key = NameKey {
        parent: Some("rv64"),
        name: "firmware-demo",
    };
    let (source, declaration) = targets
        .exports()
        .declarations()
        .declaration(key)
        .ok_or("実行入口の宣言が見つからない。")?;
    let file = project
        .sources()
        .iter()
        .position(|candidate| candidate.id.same_file(&source.id))
        .ok_or("実行入口のソースが見つからない。")?;
    let entry = project.circuits()[file]
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == declaration.form)
        .ok_or("実行入口の索引が見つからない。")?;
    let built = aliases
        .lower_clocked_vm(
            ModuleBodyPosition { file, entry },
            ClockedModuleVmLimits {
                width: 65_536,
                wiring_bits: 1_048_576,
                gates: 1_048_576,
                state_bits: 1_048_576,
            },
        )
        .map_err(|error| format!("{error:?}"))?
        .ok_or("実行入口を変換できない。")?;
    if built.inputs.total_bits() != 0 || built.outputs.total_bits() != 194 {
        return Err("検証する実行入口の入出力幅が異なる。".into());
    }
    Ok(built.vm)
}

fn reference_pcs(path: &Path) -> Result<Vec<u64>, Box<dyn Error>> {
    let log = fs::read_to_string(path)?;
    let mut pcs = Vec::new();
    for line in log.lines().filter(|line| line.starts_with("Trace ")) {
        let pc = line
            .split_once(" [")
            .and_then(|(_, state)| state.split('/').nth(1))
            .ok_or("QEMU の実行列から PC を読めない。")?;
        pcs.push(u64::from_str_radix(pc, 16)?);
    }
    if pcs.first() != Some(&ENTRY) {
        return Err("QEMU の実行列が起動アドレスから始まっていない。".into());
    }
    if pcs.last() != Some(&HALT) {
        pcs.push(HALT);
    }
    pcs.dedup();
    Ok(pcs)
}

fn word(bits: &[bool]) -> u64 {
    bits.iter()
        .enumerate()
        .fold(0, |word, (bit, &high)| word | (u64::from(high) << bit))
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let (project, trace, ticks, use_event) = match args.as_slice() {
        [project, trace, ticks] => (project, trace, ticks, false),
        [project, trace, ticks, option] if option == "--event" => (project, trace, ticks, true),
        _ => return Err("実行設定、QEMU の実行列、上限 tick 数を指定する。疎実行を使う場合は末尾に --event を付ける。".into()),
    };
    let limit = ticks
        .to_str()
        .ok_or("上限 tick 数は十進整数で指定する。")?
        .parse::<u64>()?;
    let reference = reference_pcs(Path::new(trace))?;
    let mut vm = machine(Path::new(project))?;
    let mut event = if use_event {
        Some(EventVm::new(&vm)?)
    } else {
        None
    };
    let mut next = 1;
    let mut previous_pc = ENTRY;
    let reset = vm.reset();
    if let Some(event) = event.as_mut() {
        if event.reset() != reset {
            return Err("標準 VM と疎実行で reset の出力が異なる。".into());
        }
    }
    for tick in 1..=limit {
        let standard_output;
        let output = if let Some(event) = event.as_mut() {
            let output = event
                .step(&[])
                .map_err(|error| format!("疎実行 tick {tick}: {error:?}"))?;
            if tick <= CROSS_CHECK_TICKS {
                let reference = vm
                    .step(&[])
                    .map_err(|error| format!("標準 VM tick {tick}: {error:?}"))?;
                if output != reference {
                    return Err(format!("tick {tick} で標準 VM と疎実行の出力が異なる。").into());
                }
                if tick == CROSS_CHECK_TICKS {
                    eprintln!("標準 VM と疎実行の先頭 {CROSS_CHECK_TICKS} tick が一致した。");
                }
            }
            output
        } else {
            standard_output = vm
                .step(&[])
                .map_err(|error| format!("標準 VM tick {tick}: {error:?}"))?;
            &standard_output
        };
        let pc = ENTRY.wrapping_add(word(&output[..64]));
        if output[193] {
            return Err(format!("tick {tick}、PC {pc:#x} で回路が停止した。").into());
        }
        if pc != previous_pc {
            if reference.get(next) != Some(&pc) {
                return Err(format!(
                    "tick {tick} の PC が異なる。実際 {pc:#x}、参照 {:?}。",
                    reference.get(next)
                )
                .into());
            }
            previous_pc = pc;
            next += 1;
        }
        if tick % 100_000 == 0 {
            eprintln!(
                "tick {tick}、PC {pc:#x}、PC 遷移 {}/{}",
                next - 1,
                reference.len() - 1
            );
        }
        if output[192] && pc == HALT {
            let status = word(&output[64..128]);
            let information = word(&output[128..192]);
            if next != reference.len() || status != 1 || information != INFORMATION {
                return Err(format!("終了時の結果が異なる。status={status:#x} information={information:#x} PC 遷移 {}件。", next - 1).into());
            }
            println!(
                "tick {tick} で一致した。status={status:#x} information={information:#x} PC 遷移 {}件。",
                next - 1
            );
            return Ok(());
        }
    }
    Err(format!(
        "{limit} tick で上限に達した。PC {previous_pc:#x}、一致した PC 遷移 {}件。",
        next - 1
    )
    .into())
}
