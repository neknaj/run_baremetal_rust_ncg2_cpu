//! 実行するセルフホストプログラムのメモリ配置を選ぶ。

fn main() {
    let root = env!("CARGO_MANIFEST_DIR");
    let script = if cfg!(feature = "compile-circuit") {
        "memory-compile.x"
    } else {
        "memory.x"
    };
    println!("cargo:rerun-if-changed={script}");
    println!("cargo:rustc-link-arg=-T{root}/{script}");
}
