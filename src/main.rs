use std::env;

fn main() {
    if let Err(err) = battery_monitor::cli::run(env::args_os()) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
