// Copyright (c) 2026 Wojciech Stach
// Licensed under BSL 1.1

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let version_file = manifest_dir.join("../fod_version.txt");
    println!("cargo:rerun-if-changed={}", version_file.display());
    let raw_version = fs::read_to_string(&version_file)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", version_file.display()));
    let version = raw_version.trim();
    if version.is_empty() {
        panic!("fod_version.txt is empty");
    }
    println!("cargo:rustc-env=FOD_VERSION_LABEL=FOD {version}");
}
