use std::ffi::OsString;

fn main() {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    if args.is_empty() {
        args.push(OsString::from("battery-monitor-report"));
    }
    args.insert(1, OsString::from("report"));
    if let Err(err) = battery_monitor::cli::run(args) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
