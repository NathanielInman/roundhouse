//! `rh`: the Roundhouse daemon, CLI and waybar module.

// The daemon is Unix-only for now, and it is what formats bar lines.
#[cfg_attr(not(unix), allow(dead_code))]
mod bar;
mod cli;
#[cfg(unix)]
mod daemon;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match cli::run(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rh: {e:#}");
            1
        }
    };
    std::process::exit(code);
}
