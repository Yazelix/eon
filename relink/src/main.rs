//! Copies a Nix closure into a tree that runs on the host glibc without a namespace, then
//! gates the result. Build-time tooling: patchelf and readelf run as subprocesses.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env,
    ffi::OsString,
    fs,
    io::{self, Read},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{PermissionsExt, symlink},
    },
    path::{Component, Path, PathBuf},
    process::{Command, ExitCode},
};

const STORE: &str = "/nix/store/";
/// The x86-64 psABI program interpreter every glibc distribution ships.
const INTERPRETER: &str = "/lib64/ld-linux-x86-64.so.2";
const HELP: &str = "Usage: eon-relink --registration FILE --root PATH [--exclude PATH]...
       --glibc-floor VERSION [--host-library SONAME]... [--host-runtime FILE]...
       [--substitute ORIGINAL=REPLACEMENT]... OUT

Copies every store path reachable from ROOT without passing through an excluded path
into OUT/nix/store, relinks it for the host glibc and checks the result. Each host-runtime
ELF FILE loses its RUNPATH, so it takes every library it needs from the host. Each
substitute ships REPLACEMENT, a rebuild with the same name, as ORIGINAL, with its own
store name in file contents and symlink targets renamed to ORIGINAL's.";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

/// Replaces each occurrence of `from` in `bytes` with `to`, which has the same length.
fn rename(bytes: &mut [u8], from: &[u8], to: &[u8]) -> bool {
    let mut renamed = false;
    let mut start = 0;
    while let Some(offset) = bytes[start..]
        .windows(from.len())
        .position(|window| window == from)
    {
        start += offset;
        bytes[start..start + from.len()].copy_from_slice(to);
        start += from.len();
        renamed = true;
    }
    renamed
}

/// Splits a closureInfo registration into the paths that ship (reachable from `root`
/// without passing through `exclude`) and the number pruned with the exclusions.
fn closure<'a>(
    registration: &'a str,
    root: &str,
    exclude: &[String],
) -> io::Result<(Vec<&'a str>, usize)> {
    let mut references = BTreeMap::new();
    let mut lines = registration.lines();
    while let Some(path) = lines.next() {
        // Path, NAR hash, NAR size, deriver, reference count, references.
        let count = lines
            .nth(3)
            .and_then(|count| count.parse().ok())
            .ok_or_else(|| invalid(format!("malformed registration at {path}")))?;
        let referenced: Vec<&str> = lines.by_ref().take(count).collect();
        if referenced.len() != count {
            return Err(invalid(format!("malformed registration at {path}")));
        }
        references.insert(path, referenced);
    }
    if let Some(path) = exclude
        .iter()
        .find(|path| !references.contains_key(path.as_str()))
    {
        return Err(invalid(format!(
            "excluded path is not in the closure: {path}"
        )));
    }
    let excluded = |path: &str| exclude.iter().any(|excluded| excluded == path);
    let (root, _) = references
        .get_key_value(root)
        .ok_or_else(|| invalid(format!("root is not in the closure: {root}")))?;
    let mut kept = HashSet::new();
    let mut pending = vec![*root];
    while let Some(path) = pending.pop() {
        if !excluded(path) && kept.insert(path) {
            let referenced = references.get(path);
            pending
                .extend(referenced.ok_or_else(|| invalid(format!("{path} has no registration")))?);
        }
    }
    let pruned = references
        .keys()
        .filter(|path| !kept.contains(*path) && !excluded(path))
        .count();
    let mut kept: Vec<_> = kept.into_iter().collect();
    kept.sort_unstable();
    Ok((kept, pruned))
}

#[derive(Default)]
struct Elf {
    library: bool,
    interpreter: Option<String>,
    runpath: Vec<String>,
    rpath: bool,
    needed: Vec<String>,
    glibc: Vec<String>,
    /// The dynamic section declares version needs that readelf did not list, for
    /// example because the section headers are stripped.
    unlisted_needs: bool,
}

impl Elf {
    fn read(path: &Path) -> io::Result<Self> {
        // The parser matches readelf's untranslated messages.
        let output = Command::new("readelf")
            .env("LC_ALL", "C")
            .arg("-ldVW")
            .arg(path)
            .output()?;
        if !output.status.success() {
            return Err(invalid(format!(
                "readelf failed on {}: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(Self::parse(&String::from_utf8_lossy(&output.stdout)))
    }

    fn parse(text: &str) -> Self {
        fn bracketed<'a>(line: &'a str, tag: &str) -> Option<&'a str> {
            let value = line.split_once(tag)?.1;
            value.get(value.find('[')? + 1..value.rfind(']')?)
        }
        let mut elf = Self::default();
        let (mut needs, mut declared, mut listed) = (false, false, false);
        for line in text.lines() {
            if line.starts_with("Elf file type is DYN (Shared object file)") {
                elf.library = true;
            } else if let Some((_, interpreter)) =
                line.split_once("[Requesting program interpreter: ")
            {
                elf.interpreter = Some(interpreter.trim_end().trim_end_matches(']').into());
            } else if let Some(library) = bracketed(line, "(NEEDED)") {
                elf.needed.push(library.into());
            } else if let Some(runpath) = bracketed(line, "(RUNPATH)") {
                // glibc ignores an empty RUNPATH, but an empty entry in a nonempty one
                // searches the working directory, so it is kept for the gate.
                if !runpath.is_empty() {
                    elf.runpath = runpath.split(':').map(Into::into).collect();
                }
            } else if line.contains("(RPATH)") {
                elf.rpath = true;
            } else if line.contains("(VERNEEDNUM)") {
                declared = true;
            } else if line.starts_with("Version") {
                needs = line.starts_with("Version needs section");
                listed |= needs;
            } else if needs
                && let Some((_, rest)) = line.split_once("Name: ")
                && let Some(name) = rest.split_whitespace().next()
                && name.starts_with("GLIBC_")
            {
                elf.glibc.push(name.into());
            }
        }
        elf.unlisted_needs = declared && !listed;
        elf
    }
}

/// Splits an absolute store reference into its store path name and the rest.
fn store_name(reference: &str) -> Option<(&str, &str)> {
    let reference = reference.strip_prefix(STORE)?;
    Some(reference.split_at(reference.find('/').unwrap_or(reference.len())))
}

#[derive(Default)]
struct Relink<'a> {
    kept: HashSet<&'a str>,
    excluded: HashSet<&'a str>,
    /// Host-runtime ELF files not yet copied.
    host_runtime: HashSet<PathBuf>,
    /// While a substitute is copied, its store name and the name it ships under.
    rename: Option<(&'a str, &'a str)>,
    elves: Vec<PathBuf>,
    links: Vec<PathBuf>,
    relative: usize,
    interpreters: usize,
    runpaths: usize,
    dropped: usize,
}

impl Relink<'_> {
    /// The reference relative to a directory `depth` levels below the store, if the tree
    /// ships its store path.
    fn relative(&self, reference: &str, depth: usize) -> io::Result<String> {
        match store_name(reference) {
            Some((name, rest)) if self.kept.contains(name) => {
                Ok(format!("{}{name}{rest}", "../".repeat(depth)))
            }
            _ => Err(invalid(format!("the tree does not ship {reference}"))),
        }
    }

    /// Copies `from` to `to`, which sits in a directory `depth` levels below the store.
    fn copy(&mut self, from: &Path, to: &Path, depth: usize) -> io::Result<()> {
        let metadata = fs::symlink_metadata(from)?;
        if metadata.is_dir() {
            fs::create_dir(to)?;
            for entry in fs::read_dir(from)? {
                let entry = entry?;
                self.copy(&entry.path(), &to.join(entry.file_name()), depth + 1)?;
            }
        } else if metadata.is_symlink() {
            let mut target = fs::read_link(from)?.into_os_string().into_vec();
            if let Some((replacement, original)) = self.rename {
                rename(&mut target, replacement.as_bytes(), original.as_bytes());
            }
            let target = PathBuf::from(OsString::from_vec(target));
            if target.as_os_str().as_bytes().starts_with(STORE.as_bytes()) {
                let relative = target
                    .to_str()
                    .ok_or_else(|| invalid("the target is not UTF-8"))
                    .and_then(|target| self.relative(target, depth))
                    .map_err(|error| invalid(format!("{}: {error}", from.display())))?;
                symlink(relative, to)?;
                self.relative += 1;
            } else {
                symlink(target, to)?;
            }
            self.links.push(to.into());
        } else {
            fs::copy(from, to)?;
            let writable = metadata.permissions().mode() | 0o200;
            if let Some((replacement, original)) = self.rename {
                let mut bytes = fs::read(to)?;
                if rename(&mut bytes, replacement.as_bytes(), original.as_bytes()) {
                    fs::set_permissions(to, fs::Permissions::from_mode(writable))?;
                    fs::write(to, bytes)?;
                }
            }
            let mut magic = [0; 4];
            if fs::File::open(to)?.read(&mut magic)? == 4 && magic == *b"\x7fELF" {
                fs::set_permissions(to, fs::Permissions::from_mode(writable))?;
                let host_runtime = self.host_runtime.remove(from);
                self.patch(to, depth, host_runtime)
                    .map_err(|error| invalid(format!("{}: {error}", from.display())))?;
                self.elves.push(to.into());
            }
        }
        Ok(())
    }

    fn patch(&mut self, file: &Path, depth: usize, host_runtime: bool) -> io::Result<()> {
        let elf = Elf::read(file)?;
        let mut arguments = Vec::new();
        // The host's loader replaces one from an excluded path; any other interpreter
        // stays for the gate to report.
        let interpreter = elf.interpreter.as_deref().and_then(store_name);
        if interpreter.is_some_and(|(name, _)| self.excluded.contains(name)) {
            arguments.extend(["--set-interpreter".into(), INTERPRETER.into()]);
            self.interpreters += 1;
        }
        if host_runtime {
            // The gate then requires every library it needs to be a host library.
            if !elf.runpath.is_empty() {
                arguments.push("--remove-rpath".into());
                self.runpaths += 1;
            }
        } else if elf.runpath.iter().any(|entry| entry.starts_with(STORE)) {
            let mut runpath = Vec::new();
            for entry in elf.runpath {
                if !entry.starts_with(STORE) {
                    runpath.push(entry);
                } else if store_name(&entry).is_some_and(|(name, _)| self.excluded.contains(name)) {
                    // The host supplies what the excluded paths held.
                    self.dropped += 1;
                } else {
                    runpath.push(format!("$ORIGIN/{}", self.relative(&entry, depth)?));
                }
            }
            if runpath.is_empty() {
                arguments.push("--remove-rpath".into());
            } else {
                arguments.extend(["--set-rpath".into(), runpath.join(":")]);
            }
            self.runpaths += 1;
        }
        if arguments.is_empty() {
            return Ok(());
        }
        let output = Command::new("patchelf")
            .args(arguments)
            .arg(file)
            .output()?;
        if !output.status.success() {
            return Err(invalid(format!(
                "patchelf failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(())
    }
}

struct Gates {
    floor: Vec<u32>,
    host: HashSet<String>,
}

#[derive(Default)]
struct Report {
    violations: Vec<(&'static str, String)>,
    glibc_needs: usize,
    in_tree: usize,
    on_host: usize,
}

fn version(text: &str) -> Option<Vec<u32>> {
    text.split('.').map(|part| part.parse().ok()).collect()
}

/// Resolves `rest` from `origin` one step at a time as the kernel does, so a symlink
/// resolves before a later "..". Fails with the first step outside `root`; `None` means a
/// step is missing or not a directory, so the loader finds nothing there.
fn search_directory(root: &Path, origin: &Path, rest: &str) -> Result<Option<PathBuf>, PathBuf> {
    let mut directory = origin.to_path_buf();
    for component in Path::new(rest).components() {
        match component {
            Component::ParentDir => {
                directory.pop();
            }
            Component::Normal(name) => match fs::canonicalize(directory.join(name)) {
                Ok(next) if next.is_dir() => directory = next,
                _ => return Ok(None),
            },
            _ => continue,
        }
        if !directory.starts_with(root) {
            return Err(directory);
        }
    }
    Ok(Some(directory))
}

/// Checks that each of `elves` under `root` runs on the host glibc and finds each library
/// among the shared objects in `elves` or on the host, and that each of `links` resolves
/// inside `root`.
fn check(root: &Path, elves: &[PathBuf], links: &[PathBuf], gates: &Gates) -> io::Result<Report> {
    let root = fs::canonicalize(root)?;
    let mut parsed = Vec::new();
    // glibc loads shared objects, not executables, as dependencies. Whether each has a
    // RUNPATH matters because a library's $ORIGIN is the directory it was found in.
    let mut libraries = HashMap::new();
    for file in elves {
        let elf = Elf::read(file)?;
        if elf.library {
            libraries.insert(fs::canonicalize(file)?, !elf.runpath.is_empty());
        }
        parsed.push((file, elf));
    }
    let mut report = Report::default();
    for link in links {
        if !fs::canonicalize(link).is_ok_and(|target| target.starts_with(&root)) {
            let message = format!("{} does not resolve inside the tree", link.display());
            report.violations.push(("symlink", message));
        }
    }
    for (file, elf) in parsed {
        let mut violations = Vec::new();
        if let Some(interpreter) = elf
            .interpreter
            .filter(|interpreter| interpreter != INTERPRETER)
        {
            violations.push(("interpreter", interpreter));
        }
        if elf.rpath {
            violations.push(("runpath", "uses DT_RPATH".into()));
        }
        let origin = fs::canonicalize(file.parent().unwrap())?;
        let mut directories = Vec::new();
        for entry in &elf.runpath {
            // Only a leading $ORIGIN path component counts.
            let Some(rest) = entry
                .strip_prefix("$ORIGIN")
                .filter(|rest| rest.is_empty() || rest.starts_with('/'))
            else {
                violations.push(("runpath", format!("{entry:?} is outside $ORIGIN")));
                continue;
            };
            match search_directory(&root, &origin, rest) {
                Ok(directory) => directories.extend(directory),
                Err(step) => {
                    let message = format!("{entry} leaves the tree at {}", step.display());
                    violations.push(("runpath", message));
                }
            }
        }
        if elf.unlisted_needs {
            violations.push((
                "glibc-floor",
                "has version needs readelf cannot list".into(),
            ));
        }
        report.glibc_needs += elf.glibc.len();
        for name in elf.glibc {
            let needed = name.strip_prefix("GLIBC_").and_then(version);
            if needed.is_none_or(|needed| needed > gates.floor) {
                violations.push(("glibc-floor", format!("needs {name}")));
            }
        }
        for library in elf.needed {
            // The loader opens a name with a slash from the working directory.
            if library.contains('/') {
                violations.push(("needed", format!("{library} is a path, not a soname")));
                continue;
            }
            // The loader takes the first candidate that exists and fails if it cannot load
            // it. A library found through a symlink from another directory takes that
            // directory as its $ORIGIN, so it must not resolve its own libraries from there.
            let found = directories.iter().find_map(|directory| {
                let provider = fs::canonicalize(directory.join(&library)).ok()?;
                let runpath = libraries.get(&provider).copied();
                let elsewhere = provider.parent() != Some(directory.as_path());
                Some(runpath.map(|runpath| runpath && elsewhere))
            });
            match found {
                Some(Some(false)) => report.in_tree += 1,
                Some(Some(true)) => {
                    let message = format!("{library} is found away from its $ORIGIN");
                    violations.push(("needed", message));
                }
                Some(None) => {
                    let message = format!("{library} is first found as no shared object");
                    violations.push(("needed", message));
                }
                None if gates.host.contains(&library) => report.on_host += 1,
                None => violations.push(("needed", format!("{library} resolves nowhere"))),
            }
        }
        let file = file.display();
        let violations = violations.into_iter();
        report
            .violations
            .extend(violations.map(|(kind, detail)| (kind, format!("{file}: {detail}"))));
    }
    Ok(report)
}

struct Options {
    registration: PathBuf,
    root: String,
    exclude: Vec<String>,
    host_runtime: HashSet<PathBuf>,
    substitute: HashMap<String, String>,
    gates: Gates,
    out: PathBuf,
}

fn options(mut arguments: impl Iterator<Item = String>) -> io::Result<Options> {
    let (mut registration, mut root, mut floor, mut out) = (None, None, None, None);
    let (mut exclude, mut host, mut host_runtime) = (Vec::new(), HashSet::new(), HashSet::new());
    let mut substitute = HashMap::new();
    while let Some(argument) = arguments.next() {
        let mut value = || arguments.next().ok_or_else(|| invalid(HELP));
        match argument.as_str() {
            "--registration" => registration = Some(value()?.into()),
            "--root" => root = Some(value()?),
            "--exclude" => exclude.push(value()?),
            "--glibc-floor" => floor = Some(value()?),
            "--host-library" => {
                host.insert(value()?);
            }
            "--host-runtime" => {
                host_runtime.insert(value()?.into());
            }
            "--substitute" => {
                let value = value()?;
                let (original, replacement) = value.split_once('=').ok_or_else(|| invalid(HELP))?;
                let same_name = match (store_name(original), store_name(replacement)) {
                    (Some((original, "")), Some((replacement, ""))) => {
                        original.split_once('-').map(|(_, name)| name)
                            == replacement.split_once('-').map(|(_, name)| name)
                            && original.len() == replacement.len()
                    }
                    _ => false,
                };
                if !same_name {
                    return Err(invalid(format!(
                        "substitute is not a store path with the same name: {value}"
                    )));
                }
                substitute.insert(original.into(), replacement.into());
            }
            _ if !argument.starts_with('-') && out.is_none() => out = Some(argument.into()),
            _ => return Err(invalid(HELP)),
        }
    }
    let floor = floor.as_deref().and_then(version);
    match (registration, root, floor, out) {
        (Some(registration), Some(root), Some(floor), Some(out)) => Ok(Options {
            registration,
            root,
            exclude,
            host_runtime,
            substitute,
            gates: Gates { floor, host },
            out,
        }),
        _ => Err(invalid(HELP)),
    }
}

fn run() -> io::Result<bool> {
    let options = options(env::args().skip(1))?;
    let registration = fs::read_to_string(&options.registration)?;
    let (kept, pruned) = closure(&registration, &options.root, &options.exclude)?;
    // Registered, excluded and substitute paths are store paths, so each has a name.
    let name = |path| store_name(path).unwrap().0;
    let mut relink = Relink {
        kept: kept.iter().map(|path| name(path)).collect(),
        excluded: options.exclude.iter().map(|path| name(path)).collect(),
        host_runtime: options.host_runtime,
        ..Relink::default()
    };
    if let Some(original) = options
        .substitute
        .keys()
        .find(|original| !kept.contains(&original.as_str()))
    {
        return Err(invalid(format!(
            "substituted path is not shipped: {original}"
        )));
    }
    let store = options.out.join("nix/store");
    fs::create_dir_all(&store)?;
    for path in &kept {
        let replacement = options.substitute.get(*path);
        relink.rename = replacement.map(|replacement| (name(replacement), name(path)));
        let from = replacement.map_or(*path, String::as_str);
        relink.copy(Path::new(from), &store.join(name(path)), 0)?;
    }
    if let Some(file) = relink.host_runtime.iter().next() {
        return Err(invalid(format!(
            "host-runtime file is not a shipped ELF file: {}",
            file.display()
        )));
    }
    let report = check(&options.out, &relink.elves, &relink.links, &options.gates)?;
    let mut kinds = BTreeMap::new();
    for (kind, message) in &report.violations {
        eprintln!("eon-relink: {kind}: {message}");
        *kinds.entry(*kind).or_insert(0) += 1;
    }
    let kinds: Vec<_> = kinds
        .iter()
        .map(|(kind, count)| format!("{count} {kind}"))
        .collect();
    println!(
        "eon-relink: shipped {} store paths, excluded {}, pruned {pruned}",
        kept.len(),
        options.exclude.len()
    );
    println!(
        "eon-relink: {} ELF files: {} interpreters set, {} runpaths rewritten ({} host entries dropped); {} symlinks made relative",
        relink.elves.len(),
        relink.interpreters,
        relink.runpaths,
        relink.dropped,
        relink.relative
    );
    let floor: Vec<_> = options.gates.floor.iter().map(u32::to_string).collect();
    println!(
        "eon-relink: {} GLIBC version needs against floor {}; NEEDED {} in the tree, {} on the host; violations: {}",
        report.glibc_needs,
        floor.join("."),
        report.in_tree,
        report.on_host,
        if kinds.is_empty() {
            "none".into()
        } else {
            kinds.join(", ")
        }
    );
    Ok(report.violations.is_empty())
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("eon-relink: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
