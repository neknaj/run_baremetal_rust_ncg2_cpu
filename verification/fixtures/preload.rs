//! 起動前処理を実 ROM の代わりに固定テスト用 ROM で組み立てる。

pub const SOURCES: &[(&str, &str)] = &[
    ("decode.ncg2", include_str!("../../circuit/decode.ncg2")),
    ("boot-rom.ncg2", include_str!("../../circuit/boot-rom.ncg2")),
    (
        "fetch-boot.ncg2",
        include_str!("../../circuit/fetch-boot.ncg2"),
    ),
    (
        "arithmetic.ncg2",
        include_str!("../../circuit/arithmetic.ncg2"),
    ),
    (
        "program-counter.ncg2",
        include_str!("../../circuit/program-counter.ncg2"),
    ),
    (
        "register-file.ncg2",
        include_str!("../../circuit/register-file.ncg2"),
    ),
    (
        "execute-boot.ncg2",
        include_str!("../../circuit/execute-boot.ncg2"),
    ),
    (
        "boot-machine.ncg2",
        include_str!("../../circuit/boot-machine.ncg2"),
    ),
    (
        "execute-control.ncg2",
        include_str!("../../circuit/execute-control.ncg2"),
    ),
    ("firmware-rom.ncg2", include_str!("selfhost-rom.ncg2")),
    (
        "selfhost-memory.ncg2",
        include_str!("../../circuit/selfhost-memory.ncg2"),
    ),
    (
        "selfhost-preload.ncg2",
        include_str!("../../circuit/selfhost-preload.ncg2"),
    ),
];
