//! 起動 ELF にこのパッケージのメモリ配置を与える。

fn main() {
    let root = env!("CARGO_MANIFEST_DIR");
    println!("cargo:rerun-if-changed=memory.x");
    println!("cargo:rustc-link-arg=-T{root}/memory.x");
}
