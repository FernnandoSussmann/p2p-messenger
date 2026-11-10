#![forbid(unsafe_code)]

#[cfg(not(unix))]
fn main() {
    eprintln!("messenger-headless is supported on unix only");
    std::process::exit(2);
}

#[cfg(unix)]
fn main() {
    use messenger_headless::cli;

    if let Err(e) = cli::parse(std::env::args().skip(1)) {
        eprintln!("messenger-headless: {e}");
        std::process::exit(2);
    }
    eprintln!("messenger-headless: node runtime is not wired yet");
    std::process::exit(1);
}
