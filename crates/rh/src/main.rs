//! `rh`: the Roundhouse daemon, CLI and waybar module.

// The daemon is Unix-only for now, and it is what formats bar lines.
#[cfg_attr(not(unix), allow(dead_code))]
mod bar;
mod cli;
#[cfg(unix)]
mod daemon;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `rh status | head` should end quietly, not panic on the closed pipe.
    // The daemon keeps Rust's ignored SIGPIPE: a drawer or bar that goes
    // away must not kill it.
    #[cfg(unix)]
    if !matches!(args.first().map(String::as_str), Some("daemon")) {
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }
    let code = match cli::run(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rh: {e:#}");
            1
        }
    };
    std::process::exit(code);
}
