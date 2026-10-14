#![forbid(unsafe_code)]

#[cfg(not(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64"))))]
compile_error!("Venus supports only Linux and Apple Silicon macOS");

mod application;
mod launch;

use std::{env, error::Error};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

fn main() -> Result {
    let arguments = launch::launch_arguments(
        env::args_os().skip(1),
        env::var_os("EON_VENUS_PRESENTATION_CONTROL"),
    )?;
    application::run(arguments)
}
