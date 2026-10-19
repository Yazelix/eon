use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    env, fs,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt, symlink},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

const MARKER: &str = "eon-install-v1\n";
const HELP: &str =
    "Usage: eon-install install HTTPS_ARCHIVE --signers ALLOWED_SIGNERS [--prefix DIR] [--dry-run]
       eon-install rollback SHA256 [--prefix DIR] [--dry-run]
       eon-install remove [--prefix DIR] [--dry-run]

Linux x86_64 only. Install/rollback require host GNU coreutils timeout.
Installation also requires curl and OpenSSH ssh-keygen.
The independently trusted signer authorizes eon-release in namespace eon-bundle.
Default prefix: $XDG_DATA_HOME/eon/install or $HOME/.local/share/eon/install.
Add PREFIX/bin to PATH yourself; no shell or desktop configuration is changed.
All version payloads remain in place, including after remove, to preserve live work.";

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::other(message.into())
}

fn digest_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn normal(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

fn extract(reader: impl Read, destination: &Path, root: &str) -> io::Result<()> {
    let mut archive = tar::Archive::new(reader);
    let mut paths = HashSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let kind = entry.header().entry_type();
        if !normal(&path)
            || !path.starts_with(root)
            || (path == Path::new(root) && !kind.is_dir())
            || path == Path::new(root).join(".eon-source")
            || !paths.insert(path.clone())
            || !(kind.is_file() || kind.is_dir() || kind.is_symlink() || kind.is_hard_link())
        {
            return Err(invalid(format!("unsafe archive entry: {}", path.display())));
        }
        if (kind.is_symlink() || kind.is_hard_link())
            && let Some(link) = entry.link_name()?
        {
            if kind.is_hard_link() {
                if !normal(&link) || !link.starts_with(root) {
                    return Err(invalid("hard link escapes archive root"));
                }
            } else {
                let mut depth = path.components().count() - 2;
                for component in link.components() {
                    match component {
                        Component::Normal(_) => depth += 1,
                        Component::CurDir => (),
                        Component::ParentDir if depth > 0 => depth -= 1,
                        _ => return Err(invalid("symlink escapes archive root")),
                    }
                }
            }
        }
        if !entry.unpack_in(destination)? {
            return Err(invalid("archive entry was not extracted"));
        }
        if kind.is_dir() {
            fs::set_permissions(destination.join(path), fs::Permissions::from_mode(0o700))?;
        }
    }
    // Consume the decoder through EOF so a truncated/compressed trailer cannot be ignored.
    io::copy(&mut archive.into_inner(), &mut io::sink())?;
    Ok(())
}

fn private_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != fs::metadata("/proc/self")?.uid()
        || metadata.mode() & 0o077 != 0
    {
        return Err(invalid(format!(
            "expected user-owned private directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn prefix_lock(prefix: &Path, create: bool) -> io::Result<File> {
    let created = match fs::symlink_metadata(prefix) {
        Ok(_) => false,
        Err(error) if create && error.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(prefix)?;
            true
        }
        Err(error) => return Err(error),
    };
    private_directory(prefix)?;
    let marker = prefix.join(".eon-install");
    if created {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&marker)?;
        file.write_all(MARKER.as_bytes())?;
    }
    if !fs::symlink_metadata(&marker)?.is_file() || fs::read_to_string(&marker)? != MARKER {
        return Err(invalid("prefix is not owned by eon-install"));
    }
    let lock = OpenOptions::new().read(true).write(true).open(marker)?;
    lock.try_lock()
        .map_err(|error| invalid(format!("prefix is busy: {error}")))?;
    for name in ["versions", "bin"] {
        let path = prefix.join(name);
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
        private_directory(&path)?;
    }
    Ok(lock)
}

fn selected(prefix: &Path) -> io::Result<Option<String>> {
    match fs::read_link(prefix.join("current")) {
        Ok(target) => {
            let parts: Vec<_> = target.components().collect();
            if let [Component::Normal(first), Component::Normal(hash)] = parts.as_slice()
                && *first == "versions"
                && let Some(hash) = hash.to_str()
                && digest_valid(hash)
            {
                return Ok(Some(hash.to_owned()));
            }
            Err(invalid("current is not an installer-owned selector"))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn bin_owned(prefix: &Path) -> io::Result<bool> {
    match fs::read_link(prefix.join("bin/eon")) {
        Ok(target) if target == Path::new("../current/eon") => Ok(true),
        Ok(_) => Err(invalid("preserving unrelated bin/eon symlink")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(invalid("preserving unrelated bin/eon file")),
    }
}

/// The payload's eon links to the launcher in its relinked tree, which finds the tree from
/// its own path.
fn entrypoint(root: &Path) -> io::Result<()> {
    let path = root.join("eon");
    if !fs::symlink_metadata(&path)?.is_symlink()
        || !fs::metadata(&path)
            .is_ok_and(|target| target.is_file() && target.permissions().mode() & 0o111 != 0)
    {
        return Err(invalid(format!(
            "expected packaged entrypoint link: {}",
            path.display()
        )));
    }
    Ok(())
}

fn version(prefix: &Path, hash: &str) -> io::Result<PathBuf> {
    if !digest_valid(hash) {
        return Err(invalid("version must be a lowercase SHA-256 digest"));
    }
    let path = prefix.join("versions").join(hash);
    private_directory(&path)?;
    let receipt = path.join(".eon-source");
    if !fs::symlink_metadata(&receipt)?.is_file()
        || !fs::read_to_string(receipt)?.starts_with(&format!("{MARKER}{hash}\n"))
    {
        return Err(invalid("version lacks its installer receipt"));
    }
    entrypoint(&path)?;
    Ok(path)
}

fn smoke(root: &Path) -> io::Result<()> {
    // timeout owns a separate group so a hung wrapper cannot leave its child running.
    let status = Command::new("timeout")
        .args(["--signal=KILL", "30s"])
        .arg(root.join("eon"))
        .arg("versions")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(invalid(
            "packaged eon versions failed or timed out; current unchanged",
        ))
    }
}

fn select(prefix: &Path, hash: &str) -> io::Result<()> {
    selected(prefix)?;
    let has_bin = bin_owned(prefix)?;
    smoke(&version(prefix, hash)?)?;
    let temporary = tempfile::Builder::new()
        .prefix(".eon-select-")
        .tempdir_in(prefix)?;
    symlink(
        Path::new("versions").join(hash),
        temporary.path().join("current"),
    )?;
    if !has_bin {
        symlink("../current/eon", prefix.join("bin/eon"))?;
    }
    fs::rename(temporary.path().join("current"), prefix.join("current"))
}

fn remove(prefix: &Path) -> io::Result<()> {
    selected(prefix)?;
    match bin_owned(prefix) {
        Ok(true) => fs::remove_file(prefix.join("bin/eon"))?,
        Ok(false) => (),
        Err(error) => eprintln!("eon-install: {error}"),
    }
    if selected(prefix)?.is_some() {
        fs::remove_file(prefix.join("current"))?;
    }
    Ok(())
}

fn download(url: &str, destination: &Path) -> io::Result<()> {
    let status = Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--globoff",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--tlsv1.2",
            "--connect-timeout",
            "15",
            "--max-time",
            "900",
            "--max-redirs",
            "5",
        ])
        .arg("--output")
        .arg(destination)
        .arg("--url")
        .arg(url)
        .stdin(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(invalid(format!("download failed: {url}")))
    }
}

fn release(url: &str) -> io::Result<(&str, &str, &str)> {
    let (base, filename) = url
        .rsplit_once('/')
        .ok_or_else(|| invalid("expected exact HTTPS archive URL"))?;
    if !base.starts_with("https://")
        || base.len() <= 8
        || url.chars().any(|c| c.is_control() || c == '?' || c == '#')
        || !filename
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        || !filename.starts_with("eon-")
        || !filename.ends_with("-x86_64-linux.tar.zst")
    {
        return Err(invalid(
            "expected pinned HTTPS eon-<version>-x86_64-linux.tar.zst URL",
        ));
    }
    Ok((base, filename, filename.strip_suffix(".tar.zst").unwrap()))
}

fn checksum(text: &str, filename: &str) -> io::Result<String> {
    let matches: Vec<_> = text
        .lines()
        .filter_map(|line| {
            let (hash, name) = line.split_once("  ")?;
            (name == filename).then_some(hash)
        })
        .collect();
    match matches.as_slice() {
        [hash] if digest_valid(hash) => Ok((*hash).to_owned()),
        _ => Err(invalid(
            "signed checksums must bind exactly one matching archive",
        )),
    }
}

fn install(prefix: &Path, url: &str, signers: &Path, dry_run: bool) -> io::Result<()> {
    let (base, filename, root) = release(url)?;
    let downloads = tempfile::tempdir()?;
    for name in ["SHA256SUMS", "SHA256SUMS.sig"] {
        download(&format!("{base}/{name}"), &downloads.path().join(name))?;
    }
    let sums = downloads.path().join("SHA256SUMS");
    let status = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-f"])
        .arg(signers)
        .args(["-I", "eon-release", "-n", "eon-bundle", "-s"])
        .arg(downloads.path().join("SHA256SUMS.sig"))
        .stdin(File::open(&sums)?)
        .status()?;
    if !status.success() {
        return Err(invalid("release signature verification failed"));
    }
    let hash = checksum(&fs::read_to_string(sums)?, filename)?;
    let archive = downloads.path().join(filename);
    download(url, &archive)?;
    let mut digest = Sha256::new();
    let mut input = File::open(&archive)?;
    let mut buffer = [0; 65536];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    if digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        != hash
    {
        return Err(invalid("archive SHA-256 mismatch"));
    }
    if dry_run {
        println!(
            "Verified {hash}; would install and select under {}",
            prefix.display()
        );
        return Ok(());
    }
    let _lock = prefix_lock(prefix, true)?;
    selected(prefix)?;
    bin_owned(prefix)?;
    let target = prefix.join("versions").join(&hash);
    let present = match fs::symlink_metadata(&target) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if !present {
        let staging = tempfile::Builder::new()
            .prefix(".eon-stage-")
            .tempdir_in(prefix)?;
        extract(
            zstd::stream::read::Decoder::new(File::open(archive)?)?,
            staging.path(),
            root,
        )?;
        let payload = staging.path().join(root);
        entrypoint(&payload)?;
        smoke(&payload)?;
        let mut receipt = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(payload.join(".eon-source"))?;
        writeln!(receipt, "{MARKER}{hash}\n{url}")?;
        fs::rename(payload, &target)?;
    }
    select(prefix, &hash)?;
    println!(
        "Selected {hash}\nRun {}/bin/eon; previous payloads are retained.",
        prefix.display()
    );
    Ok(())
}

struct Options {
    command: String,
    value: Option<String>,
    prefix: PathBuf,
    signers: Option<PathBuf>,
    dry_run: bool,
}

fn options(args: impl IntoIterator<Item = std::ffi::OsString>) -> io::Result<Options> {
    let mut prefix = None;
    let mut signers = None;
    let mut dry_run = false;
    let mut positions = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--prefix") if prefix.is_none() => {
                prefix = Some(PathBuf::from(
                    args.next().ok_or_else(|| invalid("missing prefix"))?,
                ))
            }
            Some("--signers") if signers.is_none() => {
                signers = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| invalid("missing signer record"))?,
                ))
            }
            Some("--dry-run") if !dry_run => dry_run = true,
            Some(value) if !value.starts_with('-') => positions.push(value.to_owned()),
            _ => return Err(invalid("invalid argument; use --help")),
        }
    }
    let valid = matches!(positions.as_slice(), [command, _] if command == "install" && signers.is_some())
        || matches!(positions.as_slice(), [command, _] if command == "rollback" && signers.is_none())
        || matches!(positions.as_slice(), [command] if command == "remove" && signers.is_none());
    if !valid {
        return Err(invalid("invalid command; use --help"));
    }
    if positions[0] == "install" {
        release(&positions[1])?;
    } else if positions[0] == "rollback" && !digest_valid(&positions[1]) {
        return Err(invalid("version must be a lowercase SHA-256 digest"));
    }
    let prefix = prefix
        .or_else(|| {
            env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|p| p.join("eon/install"))
        })
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|p| p.join(".local/share/eon/install"))
        })
        .ok_or_else(|| invalid("provide --prefix or an absolute HOME/XDG_DATA_HOME"))?;
    if !prefix.is_absolute()
        || prefix
            .components()
            .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(invalid("prefix must be absolute without parent traversal"));
    }
    Ok(Options {
        command: positions.remove(0),
        value: positions.pop(),
        prefix,
        signers,
        dry_run,
    })
}

fn run() -> io::Result<()> {
    let arguments: Vec<_> = env::args_os().skip(1).collect();
    if arguments.is_empty() || arguments == ["--help"] || arguments == ["-h"] {
        println!("{HELP}");
        return Ok(());
    }
    if arguments == ["--version"] {
        println!("eon-install {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err(invalid("only x86_64 Linux is supported"));
    }
    let options = options(arguments)?;
    if options.command == "install" {
        return install(
            &options.prefix,
            options.value.as_deref().unwrap(),
            options.signers.as_deref().unwrap(),
            options.dry_run,
        );
    }
    if options.dry_run {
        println!(
            "Would {} under {}; all payloads retained",
            options.command,
            options.prefix.display()
        );
        return Ok(());
    }
    let _lock = prefix_lock(&options.prefix, false)?;
    if options.command == "rollback" {
        select(&options.prefix, options.value.as_deref().unwrap())?;
        println!("Selected {}", options.value.unwrap());
    } else {
        remove(&options.prefix)?;
        println!(
            "Removed owned entrypoints; payloads retained under {}",
            options.prefix.join("versions").display()
        );
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("eon-install: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests;
