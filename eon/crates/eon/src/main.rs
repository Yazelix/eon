mod product;
mod report;
mod update;

use std::process::ExitCode;

fn main() -> ExitCode {
    let inputs = product::inputs();
    let arguments: Vec<_> = std::env::args_os().collect();
    #[cfg(target_os = "linux")]
    if arguments.first().is_some_and(|program| {
        std::path::Path::new(program).file_name() == Some(std::ffi::OsStr::new("eon"))
    }) && arguments.get(1).is_some_and(|command| command == "agent")
    {
        let primary =
            eon_runtime::configuration_directory().map(|root| Some(root.join("agent.toml")));
        return rota::cli::main(arguments.into_iter().skip(2), primary);
    }
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
