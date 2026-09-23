#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    use clap::Parser;
    match fresco_example_engine_host::native::run(
        fresco_example_engine_host::native::Options::parse(),
    ) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(target_arch = "wasm32")]
compile_error!(
    "The native host is not a WASM executable; build the browser library with --no-default-features."
);
