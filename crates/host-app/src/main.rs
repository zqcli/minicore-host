//! The application owns process facilities; business execution stays in Host.
#![forbid(unsafe_code)]

mod cli;
mod diagnostics;
mod process;
mod signals;
mod transport;

use std::process::ExitCode;

fn main() -> ExitCode {
    // Panic payloads can include private configuration or execution content.
    std::panic::set_hook(Box::new(|_| {
        diagnostics::panic();
    }));
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("minicore-host: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), &'static str> {
    let Some(options) = cli::Options::parse(std::env::args_os().skip(1))? else {
        eprintln!("{}", cli::HELP);
        return Ok(());
    };
    let initial = options.initial_config()?;
    // A local Host waits on I/O; it needs no thread/allocator arena per CPU.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|_| "cannot initialize the execution runtime")?;
    let mut diagnostics = None;
    let result = runtime.block_on(process::run(options, initial, &mut diagnostics));
    // The stopped barrier has joined business work. Tokio's parked stdin read
    // is not cancellable and must not hold an otherwise stopped process alive.
    runtime.shutdown_background();
    if let Some(diagnostics) = diagnostics {
        diagnostics.finish();
    }
    result
}
