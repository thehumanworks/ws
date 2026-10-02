//! Entry point: wires the real process environment into [`ws::cli::run`].

use std::process::ExitCode;

fn main() -> ExitCode {
    let env = |key: &str| std::env::var(key).ok();
    let code = ws::cli::run(
        std::env::args_os(),
        &env,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    ExitCode::from(code)
}
