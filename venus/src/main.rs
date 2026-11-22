#![forbid(unsafe_code)]

#[cfg(not(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64"))))]
compile_error!("Venus supports only Linux and Apple Silicon macOS");

fn main() -> Result<(), Box<dyn std::error::Error>> {
    yazelix_venus::run_venus()
}
