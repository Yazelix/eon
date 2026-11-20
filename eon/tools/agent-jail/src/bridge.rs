//! Init's loopback bridge: `127.0.0.1:3128` in the outer layer, forwarded
//! to the host proxy only for sockets the harness process holds.

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, SocketAddrV4, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;

use crate::proxy::DENY_OWNER;

/// Who holds a socket, as far as `/proc` shows.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Holders {
    pub pids: Vec<u32>,
    /// Processes whose descriptor table could not be read.
    pub unreadable: usize,
}

/// The `/proc/net/tcp` inode of the socket bound to `local` and connected
/// to `remote`.
pub fn find_inode(table: &str, local: SocketAddrV4, remote: SocketAddrV4) -> Option<u64> {
    let encode = |address: SocketAddrV4| {
        format!(
            "{:08X}:{:04X}",
            u32::from_ne_bytes(address.ip().octets()),
            address.port()
        )
    };
    let (local, remote) = (encode(local), encode(remote));
    table.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        (fields.len() > 9 && fields[1] == local && fields[2] == remote)
            .then(|| fields[9].parse().ok())
            .flatten()
    })
}

/// Every process under `proc_root` holding `socket:[inode]`.
pub fn holders(proc_root: &Path, inode: u64) -> Holders {
    let target = format!("socket:[{inode}]");
    let mut found = Holders::default();
    let Ok(entries) = std::fs::read_dir(proc_root) else {
        return found;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(fds) = std::fs::read_dir(entry.path().join("fd")) else {
            found.unreadable += 1;
            continue;
        };
        if fds.flatten().any(|fd| {
            std::fs::read_link(fd.path()).is_ok_and(|link| link.as_os_str() == target.as_str())
        }) {
            found.pids.push(pid);
        }
    }
    found.pids.sort_unstable();
    found
}

/// The bridge's decision for one accepted connection.
#[derive(Debug, PartialEq, Eq)]
pub enum Owner {
    Harness,
    Other(String),
}

/// Decides whether the peer socket of `stream` belongs to `harness` alone.
pub fn owner(proc_root: &Path, stream: &TcpStream, harness: u32) -> Owner {
    let (Ok(SocketAddr::V4(peer)), Ok(SocketAddr::V4(local))) =
        (stream.peer_addr(), stream.local_addr())
    else {
        return Owner::Other("peer is not IPv4 loopback".into());
    };
    let Ok(table) = std::fs::read_to_string(proc_root.join("net/tcp")) else {
        return Owner::Other("cannot read net/tcp".into());
    };
    let Some(inode) = find_inode(&table, peer, local) else {
        return Owner::Other(format!("no socket for peer port {}", peer.port()));
    };
    let found = holders(proc_root, inode);
    if found.pids == [harness] {
        return Owner::Harness;
    }
    let describe = |pid: &u32| {
        let exe = std::fs::read_link(proc_root.join(pid.to_string()).join("exe"))
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| "?".into());
        format!("pid={pid} exe={exe}")
    };
    let holders: Vec<String> = found
        .pids
        .iter()
        .filter(|&&pid| pid != harness)
        .map(describe)
        .collect();
    Owner::Other(if holders.is_empty() {
        format!("no holder found (unreadable={})", found.unreadable)
    } else {
        holders.join(" ")
    })
}

pub fn listen() -> std::io::Result<TcpListener> {
    TcpListener::bind(crate::BRIDGE)
}

/// Serves the bridge on a background thread. `harness` is 0 until init
/// has spawned the harness, which refuses every connection.
pub fn serve(listener: TcpListener, harness: Arc<AtomicU32>, proxy: PathBuf, proc_root: PathBuf) {
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let (harness, proxy, proc_root) =
                (Arc::clone(&harness), proxy.clone(), proc_root.clone());
            thread::spawn(move || {
                let pid = harness.load(Ordering::SeqCst);
                let verdict = if pid == 0 {
                    Owner::Other("harness not started".into())
                } else {
                    owner(&proc_root, &stream, pid)
                };
                let Ok(mut upstream) = UnixStream::connect(&proxy) else {
                    return;
                };
                match verdict {
                    Owner::Harness => relay(stream, upstream),
                    Owner::Other(detail) => {
                        let _ = write!(upstream, "{DENY_OWNER} {detail}\r\n\r\n");
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                }
            });
        }
    });
}

fn relay(client: TcpStream, upstream: UnixStream) {
    let (Ok(mut client_read), Ok(mut upstream_read)) = (client.try_clone(), upstream.try_clone())
    else {
        return;
    };
    let (mut client_write, mut upstream_write) = (client, upstream);
    let up = thread::spawn(move || {
        let _ = copy(&mut client_read, &mut upstream_write);
        let _ = upstream_write.shutdown(Shutdown::Write);
    });
    let _ = copy(&mut upstream_read, &mut client_write);
    let _ = client_write.shutdown(Shutdown::Write);
    let _ = up.join();
}

fn copy(from: &mut impl Read, to: &mut impl Write) -> std::io::Result<u64> {
    std::io::copy(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn net_tcp_rows_map_to_inodes() {
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n   0: 0100007F:0C38 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 111 1 0 100 0 0 10 0\n   1: 0100007F:C350 0100007F:0C38 01 00000000:00000000 00:00000000 00000000  1000        0 222 1 0 20 4 30 10 -1\n";
        let loopback = Ipv4Addr::LOCALHOST;
        if u32::from_ne_bytes(loopback.octets()) != 0x0100_007f {
            return; // The fixture is written for little-endian hosts.
        }
        assert_eq!(
            find_inode(
                table,
                SocketAddrV4::new(loopback, 0xC350),
                SocketAddrV4::new(loopback, 3128)
            ),
            Some(222)
        );
        assert_eq!(
            find_inode(
                table,
                SocketAddrV4::new(loopback, 1),
                SocketAddrV4::new(loopback, 3128)
            ),
            None
        );
    }

    #[test]
    fn the_owner_check_names_the_holding_process() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        let me = std::process::id();
        assert_eq!(owner(Path::new("/proc"), &accepted, me), Owner::Harness);
        match owner(Path::new("/proc"), &accepted, me + 1) {
            Owner::Other(detail) => assert!(detail.contains(&format!("pid={me}")), "{detail}"),
            Owner::Harness => panic!("a different harness PID must not match"),
        }
        drop(client);
    }
}
