use std::path::{Path, PathBuf};
use std::process::Command;

fn compile(directory: &Path, compiler: &str, source: &str, standard: &str) -> PathBuf {
    let source = directory.join(source);
    let object = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join(format!(
        "{}.o",
        source.file_name().unwrap().to_str().unwrap()
    ));
    let profile = std::env::var("PROFILE").unwrap();
    let mut command = Command::new(compiler);
    command.args([
        "--target=x86_64-unknown-none",
        standard,
        "-march=x86-64-v3",
        "-mno-mmx",
        "-ffreestanding",
        "-fno-stack-protector",
        "-fno-pic",
        "-fno-pie",
        "-mno-red-zone",
        "-ftls-model=local-exec",
        "-fno-unwind-tables",
        "-fno-asynchronous-unwind-tables",
        "-Wall",
        "-Wextra",
        "-Werror",
        if profile == "release" { "-O2" } else { "-O1" },
    ]);
    if compiler == "clang++" {
        command.args(["-fno-exceptions", "-fno-rtti"]);
    }
    command.arg("-I").arg(directory.join("../libsys/include"));
    let status = command
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .status()
        .unwrap_or_else(|error| panic!("Cannot invoke {compiler}: {error}"));
    assert!(status.success(), "ABI peer compilation failed: {source:?}");
    println!("cargo::rerun-if-changed={}", source.display());
    object
}

fn main() {
    let directory = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let linker = directory.join("linker.ld");
    for object in [
        compile(&directory, "clang", "src/peer.c", "-std=c11"),
        compile(&directory, "clang++", "src/peer.cpp", "-std=c++23"),
        compile(&directory, "clang", "src/native.c", "-std=c11"),
    ] {
        println!("cargo::rustc-link-arg={}", object.display());
    }
    println!(
        "cargo::rerun-if-changed={}",
        directory
            .join("../libsys/include/cryolite/native.h")
            .display()
    );
    println!("cargo::rerun-if-changed={}", linker.display());
    println!("cargo::rustc-link-arg=-T{}", linker.display());
    println!("cargo::rustc-link-arg=--no-pie");
    println!("cargo::rustc-link-arg=--no-dynamic-linker");
    println!("cargo::rustc-link-arg=--build-id=none");
    println!("cargo::rustc-link-arg=-zmax-page-size=0x1000");
}
