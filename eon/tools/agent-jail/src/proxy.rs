//! The host-side CONNECT proxy: the jail's only route out.
//!
//! It listens on `<run>/sock/proxy`, serves only the jail's init (checked
//! with `SO_PEERCRED` against the host PID bwrap reported), and tunnels
//! `CONNECT host:443` to hosts on the profile's allowlist. Every connection
//! leaves one line in `meta/proxy.jsonl`.

use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::allowlist::Allowlist;

/// The request line init sends to have a refused bridge connection logged.
pub const DENY_OWNER: &str = "AGENT-JAIL-DENY-OWNER";

/// Resolves an allowed host; tests inject a loopback resolver.
pub type Resolver = Arc<dyn Fn(&str, u16) -> std::io::Result<Vec<SocketAddr>> + Send + Sync>;

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_connections: usize,
    pub max_header: usize,
    pub header_timeout: Duration,
    pub idle_timeout: Duration,
    pub connect_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 32,
            max_header: 8 * 1024,
            header_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(15 * 60),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

pub struct Proxy {
    allow: Allowlist,
    /// Init's host PID; 0 until bwrap reports it, which refuses every peer.
    init_pid: AtomicU32,
    log: Mutex<File>,
    limits: Limits,
    resolver: Resolver,
    active: AtomicUsize,
}

#[derive(Serialize, Default)]
struct LogLine {
    t_ms: u128,
    verdict: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    peer_pid: Option<i32>,
    #[serde(skip_serializing_if = "String::is_empty")]
    detail: String,
    up: u64,
    down: u64,
    ms: u128,
    #[serde(skip_serializing_if = "String::is_empty")]
    error: String,
}

impl Proxy {
    pub fn new(allow: Allowlist, log: File, limits: Limits, resolver: Resolver) -> Arc<Self> {
        Arc::new(Self {
            allow,
            init_pid: AtomicU32::new(0),
            log: Mutex::new(log),
            limits,
            resolver,
            active: AtomicUsize::new(0),
        })
    }

    /// The system resolver, used only after a host matched.
    pub fn system_resolver() -> Resolver {
        Arc::new(|host, port| (host, port).to_socket_addrs().map(Iterator::collect))
    }

    pub fn set_init_pid(&self, pid: u32) {
        self.init_pid.store(pid, Ordering::SeqCst);
    }

    /// Serves `listener` on a background thread for the life of the process.
    pub fn serve(self: &Arc<Self>, listener: UnixListener) -> thread::JoinHandle<()> {
        let proxy = Arc::clone(self);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let proxy = Arc::clone(&proxy);
                thread::spawn(move || proxy.handle(stream));
            }
        })
    }

    fn write_log(&self, mut line: LogLine) {
        line.t_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_millis());
        if let (Ok(text), Ok(mut log)) = (serde_json::to_string(&line), self.log.lock()) {
            let _ = writeln!(log, "{text}");
            let _ = log.flush();
        }
    }

    fn handle(&self, mut client: UnixStream) {
        let started = Instant::now();
        let peer_pid = rustix::net::sockopt::socket_peercred(&client)
            .ok()
            .map(|cred| cred.pid.as_raw_nonzero().get());
        let init = self.init_pid.load(Ordering::SeqCst);
        if init == 0 || peer_pid != i32::try_from(init).ok() {
            self.write_log(LogLine {
                verdict: "deny-peer",
                peer_pid,
                ms: started.elapsed().as_millis(),
                ..LogLine::default()
            });
            return;
        }
        if self.active.fetch_add(1, Ordering::SeqCst) >= self.limits.max_connections {
            self.active.fetch_sub(1, Ordering::SeqCst);
            let _ = client.write_all(b"HTTP/1.1 503 Busy\r\n\r\n");
            self.write_log(LogLine {
                verdict: "busy",
                ms: started.elapsed().as_millis(),
                ..LogLine::default()
            });
            return;
        }
        let mut line = self.tunnel(&mut client);
        self.active.fetch_sub(1, Ordering::SeqCst);
        line.ms = started.elapsed().as_millis();
        self.write_log(line);
    }

    fn tunnel(&self, client: &mut UnixStream) -> LogLine {
        let (head, rest) = match read_head(client, &self.limits) {
            Ok(parts) => parts,
            Err(error) => {
                let _ = client.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
                return LogLine {
                    verdict: "bad-request",
                    error,
                    ..LogLine::default()
                };
            }
        };
        let request_line = head.lines().next().unwrap_or_default();
        if let Some(detail) = request_line.strip_prefix(DENY_OWNER) {
            return LogLine {
                verdict: "deny-owner",
                detail: detail.trim().chars().take(512).collect(),
                ..LogLine::default()
            };
        }
        let (host, port) = match parse_connect(request_line) {
            Ok(target) => target,
            Err(error) => {
                let _ = client.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
                return LogLine {
                    verdict: "bad-request",
                    error,
                    ..LogLine::default()
                };
            }
        };
        if port != 443 || !self.allow.allows(&host) {
            let _ = client.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n");
            return LogLine {
                verdict: "deny-host",
                host,
                port: Some(port),
                ..LogLine::default()
            };
        }
        let upstream = match self.connect(&host, port) {
            Ok(upstream) => upstream,
            Err(error) => {
                let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n");
                return LogLine {
                    verdict: "allow",
                    host,
                    port: Some(port),
                    error,
                    ..LogLine::default()
                };
            }
        };
        let mut line = LogLine {
            verdict: "allow",
            host,
            port: Some(port),
            ..LogLine::default()
        };
        if let Err(error) = client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n") {
            line.error = error.to_string();
            return line;
        }
        let (up, down, error) = splice(client, upstream, &rest, self.limits.idle_timeout);
        line.up = up;
        line.down = down;
        line.error = error;
        line
    }

    fn connect(&self, host: &str, port: u16) -> Result<TcpStream, String> {
        let addresses = (self.resolver)(host, port).map_err(|err| format!("resolve: {err}"))?;
        let mut last = String::from("resolve: no addresses");
        for address in addresses {
            match TcpStream::connect_timeout(&address, self.limits.connect_timeout) {
                Ok(stream) => return Ok(stream),
                Err(err) => last = format!("connect: {err}"),
            }
        }
        Err(last)
    }
}

/// Reads the request head; returns it and any bytes read past it.
fn read_head(client: &mut UnixStream, limits: &Limits) -> Result<(String, Vec<u8>), String> {
    let deadline = Instant::now() + limits.header_timeout;
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = find(&buffer, b"\r\n\r\n") {
            let head = String::from_utf8(buffer[..end].to_vec())
                .map_err(|_| "request head is not UTF-8".to_owned())?;
            return Ok((head, buffer[end + 4..].to_vec()));
        }
        if buffer.len() > limits.max_header {
            return Err("request head too large".into());
        }
        let left = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or("request head timed out")?;
        client
            .set_read_timeout(Some(left))
            .map_err(|err| err.to_string())?;
        match client.read(&mut chunk) {
            Ok(0) => return Err("connection closed before the request head ended".into()),
            Ok(count) => buffer.extend_from_slice(&chunk[..count]),
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err("request head timed out".into());
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => {}
            Err(err) => return Err(err.to_string()),
        }
    }
}

/// Parses `CONNECT host:port HTTP/1.x`.
pub fn parse_connect(line: &str) -> Result<(String, u16), String> {
    let mut parts = line.split(' ');
    let (Some(method), Some(authority), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err("malformed request line".into());
    };
    if method != "CONNECT" {
        return Err(format!("method {method} is not CONNECT"));
    }
    if version != "HTTP/1.1" && version != "HTTP/1.0" {
        return Err("unsupported HTTP version".into());
    }
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or("CONNECT target has no port")?;
    let port: u16 = port.parse().map_err(|_| "CONNECT port is not a number")?;
    if host.is_empty() {
        return Err("CONNECT target has no host".into());
    }
    Ok((host.chars().take(253).collect(), port))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Copies both ways until both sides close or nothing moves for `idle`.
/// Returns bytes sent up, bytes sent down and the first error.
fn splice(
    client: &UnixStream,
    upstream: TcpStream,
    rest: &[u8],
    idle: Duration,
) -> (u64, u64, String) {
    let started = Instant::now();
    let last = Arc::new(AtomicU64::new(0));
    let clones = (|| {
        Ok::<_, std::io::Error>((
            client.try_clone()?,
            client.try_clone()?,
            upstream.try_clone()?,
        ))
    })();
    let Ok((client_read, client_write, mut upstream_write)) = clones else {
        return (0, 0, "cannot clone a stream".into());
    };
    if let Err(err) = upstream_write.write_all(rest) {
        return (0, 0, err.to_string());
    }
    let up = {
        let last = Arc::clone(&last);
        thread::spawn(move || pump(client_read, upstream_write, &last, started, idle))
    };
    let down = {
        let last = Arc::clone(&last);
        thread::spawn(move || pump(upstream, client_write, &last, started, idle))
    };
    let (up_bytes, up_error) = up.join().unwrap_or((0, "pump panicked".into()));
    let (down_bytes, down_error) = down.join().unwrap_or((0, "pump panicked".into()));
    let error = if up_error.is_empty() {
        down_error
    } else {
        up_error
    };
    (up_bytes + rest.len() as u64, down_bytes, error)
}

trait Stream: Read + Write {
    fn set_timeout(&self, timeout: Duration) -> std::io::Result<()>;
    fn half_close(&self);
}

impl Stream for UnixStream {
    fn set_timeout(&self, timeout: Duration) -> std::io::Result<()> {
        self.set_read_timeout(Some(timeout))
    }
    fn half_close(&self) {
        let _ = self.shutdown(Shutdown::Write);
    }
}

impl Stream for TcpStream {
    fn set_timeout(&self, timeout: Duration) -> std::io::Result<()> {
        self.set_read_timeout(Some(timeout))
    }
    fn half_close(&self) {
        let _ = self.shutdown(Shutdown::Write);
    }
}

/// Copies `from` into `to`, then half-closes `to`.
fn pump(
    mut from: impl Stream,
    mut to: impl Stream,
    last: &AtomicU64,
    started: Instant,
    idle: Duration,
) -> (u64, String) {
    let elapsed = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let idle_ms = u64::try_from(idle.as_millis()).unwrap_or(u64::MAX);
    if let Err(err) = from.set_timeout(idle.min(Duration::from_secs(30))) {
        return (0, err.to_string());
    }
    let mut total = 0u64;
    let mut buffer = vec![0u8; 16 * 1024];
    let error = loop {
        match from.read(&mut buffer) {
            Ok(0) => break String::new(),
            Ok(count) => {
                if let Err(err) = to.write_all(&buffer[..count]) {
                    break err.to_string();
                }
                total += count as u64;
                last.store(elapsed(), Ordering::SeqCst);
            }
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if elapsed().saturating_sub(last.load(Ordering::SeqCst)) >= idle_ms {
                    break "idle timeout".into();
                }
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => {}
            Err(err) => break err.to_string(),
        }
    };
    to.half_close();
    (total, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::os::unix::net::UnixStream;

    fn fixture(limits: Limits) -> (Arc<Proxy>, std::path::PathBuf, std::path::PathBuf, u16) {
        let dir = crate::test_dir("proxy");
        let socket = dir.join("proxy");
        let log_path = dir.join("proxy.jsonl");
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = upstream.local_addr().unwrap().port();
        // A fake upstream that echoes with a prefix.
        thread::spawn(move || {
            for stream in upstream.incoming() {
                let Ok(mut stream) = stream else { continue };
                thread::spawn(move || {
                    let mut buffer = [0u8; 256];
                    while let Ok(count) = stream.read(&mut buffer) {
                        if count == 0 {
                            break;
                        }
                        let mut reply = b"echo:".to_vec();
                        reply.extend_from_slice(&buffer[..count]);
                        if stream.write_all(&reply).is_err() {
                            break;
                        }
                    }
                });
            }
        });
        let resolver: Resolver = Arc::new(move |host, _| {
            if host == "api.example.com" {
                Ok(vec![SocketAddr::from(([127, 0, 0, 1], port))])
            } else {
                Err(std::io::Error::other("unexpected resolve"))
            }
        });
        let proxy = Proxy::new(
            Allowlist::parse("api.example.com\n").unwrap(),
            File::create(&log_path).unwrap(),
            limits,
            resolver,
        );
        proxy.serve(UnixListener::bind(&socket).unwrap());
        (proxy, socket, log_path, port)
    }

    fn log_lines(path: &std::path::Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn wait_for_lines(path: &std::path::Path, count: usize) -> Vec<serde_json::Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let lines = log_lines(path);
            if lines.len() >= count || Instant::now() > deadline {
                return lines;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn request(socket: &std::path::Path, text: &[u8]) -> String {
        let mut stream = UnixStream::connect(socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        // A refused peer is dropped before its request is read, so the
        // write may meet a closed socket; the reply (none) is what counts.
        let _ = stream.write_all(text);
        let mut reply = Vec::new();
        let mut buffer = [0u8; 256];
        while let Ok(count) = stream.read(&mut buffer) {
            if count == 0 {
                break;
            }
            reply.extend_from_slice(&buffer[..count]);
            if reply.windows(4).any(|window| window == b"\r\n\r\n")
                && !reply.starts_with(b"HTTP/1.1 200")
            {
                break;
            }
            if reply.ends_with(b"echo:hello") {
                break;
            }
        }
        String::from_utf8_lossy(&reply).into_owned()
    }

    #[test]
    fn allowed_connect_tunnels_and_forwards_buffered_bytes() {
        let (proxy, socket, log, _) = fixture(Limits::default());
        proxy.set_init_pid(std::process::id());
        let reply = request(
            &socket,
            b"CONNECT api.example.com:443 HTTP/1.1\r\nHost: x\r\n\r\nhello",
        );
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(reply.ends_with("echo:hello"), "{reply}");
        let lines = wait_for_lines(&log, 1);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["verdict"], "allow");
        assert_eq!(lines[0]["host"], "api.example.com");
        assert_eq!(lines[0]["up"], 5);
    }

    #[test]
    fn refusals_are_logged_once_each() {
        let (proxy, socket, log, _) = fixture(Limits::default());
        proxy.set_init_pid(std::process::id());
        let cases: [(&[u8], &str, &str); 7] = [
            (
                b"CONNECT github.com:443 HTTP/1.1\r\n\r\n",
                "403",
                "deny-host",
            ),
            (
                b"CONNECT api.example.com:80 HTTP/1.1\r\n\r\n",
                "403",
                "deny-host",
            ),
            (
                b"CONNECT 140.82.112.3:443 HTTP/1.1\r\n\r\n",
                "403",
                "deny-host",
            ),
            (
                b"CONNECT API.EXAMPLE.COM:443 HTTP/1.1\r\n\r\n",
                "403",
                "deny-host",
            ),
            (
                b"GET http://api.example.com/ HTTP/1.1\r\n\r\n",
                "400",
                "bad-request",
            ),
            (
                b"CONNECT api.example.com HTTP/1.1\r\n\r\n",
                "400",
                "bad-request",
            ),
            (
                b"AGENT-JAIL-DENY-OWNER 7 /bin/curl\r\n\r\n",
                "",
                "deny-owner",
            ),
        ];
        for (index, (text, status, _)) in cases.iter().enumerate() {
            let reply = request(&socket, text);
            assert!(reply.contains(status), "case {index}: {reply}");
            wait_for_lines(&log, index + 1);
        }
        let lines = wait_for_lines(&log, cases.len());
        assert_eq!(lines.len(), cases.len());
        for (line, (_, _, verdict)) in lines.iter().zip(cases) {
            assert_eq!(line["verdict"], verdict, "{line}");
        }
    }

    #[test]
    fn peers_other_than_init_are_refused() {
        let (proxy, socket, log, _) = fixture(Limits::default());
        // Before bwrap reports init, every peer is refused.
        let reply = request(&socket, b"CONNECT api.example.com:443 HTTP/1.1\r\n\r\n");
        assert!(reply.is_empty(), "{reply}");
        proxy.set_init_pid(1);
        let reply = request(&socket, b"CONNECT api.example.com:443 HTTP/1.1\r\n\r\n");
        assert!(reply.is_empty(), "{reply}");
        let lines = wait_for_lines(&log, 2);
        assert!(
            lines.iter().all(|line| line["verdict"] == "deny-peer"),
            "{lines:?}"
        );
        assert_eq!(lines[1]["peer_pid"], std::process::id());
    }

    #[test]
    fn oversized_and_slow_heads_are_refused() {
        let limits = Limits {
            header_timeout: Duration::from_millis(300),
            ..Limits::default()
        };
        let (proxy, socket, log, _) = fixture(limits);
        proxy.set_init_pid(std::process::id());
        let mut huge = b"CONNECT api.example.com:443 HTTP/1.1\r\nX: ".to_vec();
        huge.extend(std::iter::repeat_n(b'a', 9000));
        let reply = request(&socket, &huge);
        assert!(reply.contains("400"), "{reply}");
        let reply = request(&socket, b"CONNECT api.example.com:443 HTTP/1.1\r\n");
        assert!(reply.contains("400"), "{reply}");
        let lines = wait_for_lines(&log, 2);
        assert_eq!(lines[0]["error"], "request head too large");
        assert_eq!(lines[1]["error"], "request head timed out");
    }

    #[test]
    fn the_connection_cap_holds() {
        let limits = Limits {
            max_connections: 1,
            ..Limits::default()
        };
        let (proxy, socket, log, _) = fixture(limits);
        proxy.set_init_pid(std::process::id());
        let mut first = UnixStream::connect(&socket).unwrap();
        first
            .write_all(b"CONNECT api.example.com:443 HTTP/1.1\r\n\r\n")
            .unwrap();
        let mut ok = [0u8; 39];
        first.read_exact(&mut ok).unwrap();
        let reply = request(&socket, b"CONNECT api.example.com:443 HTTP/1.1\r\n\r\n");
        assert!(reply.contains("503"), "{reply}");
        let lines = wait_for_lines(&log, 1);
        assert_eq!(lines[0]["verdict"], "busy");
        drop(first);
    }

    #[test]
    fn idle_tunnels_close() {
        let limits = Limits {
            idle_timeout: Duration::from_millis(400),
            ..Limits::default()
        };
        let (proxy, socket, log, _) = fixture(limits);
        proxy.set_init_pid(std::process::id());
        let mut stream = UnixStream::connect(&socket).unwrap();
        stream
            .write_all(b"CONNECT api.example.com:443 HTTP/1.1\r\n\r\n")
            .unwrap();
        let lines = wait_for_lines(&log, 1);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["error"], "idle timeout");
    }
}
