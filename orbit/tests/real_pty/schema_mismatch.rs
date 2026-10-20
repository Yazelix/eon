use crate::support::*;
use std::{env, os::unix::fs::symlink};

/// The digests a refusal names, in order.
fn digests(message: &str) -> Vec<&str> {
    message
        .split("\"schema ")
        .skip(1)
        .filter_map(|rest| rest.get(..16))
        .filter(|digest| digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .collect()
}

pub fn run() {
    let directory = TempDir::new();
    // A worker generated from a schema with one more flag (nix/orbit-checks.nix builds
    // it).
    let worker = env::var_os("MISMATCHED_WORKER")
        .expect("MISMATCHED_WORKER names a worker generated from another schema");
    symlink(worker, directory.join("yazelix-orbit-worker")).expect("link worker");
    let refused = run_unopened(&directory);

    // The host refuses the worker with a message naming both schemas, before
    // it binds the socket or starts the child.
    refused.assert_refused();
    let message = String::from_utf8_lossy(&refused.stderr);
    let named = digests(&message);
    assert!(
        named.len() == 2 && named[0] != named[1],
        "{message:?} should name two schemas"
    );
}
