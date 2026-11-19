//! The network of eon-bench's Nix-free update journey: a stand-in for `curl`
//! that serves release URLs from local files.
//!
//! The journey binds this static program at `/usr/bin/curl` (where
//! `eon-install` finds curl) and over each payload's own curl (where the
//! bundle launcher points `EON_CURL`). It accepts exactly the argument forms
//! those two callers pass, so a caller that changes them fails with exit 2
//! instead of passing on a misread request.
//!
//! `$EON_BENCH_NET/routes` maps URLs to files, one `URL<TAB>FILE` line each;
//! an unrouted URL answers 404. Every request is appended to
//! `$EON_BENCH_NET/requests.log` as `URL<TAB>STATUS`.

use std::{
    env,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const NET: &str = "EON_BENCH_NET";
/// `eon update`'s `--write-out`: a newline, then the status.
const STATUS_LINE: &str = "\n%{http_code}";
/// curl's exit for an HTTP error under `--fail`.
const HTTP_ERROR: u8 = 22;
/// curl's exit for a body over `--max-filesize`.
const TOO_LARGE: u8 = 63;
/// curl's exit for an unsupported protocol.
const PROTOCOL: u8 = 1;
/// A usage or fixture error: this program, not the request, is wrong.
const USAGE: u8 = 2;

const SWITCHES: [&str; 8] = [
    "-q",
    "--disable",
    "--fail",
    "--location",
    "--silent",
    "--show-error",
    "--globoff",
    "--tlsv1.2",
];
const VALUED: [&str; 11] = [
    "--proto",
    "--proto-redir",
    "--max-redirs",
    "--connect-timeout",
    "--max-time",
    "--max-filesize",
    "--header",
    "--user-agent",
    "--write-out",
    "--output",
    "--url",
];

#[derive(Debug, Default, PartialEq)]
struct Request {
    url: String,
    output: Option<PathBuf>,
    fail: bool,
    max_filesize: Option<u64>,
    write_status: bool,
}

/// An exit status with the message curl would print.
#[derive(Debug, PartialEq)]
struct Exit(u8, String);

fn usage(message: impl Into<String>) -> Exit {
    Exit(USAGE, format!("fake curl: {}", message.into()))
}

fn parse(args: &[String]) -> Result<Request, Exit> {
    let mut request = Request::default();
    let mut url = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if SWITCHES.contains(&arg.as_str()) {
            request.fail |= arg == "--fail";
            continue;
        }
        if !VALUED.contains(&arg.as_str()) {
            if arg.starts_with('-') {
                return Err(usage(format!("unsupported argument {arg}")));
            }
            if url.replace(arg.clone()).is_some() {
                return Err(usage("more than one URL"));
            }
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| usage(format!("{arg} needs a value")))?;
        match arg.as_str() {
            "--proto" | "--proto-redir" if value != "=https" => {
                return Err(usage(format!("{arg} {value}: only =https is supported")));
            }
            "--max-filesize" => {
                request.max_filesize = Some(
                    value
                        .parse()
                        .map_err(|_| usage(format!("--max-filesize {value}")))?,
                );
            }
            "--write-out" if value != STATUS_LINE => {
                return Err(usage(format!("--write-out {value:?} is unsupported")));
            }
            "--write-out" => request.write_status = true,
            "--output" => request.output = Some(value.into()),
            "--url" if url.replace(value.clone()).is_some() => {
                return Err(usage("more than one URL"));
            }
            _ => {}
        }
    }
    request.url = url.ok_or_else(|| usage("no URL"))?;
    if !request.url.starts_with("https://") {
        return Err(Exit(
            PROTOCOL,
            format!("curl: (1) Protocol not supported: {}", request.url),
        ));
    }
    Ok(request)
}

/// The file `url` is routed to, if any.
fn route(net: &Path, url: &str) -> Result<Option<PathBuf>, Exit> {
    let routes = fs::read_to_string(net.join("routes"))
        .map_err(|error| usage(format!("{}/routes: {error}", net.display())))?;
    for line in routes.lines() {
        let (pattern, file) = line
            .split_once('\t')
            .ok_or_else(|| usage(format!("malformed route {line:?}")))?;
        if pattern == url {
            return Ok(Some(file.into()));
        }
    }
    Ok(None)
}

fn log(net: &Path, url: &str, status: &str) -> Result<(), Exit> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(net.join("requests.log"))
        .and_then(|mut log| log.write_all(format!("{url}\t{status}\n").as_bytes()))
        .map_err(|error| usage(format!("requests.log: {error}")))
}

/// Answers `args` from `net`, writing a body without `--output` and the
/// status line to `stdout`.
fn run(args: &[String], net: Option<OsString>, stdout: &mut dyn Write) -> Result<(), Exit> {
    let request = parse(args)?;
    let net = PathBuf::from(net.ok_or_else(|| usage(format!("{NET} is unset")))?);
    let body = route(&net, &request.url)?;
    let status = if body.is_some() { 200 } else { 404 };
    let size = match &body {
        Some(file) => Some(
            fs::metadata(file)
                .map_err(|error| usage(format!("{}: {error}", file.display())))?
                .len(),
        ),
        None => None,
    };
    if let (Some(size), Some(limit)) = (size, request.max_filesize)
        && size > limit
    {
        log(&net, &request.url, "too-large")?;
        return Err(Exit(
            TOO_LARGE,
            "curl: (63) Maximum file size exceeded".into(),
        ));
    }
    log(&net, &request.url, &status.to_string())?;
    if status >= 400 && request.fail {
        return Err(Exit(
            HTTP_ERROR,
            format!("curl: (22) The requested URL returned error: {status}"),
        ));
    }
    let io = |error: io::Error| usage(format!("writing the response: {error}"));
    if let Some(file) = &body {
        let mut input = File::open(file).map_err(io)?;
        match &request.output {
            Some(path) => {
                io::copy(&mut input, &mut File::create(path).map_err(io)?).map_err(io)?;
            }
            None => {
                io::copy(&mut input, stdout).map_err(io)?;
            }
        }
    } else if let Some(path) = &request.output {
        File::create(path).map_err(io)?;
    }
    if request.write_status {
        write!(stdout, "\n{status}").map_err(io)?;
    }
    stdout.flush().map_err(io)
}

fn main() -> ExitCode {
    let args: Vec<String> = match env::args_os().skip(1).map(OsString::into_string).collect() {
        Ok(args) => args,
        Err(_) => {
            eprintln!("fake curl: non-UTF-8 argument");
            return ExitCode::from(USAGE);
        }
    };
    match run(&args, env::var_os(NET), &mut io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Exit(code, message)) => {
            eprintln!("{message}");
            ExitCode::from(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "https://api.github.com/repos/Yazelix/eon/releases?per_page=100&page=1";
    const SUMS: &str = "https://github.com/Yazelix/eon/releases/download/v0.1.29/SHA256SUMS";

    struct Net(PathBuf);

    impl Net {
        fn new(name: &str) -> Self {
            let dir = env::temp_dir().join(format!("fake-curl-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("listing.json"), "[]").unwrap();
            fs::write(dir.join("sums"), "abc  eon.tar.zst\n").unwrap();
            fs::write(
                dir.join("routes"),
                format!(
                    "{LISTING}\t{}\n{SUMS}\t{}\n",
                    dir.join("listing.json").display(),
                    dir.join("sums").display()
                ),
            )
            .unwrap();
            Self(dir)
        }

        fn run(&self, args: &[&str]) -> (Result<(), Exit>, Vec<u8>) {
            let args: Vec<String> = args.iter().map(|arg| (*arg).into()).collect();
            let mut out = Vec::new();
            let result = run(&args, Some(self.0.clone().into()), &mut out);
            (result, out)
        }

        fn log(&self) -> String {
            fs::read_to_string(self.0.join("requests.log")).unwrap_or_default()
        }
    }

    impl Drop for Net {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// `eon update`'s argv (eon/crates/eon/src/update.rs, `System::get`).
    fn update_args(url: &str) -> Vec<&str> {
        vec![
            "-q",
            "--silent",
            "--show-error",
            "--proto",
            "=https",
            "--max-redirs",
            "0",
            "--connect-timeout",
            "10",
            "--max-time",
            "30",
            "--max-filesize",
            "16777216",
            "--header",
            "Accept: application/vnd.github+json",
            "--header",
            "X-GitHub-Api-Version: 2022-11-28",
            "--user-agent",
            "eon/0.1.28",
            "--write-out",
            STATUS_LINE,
            url,
        ]
    }

    /// `eon-install`'s argv (installer/src/main.rs, `download`).
    fn installer_args<'a>(url: &'a str, output: &'a str) -> Vec<&'a str> {
        vec![
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
            "--output",
            output,
            "--url",
            url,
        ]
    }

    #[test]
    fn the_updaters_request_gets_its_body_and_status_line() {
        let net = Net::new("update");
        let (result, out) = net.run(&update_args(LISTING));
        assert_eq!(result, Ok(()));
        assert_eq!(out, b"[]\n200");
        let missing = format!("{LISTING}x");
        let (result, out) = net.run(&update_args(&missing));
        assert_eq!(result, Ok(()));
        assert_eq!(out, b"\n404");
        assert_eq!(net.log(), format!("{LISTING}\t200\n{missing}\t404\n"));
    }

    #[test]
    fn the_installers_download_writes_its_output_and_fails_on_404() {
        let net = Net::new("installer");
        let output = net.0.join("downloaded");
        let output = output.to_str().unwrap();
        let (result, out) = net.run(&installer_args(SUMS, output));
        assert_eq!((result, out), (Ok(()), Vec::new()));
        assert_eq!(fs::read_to_string(output).unwrap(), "abc  eon.tar.zst\n");
        fs::remove_file(output).unwrap();
        let (result, _) = net.run(&installer_args(&format!("{SUMS}.sig"), output));
        assert_eq!(result.unwrap_err().0, HTTP_ERROR);
        assert!(!Path::new(output).exists());
    }

    #[test]
    fn other_protocols_flags_and_oversized_bodies_are_refused() {
        let net = Net::new("refusals");
        let http = LISTING.replacen("https", "http", 1);
        assert_eq!(net.run(&update_args(&http)).0.unwrap_err().0, PROTOCOL);
        for bad in [
            vec!["--insecure", SUMS],
            vec!["--proto", "=http", SUMS],
            vec!["--write-out", "%{http_code}", SUMS],
            vec![SUMS, SUMS],
            vec!["--silent"],
        ] {
            assert_eq!(net.run(&bad).0.unwrap_err().0, USAGE, "{bad:?}");
        }
        let (result, _) = net.run(&["--max-filesize", "3", SUMS]);
        assert_eq!(result.unwrap_err().0, TOO_LARGE);
        assert_eq!(net.log(), format!("{SUMS}\ttoo-large\n"));
        let args: Vec<String> = vec![SUMS.into()];
        assert_eq!(run(&args, None, &mut Vec::new()).unwrap_err().0, USAGE);
    }
}
