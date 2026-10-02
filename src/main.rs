use std::process::ExitCode;

use koda::app::App;

fn main() -> ExitCode {
    // Koda is usable both as a library and as a CLI. The CLI accepts an optional
    // path: `koda`, `koda .`, or `koda path/to/file.rs`.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let target = args.into_iter().find(|a| !a.starts_with('-'));

    match App::start(target) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("koda: {err}");
            ExitCode::FAILURE
        }
    }
}
