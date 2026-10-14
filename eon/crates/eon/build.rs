//! Digests every file in the source tree this build compiles from. Eon supplies
//! the digest as its assembly contribution to runtime generation identity, so a
//! change to any in-tree component selects a different generation.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = fs::canonicalize(manifest.join("../../..")).unwrap();
    let mut files = Vec::new();
    let mut watched = Vec::new();
    collect(&root, &root, &mut files, &mut watched);
    files.sort();

    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;
    let mut hash = OFFSET;
    for relative in &files {
        let path = root.join(relative);
        println!("cargo::rerun-if-changed={}", path.display());
        let contents = fs::read(&path).unwrap();
        for input in [relative.as_bytes(), &contents] {
            for byte in (input.len() as u64).to_le_bytes().iter().chain(input) {
                hash ^= u128::from(*byte);
                hash = hash.wrapping_mul(PRIME);
            }
        }
    }
    // Cargo watches a directory recursively, so files added there rerun this
    // script. Directories holding build output are covered file by file.
    for directory in watched {
        println!("cargo::rerun-if-changed={}", directory.display());
    }
    println!("cargo::rustc-env=EON_WORKSPACE_DIGEST={hash:032x}");
}

// Dot entries hold tool state, `target` holds build output, and symlinks and
// special files are not sources. Returns whether Cargo can watch the directory
// recursively: it is not the root and holds no `target`.
fn collect(
    root: &Path,
    directory: &Path,
    files: &mut Vec<String>,
    watched: &mut Vec<PathBuf>,
) -> bool {
    let mut clean = directory != root;
    let mut children = Vec::new();
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let name = entry.file_name();
        let name = name.to_str().expect("workspace paths are UTF-8");
        if kind.is_dir() && name == "target" {
            clean = false;
            continue;
        }
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            if collect(root, &path, files, watched) {
                children.push(path);
            } else {
                clean = false;
            }
        } else if kind.is_file() {
            let relative = path.strip_prefix(root).unwrap().to_str().unwrap();
            files.push(relative.into());
        }
    }
    if !clean {
        watched.extend(children);
    }
    clean
}
