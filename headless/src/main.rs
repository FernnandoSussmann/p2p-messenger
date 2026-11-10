#![forbid(unsafe_code)]

#[cfg(not(unix))]
fn main() {
    eprintln!("messenger-headless is supported on unix only");
    std::process::exit(2);
}

#[cfg(unix)]
#[tokio::main]
async fn main() {
    use messenger_headless::cli::{self, Command};
    use messenger_headless::{control, node};

    let command = match cli::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(e) => {
            eprintln!("messenger-headless: {e}");
            std::process::exit(2);
        }
    };
    let code = match command {
        Command::Ctl { data_dir, command } => control::ctl(&data_dir, &command).await,
        Command::Run(args) => match node::run(args).await {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("messenger-headless: {e}");
                1
            }
        },
    };
    std::process::exit(code);
}
