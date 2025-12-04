use std::env;

fn main() {
    if let Err(err) = symmetri::cli::run(env::args_os()) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
