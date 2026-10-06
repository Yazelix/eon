mod product;

use std::{process::ExitCode, thread, time::Duration};

fn main() -> ExitCode {
    let (product, result) = eon_runtime::run(product::inputs());
    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("{product}: {error}");
            if product == "eon-directory-picker" {
                thread::sleep(Duration::from_secs(2));
            }
            ExitCode::FAILURE
        }
    }
}
