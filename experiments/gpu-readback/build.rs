use std::{env, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let rustc = env::var_os("RUSTC").expect("Cargo supplies RUSTC");
    let version = Command::new(rustc)
        .arg("--version")
        .output()
        .expect("query build compiler");
    assert!(version.status.success(), "rustc --version failed");
    let version = String::from_utf8(version.stdout).expect("UTF-8 rustc version");
    println!("cargo:rustc-env=B1_RUSTC={}", version.trim());
}
