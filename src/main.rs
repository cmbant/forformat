use findent::{
    cli::{self, Command},
    io,
};

fn main() {
    let command = match cli::parse(std::env::args()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("findent: {e}");
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
            Err(error) => {
                eprintln!("findent: {error}");
                std::process::exit(error.status());
            }
        },
    }
}
