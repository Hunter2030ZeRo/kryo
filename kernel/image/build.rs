use std::path::PathBuf;

fn main() {
    let linker =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../boot/linker.ld");
    println!("cargo::rerun-if-changed={}", linker.display());
    println!("cargo::rustc-link-arg-bin=kryo=-T{}", linker.display());
}
