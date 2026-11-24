mod doctor;
mod launch;
#[allow(unsafe_code)]
mod platform;
mod product;
mod report;
mod tutor;
mod update;

use std::process::ExitCode;

fn main() -> ExitCode {
    let launch = launch::capture();
    // SAFETY: no safe API clears the process environment; main is still single-threaded,
    // before product initialization or any worker.
    #[allow(unsafe_code)]
    unsafe {
        launch::clear_capture()
    };
    let arguments: Vec<_> = std::env::args_os().collect();
    let product_cli = arguments.first().is_some_and(|program| {
        std::path::Path::new(program).file_name() == Some(std::ffi::OsStr::new("eon"))
            || (std::path::Path::new(program).file_name() != Some(std::ffi::OsStr::new("eonbase"))
                && std::env::current_exe()
                    .is_ok_and(|executable| executable.as_os_str() == program))
    });
    if product_cli
        && matches!(&arguments[1..], [flag, token] if flag == "--eon-initial-picker-header" && token == "initial")
    {
        return ExitCode::from(tutor::initial_picker_header());
    }
    let inputs = product::inputs(launch);
    if product_cli && arguments.get(1).is_some_and(|command| command == "tutor") {
        return ExitCode::from(tutor::run(&arguments[2..]));
    }
    #[cfg(target_os = "linux")]
    if product_cli && arguments.get(1).is_some_and(|command| command == "agent") {
        let session = |deadline| eon_runtime::quota_facts_json(&inputs, deadline);
        let host = rota::cli::Host {
            program: "eon agent",
            reinvoke: &["agent"],
            config: eon_runtime::configuration_directory()
                .map(|root| Some(root.join("agent.toml"))),
            config_label: Some("<eon config dir>/agent.toml"),
            slot_config_flag: true,
            report_config_file: true,
            session: &session,
            route: Some(&eon_boat::Boat::EON),
        };
        return rota::cli::main(arguments.into_iter().skip(2), &host);
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
    if product_cli
        && arguments.get(1).is_some_and(|command| command == "doctor")
        && !matches!(&arguments[2..], [flag] if flag == "-h" || flag == "--help")
    {
        return ExitCode::from(doctor::run(&arguments, invoked(), &inputs));
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
