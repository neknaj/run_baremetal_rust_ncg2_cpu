//! UART ブート検証で揃えるプロジェクト、バス観測、送信波形。

use std::{error::Error, fs, path::Path};

use circuitgame2_core::{
    netlist::{ClockedVm, PortBitLayout},
    project::{
        ClockedModuleVmLimits, ModuleBodyPosition, NameKey, SourceSnapshot, check_use_aliases,
        index_use_targets, parse_circuit_project,
    },
    syntax::{
        Form,
        circuit::{PortSite, routine_ports},
    },
};

/// UART 波形の1ビットを保持する tick 数。
pub const BIT_TICKS: usize = 16;

/// プロジェクトの公開ポートを照合し、検証に使う VM を組み立てる。
pub fn machine(path: &Path) -> Result<ClockedVm, Box<dyn Error>> {
    let config = circuitgame2_project_config::parse(&fs::read(path)?)
        .map_err(|error| format!("実行設定: {error:?}"))?;
    if config.entry.as_ref().map(|entry| entry.get_ref().as_str()) != Some("::rv64::uart-system")
        || !config.device.is_empty()
        || !config.vram.is_empty()
    {
        return Err("外部デバイスと VRAM 設定のない ::rv64::uart-system を指定する。".into());
    }
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let paths = config.sources.get_ref();
    let texts = paths
        .iter()
        .map(|path| fs::read_to_string(directory.join(path.get_ref())))
        .collect::<Result<Vec<_>, _>>()?;
    let sources = paths
        .iter()
        .zip(&texts)
        .map(|(path, text)| SourceSnapshot {
            path: path.get_ref(),
            text,
            revision: 0,
        })
        .collect::<Vec<_>>();
    let project = parse_circuit_project(&sources).map_err(|error| format!("{error:?}"))?;
    let targets = index_use_targets(&project).map_err(|error| format!("{error:?}"))?;
    let aliases = check_use_aliases(&targets).map_err(|error| format!("{error:?}"))?;
    let (source, declaration) = targets
        .exports()
        .declarations()
        .declaration(NameKey {
            parent: Some("rv64"),
            name: "uart-system",
        })
        .ok_or("UART の実行入口が見つからない。")?;
    let file = project
        .sources()
        .iter()
        .position(|candidate| candidate.id.same_file(&source.id))
        .ok_or("UART の実行入口を含むソースが見つからない。")?;
    let entry = project.circuits()[file]
        .syntax
        .entries
        .iter()
        .position(|entry| entry.form == declaration.form)
        .ok_or("UART の実行入口を指す索引が見つからない。")?;
    let compiled = aliases
        .lower_clocked_vm(
            ModuleBodyPosition { file, entry },
            ClockedModuleVmLimits {
                width: 65_536,
                wiring_bits: 1_048_576,
                gates: 1_048_576,
                state_bits: 8_388_608,
            },
        )
        .map_err(|error| format!("{error:?}"))?
        .ok_or("UART の実行入口を展開できない。")?;
    let syntax = &project.circuits()[file].syntax;
    let text = source.text();
    let sites = routine_ports(text, syntax, entry).ok_or("UART の公開ポートを取得できない。")?;
    let matches = |ports: &[PortSite], layout: &PortBitLayout, expected: &[(&str, usize)]| {
        ports.len() == expected.len()
            && layout.ranges().len() == expected.len()
            && ports.iter().zip(layout.ranges()).zip(expected).all(
                |((site, range), &(name, width))| {
                    let Some(Form::Atom { span, .. }) = syntax.forms.get(site.name) else {
                        return false;
                    };
                    text.get(span.clone()) == Some(name) && range.len() == width
                },
            )
    };
    if !matches(
        &sites.inputs,
        &compiled.inputs,
        &[("rx", 1), ("probe-address", 64)],
    ) || !matches(
        &sites.outputs,
        &compiled.outputs,
        &[
            ("pc", 64),
            ("bus-address", 64),
            ("bus-read-data", 64),
            ("uart-ready", 1),
            ("uart-error", 1),
            ("commit", 1),
            ("stopped", 1),
            ("bus-write", 1),
        ],
    ) {
        return Err("UART 回路の公開ポート名、順序、幅が検証対象と異なる。".into());
    }
    Ok(compiled.vm)
}

/// 64ビット以下の列を最下位ビットから読み取る。
pub fn word(bits: &[bool]) -> u64 {
    bits.iter().enumerate().fold(0, |value, (offset, &bit)| {
        value | (u64::from(bit) << offset)
    })
}

/// 8N1 の開始、データ、停止ビットを物理 RX の tick に広げる。
pub fn waveform(bytes: [u8; 8]) -> impl Iterator<Item = bool> {
    bytes.into_iter().flat_map(|byte| {
        std::iter::once(false)
            .chain((0..8).map(move |bit| (byte >> bit) & 1 != 0))
            .chain(std::iter::once(true))
            .flat_map(|level| std::iter::repeat_n(level, BIT_TICKS))
    })
}

/// 波形は1 tick ずつ送り、ACK 待ちでは同じ有効フラグが続く区間をまとめる。
///
/// コールバックは有効フラグと実行した tick 数を返す。tick 数は1から渡した
/// 上限までで、その区間の各 tick で有効フラグが同じであることを呼び出し側が確認する。
/// 前に観測した有効フラグが true で、その後 false になった場合は、その tick だけ進める。
///
/// # Errors
///
/// コールバックのエラーをそのまま返す。
///
/// # Panics
///
/// コールバックが上限外の tick 数を返した場合や、消費を検出した呼び出しで
/// 2 tick 以上進めた場合にパニックする。
#[allow(dead_code)]
pub fn send_with_idle<E>(
    bytes: [u8; 8],
    mut tick: impl FnMut(bool, usize) -> Result<(bool, usize), E>,
) -> Result<bool, E> {
    let mut ready = false;
    for level in waveform(bytes) {
        let (latest, span) = tick(level, 1)?;
        assert_eq!(span, 1);
        ready |= latest;
    }
    const ACK_TICKS: usize = 20_000;
    let mut remaining = ACK_TICKS;
    while remaining != 0 {
        // 波形中に有効フラグが立って下がった場合も、最初の待ち tick で消費を検出する。
        let limit = if remaining == ACK_TICKS { 1 } else { remaining };
        let (latest, span) = tick(true, limit)?;
        assert!((1..=limit).contains(&span));
        if ready && !latest {
            assert_eq!(span, 1);
            return Ok(true);
        }
        ready |= latest;
        remaining -= span;
    }
    Ok(false)
}
