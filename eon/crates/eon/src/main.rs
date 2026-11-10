mod product;
mod report;
mod update;

use std::process::ExitCode;

fn main() -> ExitCode {
    let inputs = product::inputs();
    let arguments: Vec<_> = std::env::args_os().collect();
    let invoked = || update::Build {
        version: Some(inputs.version.into()),
        build: eon_runtime::build_identity(&inputs).ok(),
        revision: None,
        platform: eon_manifest::target().map(Into::into),
    };
    if update::wanted(&arguments) {
        return ExitCode::from(update::run(&arguments, invoked(), &inputs.defaults));
    }
    if report::wanted(&arguments) {
        return ExitCode::from(report::run(&arguments, invoked(), &inputs));
    }
    let (product, result) = eon_runtime::run(inputs);
    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("{product}: {}", error.message);
            ExitCode::from(error.status)
        }
    }
}
