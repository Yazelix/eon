//! Builds the Bend worker (docs/HOST.md): bun runs the pinned Bend compiler
//! from `BEND_SOURCE_DIR`, which turns src/main.bend into C, and the clang
//! at `BEND_CLANG` compiles that C. The host runs the worker from its package's libexec, or
//! else this build's copy, at `ORBIT_BUILT_WORKER` (src/main.rs).

use std::{env, fs, path::PathBuf, process::Command};

fn run(command: &mut Command) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("{command:?}: {error}"));
    assert!(status.success(), "{command:?}: {status}");
}

fn main() {
    println!("cargo::rerun-if-env-changed=BEND_SOURCE_DIR");
    println!("cargo::rerun-if-env-changed=BEND_CLANG");
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("src");
    for entry in fs::read_dir(&source).expect("read src") {
        let path = entry.expect("read src").path();
        if path
            .extension()
            .is_some_and(|extension| extension == "bend")
        {
            println!("cargo::rerun-if-changed={}", path.display());
        }
    }
    let bend = PathBuf::from(
        env::var_os("BEND_SOURCE_DIR")
            .expect("BEND_SOURCE_DIR must name the pinned Bend source, as the dev shell sets it"),
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let c = out.join("yazelix-orbit-worker.c");
    let worker = out.join("yazelix-orbit-worker");
    run(Command::new("bun")
        .env("BEND_NO_TELEMETRY", "1")
        .arg(bend.join("bend2/main.ts"))
        .arg("main.bend")
        .arg("-o")
        .arg(&c)
        .current_dir(&source));
    let mut clang = Command::new(
        env::var_os("BEND_CLANG").expect("BEND_CLANG must name clang, as the dev shell sets it"),
    );
    clang.args(["-std=c11", "-O2"]);
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        clang.arg("-Wl,-z,noexecstack");
    }
    run(clang.arg(&c).args(["-lpthread", "-lm", "-o"]).arg(&worker));
    println!("cargo::rustc-env=ORBIT_BUILT_WORKER={}", worker.display());
}
