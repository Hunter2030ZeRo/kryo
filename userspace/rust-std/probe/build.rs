use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=CRYOLITE_STD_PROVIDER");
    let provider = PathBuf::from(
        env::var_os("CRYOLITE_STD_PROVIDER").expect("build this probe through tools/build_std.py"),
    );
    for relative in ["sysroot/usr/lib/libc.a", "meson/sysdeps/cryolite/crt1.o"] {
        println!(
            "cargo:rerun-if-changed={}",
            provider.join(relative).display()
        );
    }
    println!("cargo:rerun-if-changed=../../libc/probe/linker.ld");
}
