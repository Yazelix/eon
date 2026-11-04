mod product;

use std::process::ExitCode;

fn main() -> ExitCode {
    let (product, result) = eon_runtime::run(product::inputs());
    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("{product}: {}", error.message);
            ExitCode::from(error.status)
        }
    }
}
