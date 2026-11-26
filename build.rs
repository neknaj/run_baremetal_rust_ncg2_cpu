//! リンカスクリプトの変更時に起動 ELF をリンクし直す。

fn main() {
    println!("cargo:rerun-if-changed=memory.x");
}
