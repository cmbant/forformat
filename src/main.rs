use findent::{
    cli::{self, Command},
    format_to_owned, FormatError,
};
use std::io::{self, Read, Write};

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
        Command::Run(config) => {
            let mut input = Vec::new();
            if let Err(e) = io::stdin().read_to_end(&mut input) {
                eprintln!("findent: {e}");
                std::process::exit(1)
            }
            let mut out = io::BufWriter::with_capacity(64 * 1024, io::stdout());
            let result = format_to_owned(input, &config, &mut out);
            if let Err(e) = result.as_ref() {
                if !matches!(&e, FormatError::Write(error) if error.kind() == io::ErrorKind::BrokenPipe)
                {
                    eprintln!("findent: {e}");
                    std::process::exit(1)
                }
            } else if let Err(e) = out.flush().map_err(FormatError::Write) {
                if !matches!(&e, FormatError::Write(error) if error.kind() == io::ErrorKind::BrokenPipe)
                {
                    eprintln!("findent: {e}");
                    std::process::exit(1)
                }
            } else if let Ok(meta) = result {
                for (line, reason) in meta.declines {
                    eprintln!("findent: declined wrap at line {}: {reason:?}", line + 1);
                }
            }
        }
    }
}
