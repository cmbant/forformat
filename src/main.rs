use forformat::{
    cli::{self, Command},
    io,
};

fn main() {
    let command = match cli::parse(std::env::args()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("forformat: {e}");
            std::process::exit(2)
        }
    };
    match command {
        Command::Help => {
            println!("{}", cli::usage())
        }
        Command::Version => println!("{}", cli::VERSION),
        Command::Run(invocation) => match io::execute(*invocation) {
            Ok(status) => std::process::exit(status),
            Err(error) if error.is_broken_pipe() => std::process::exit(0),
            Err(error) => {
                eprintln!("forformat: {error}");
                std::process::exit(error.status());
            }
        },
    }
}
