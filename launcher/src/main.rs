//! Stands in for the makeWrapper scripts of the relinked bundle. A copy of this static
//! program replaces each wrapper, applies that wrapper's environment relative to the tree
//! it runs from, and runs the wrapped program with the same arguments, argv[0] included.

use std::{
    env,
    ffi::{OsStr, OsString},
    io,
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    process::Command,
};

/// One makeWrapper script. Every path is a store path, which the launcher prefixes with
/// the tree root.
struct Wrapper {
    /// The script this copy replaces.
    program: &'static str,
    /// wrapProgram's renamed original.
    target: &'static str,
    set: &'static [(&'static str, &'static str)],
    set_default: &'static [(&'static str, &'static str)],
    prefix: &'static [(&'static str, &'static [&'static str])],
}

// nix/bundle-relink.nix renders `const WRAPPERS: &[Wrapper]` from the wrapper environments
// that flake.nix also hands to makeWrapper.
include!(env!("EON_LAUNCHER_TABLE"));

/// makeWrapper's `--prefix NAME : path`: drop the first element equal to `path`, then put
/// `path` first. An empty value counts as unset.
fn prefix(path: &[u8], current: &OsStr) -> OsString {
    let mut elements: Vec<&[u8]> = Vec::new();
    if !current.is_empty() {
        elements.extend(current.as_bytes().split(|byte| *byte == b':'));
        if let Some(index) = elements.iter().position(|element| *element == path) {
            elements.remove(index);
        }
    }
    elements.insert(0, path);
    OsString::from_vec(elements.join(&b':'))
}

fn launch() -> io::Result<()> {
    let executable = env::current_exe()?.into_os_string().into_vec();
    let (root, wrapper) = WRAPPERS
        .iter()
        .find_map(|wrapper| {
            let root = executable.strip_suffix(wrapper.program.as_bytes())?;
            Some((root, wrapper))
        })
        .ok_or_else(|| {
            io::Error::other(format!(
                "{} is not a bundle wrapper",
                String::from_utf8_lossy(&executable)
            ))
        })?;
    let relocate = |path: &str| [root, path.as_bytes()].concat();
    let mut arguments = env::args_os();
    let mut command = Command::new(OsString::from_vec(relocate(wrapper.target)));
    if let Some(argv0) = arguments.next() {
        command.arg0(argv0);
    }
    command.args(arguments);
    for (name, path) in wrapper.set {
        command.env(name, OsString::from_vec(relocate(path)));
    }
    for (name, path) in wrapper.set_default {
        if env::var_os(name).is_none() {
            command.env(name, OsString::from_vec(relocate(path)));
        }
    }
    for (name, paths) in wrapper.prefix {
        // makeWrapper prepends a list's elements last to first, so the list keeps its order.
        let value = paths
            .iter()
            .rev()
            .fold(env::var_os(name).unwrap_or_default(), |value, path| {
                prefix(&relocate(path), &value)
            });
        command.env(name, value);
    }
    Err(command.exec())
}

fn main() {
    if let Err(error) = launch() {
        eprintln!("eon bundle: {error}");
        std::process::exit(1);
    }
}
