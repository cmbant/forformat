use crate::{config::FormatConfig, error::FormatError};

pub const VERSION: &str = "findent 0.1.0";

pub enum Command {
    Run(FormatConfig),
    Help,
    Version,
}

pub fn parse<I>(args: I) -> Result<Command, FormatError>
where
    I: IntoIterator<Item = String>,
    I::IntoIter: Iterator<Item = String>,
{
    let mut a = args.into_iter();
    let _program = a.next();
    let mut c = FormatConfig::default();
    let mut help = false;
    let mut version = false;
    let mut options_ended = false;
    while let Some(arg) = a.next() {
        if arg == "--" {
            options_ended = true;
            continue;
        }
        if options_ended {
            return Err(FormatError::InvalidOption(arg));
        }
        if arg == "-h" || arg == "--help" {
            help = true;
            continue;
        }
        if arg == "-v" || arg == "--version" {
            version = true;
            continue;
        }
        if arg == "-lastindent" {
            c.last_indent = true;
            continue;
        }
        if arg == "-lastusable" {
            c.last_usable = true;
            continue;
        }
        if arg == "-ifree"
            || arg == "-ofree"
            || arg == "-osame"
            || arg == "--input-format=free"
            || arg == "--output-format=free"
        {
            continue;
        }
        if arg == "-ifixed"
            || arg == "-ofixed"
            || arg == "-iauto"
            || arg == "--input-format=fixed"
            || arg == "--output-format=fixed"
        {
            return Err(FormatError::Unsupported(
                "fixed-form input/output is not supported".into(),
            ));
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (name, val) = if let Some((n, v)) = long.split_once('=') {
                (
                    n.replace('_', "-").to_ascii_lowercase(),
                    Some(v.to_string()),
                )
            } else {
                (long.replace('_', "-").to_ascii_lowercase(), None)
            };
            let mut value = val;
            let need = |v: &mut Option<String>,
                        a: &mut I::IntoIter|
             -> Result<String, FormatError> {
                if let Some(x) = v.take() {
                    Ok(x)
                } else {
                    a.next()
                        .ok_or_else(|| FormatError::InvalidOption("missing option value".into()))
                }
            };
            match name.as_str() {
                "indent" => {
                    let v = need(&mut value, &mut a)?;
                    if v == "none" {
                        c.apply_indent = false
                    } else {
                        c.indent = parse_num(&v)?;
                        c.construct_indents.set_all(c.indent);
                        c.contains_indent = c.indent;
                        c.continuation_indent = c.indent;
                        c.case_indent = c.indent.saturating_sub(c.indent / 2);
                        c.entry_indent = c.case_indent
                    }
                }
                "start-indent" => set_start(&mut c, &need(&mut value, &mut a)?)?,
                "indent-contains" => {
                    let v = need(&mut value, &mut a)?;
                    if v == "restart" {
                        c.contains_restart = true
                    } else {
                        c.contains_indent = parse_num(&v)?
                    }
                }
                "include-left" => c.include_left = parse_bool(&need(&mut value, &mut a)?)?,
                "label-left" => c.label_left = parse_bool(&need(&mut value, &mut a)?)?,
                "max-indent" => c.max_indent = parse_num(&need(&mut value, &mut a)?)?,
                "openmp" => c.openmp = parse_bool(&need(&mut value, &mut a)?)?,
                "indent-ampersand" => {
                    c.indent_ampersand = value
                        .as_deref()
                        .map(parse_bool)
                        .transpose()?
                        .unwrap_or(true)
                }
                "indent-continuation" => {
                    let v = need(&mut value, &mut a)?;
                    if v == "none" || v == "-" {
                        c.indent_continuation = false;
                    } else if v == "default" || v == "d" {
                        c.indent_continuation = true;
                    } else {
                        c.continuation_indent = parse_num(&v)?;
                    }
                }
                "align-paren" => {
                    c.align_paren_value = value.as_deref().map(parse_num).transpose()?.unwrap_or(1);
                    c.align_paren = c.align_paren_value != 0;
                }
                "ws-remred" => {
                    c.ws_remred_value = value.as_deref().map(parse_num).transpose()?.unwrap_or(1);
                    c.ws_remred = c.ws_remred_value != 0;
                }
                "last-indent" => c.last_indent = true,
                "last-usable" => c.last_usable = true,
                "indent-changeteam" => {
                    c.construct_indents.changeteam = parse_num(&need(&mut value, &mut a)?)?
                }
                "refactor-end" | "refactor-procedures" => {
                    c.refactor_end = true;
                    c.uppercase_end = value.as_deref() == Some("upcase")
                }
                n if n.starts_with("indent-") => {
                    let v = parse_num(&need(&mut value, &mut a)?)?;
                    set_construct(&mut c, n.trim_start_matches("indent-"), v)?
                }
                "input-format" => match need(&mut value, &mut a)?.to_ascii_lowercase().as_str() {
                    "free" => {}
                    "fixed" | "auto" => {
                        return Err(FormatError::Unsupported(
                            "fixed-form input/output is not supported".into(),
                        ))
                    }
                    other => {
                        return Err(FormatError::InvalidOption(format!(
                            "--input-format={other}"
                        )))
                    }
                },
                "output-format" => match need(&mut value, &mut a)?.to_ascii_lowercase().as_str() {
                    "free" | "same" => {}
                    "fixed" => {
                        return Err(FormatError::Unsupported(
                            "fixed-form input/output is not supported".into(),
                        ))
                    }
                    other => {
                        return Err(FormatError::InvalidOption(format!(
                            "--output-format={other}"
                        )))
                    }
                },
                _ => return Err(FormatError::InvalidOption(format!("--{name}"))),
            }
            continue;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            let b = arg.as_bytes();
            let ch = b[1] as char;
            let value = &arg[2..];
            match ch {
                'a' | 'b' | 'c' | 'd' | 'e' | 'E' | 'f' | 'F' | 'j' | 'm' | 'r' | 's' | 't'
                | 'w' | 'x' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption(format!("-{ch} requires a value"))
                        })?
                    } else {
                        value.to_string()
                    };
                    let n = parse_num(&v)?;
                    set_short(&mut c, ch, n)?
                }
                'C' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-C requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    if v == "-" {
                        c.contains_restart = true
                    } else {
                        c.contains_indent = parse_num(&v)?;
                        c.contains_restart = false
                    }
                }
                'i' => {
                    if value == "-" {
                        c.apply_indent = false
                    } else if value == "free" {
                    } else if value == "auto" {
                        return Err(FormatError::Unsupported(
                            "automatic fixed/free format detection is not supported".into(),
                        ));
                    } else if value == "fixed" {
                        return Err(FormatError::Unsupported(
                            "fixed-form input/output is not supported".into(),
                        ));
                    } else {
                        let v = if value.is_empty() {
                            a.next().ok_or_else(|| {
                                FormatError::InvalidOption("-i requires a value".into())
                            })?
                        } else {
                            value.to_string()
                        };
                        c.indent = parse_num(&v)?;
                        c.construct_indents.set_all(c.indent);
                        c.contains_indent = c.indent;
                        c.continuation_indent = c.indent;
                        c.case_indent = c.indent.saturating_sub(c.indent / 2);
                        c.entry_indent = c.case_indent
                    }
                }
                'I' => {
                    let start_value = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-I requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    set_start(&mut c, &start_value)?;
                }
                'k' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-k requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    if v == "-" || v == "none" {
                        c.indent_continuation = false
                    } else if v == "d" || v == "default" {
                        c.indent_continuation = true
                    } else {
                        c.continuation_indent = parse_num(&v)?
                    }
                }
                'K' => c.indent_ampersand = true,
                'l' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-l requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    c.label_left = parse_bool(&v)?;
                }
                'M' => {
                    let v = if value.is_empty() {
                        a.next().ok_or_else(|| {
                            FormatError::InvalidOption("-M requires a value".into())
                        })?
                    } else {
                        value.to_string()
                    };
                    c.max_indent = parse_num(&v)?;
                }
                'R' => {
                    c.refactor_end = true;
                    c.uppercase_end = value == "R" || value == "r" && arg == "-RR"
                }
                _ => return Err(FormatError::InvalidOption(arg)),
            }
            continue;
        }
        return Err(FormatError::InvalidOption(arg));
    }
    if help {
        Ok(Command::Help)
    } else if version {
        Ok(Command::Version)
    } else {
        Ok(Command::Run(c))
    }
}

fn parse_num(s: &str) -> Result<usize, FormatError> {
    s.parse::<isize>()
        .ok()
        .filter(|x| *x >= 0)
        .map(|x| x as usize)
        .ok_or_else(|| {
            FormatError::InvalidOption(format!("expected non-negative integer, got {s}"))
        })
}
fn parse_bool(s: &str) -> Result<bool, FormatError> {
    match s {
        "0" | "false" | "no" => Ok(false),
        "1" | "true" | "yes" => Ok(true),
        _ => Err(FormatError::InvalidOption(format!(
            "expected 0 or 1, got {s}"
        ))),
    }
}
fn set_start(c: &mut FormatConfig, s: &str) -> Result<(), FormatError> {
    if s.eq_ignore_ascii_case("a") || s.eq_ignore_ascii_case("auto") {
        c.auto_start_indent = true
    } else {
        c.start_indent = parse_num(s)?;
        c.auto_start_indent = false
    }
    Ok(())
}
fn set_short(c: &mut FormatConfig, ch: char, n: usize) -> Result<(), FormatError> {
    match ch {
        'a' => c.construct_indents.associate = n,
        'b' => c.construct_indents.block = n,
        'c' => c.case_indent = n,
        'd' => c.construct_indents.do_ = n,
        'e' => c.entry_indent = n,
        'E' => c.construct_indents.r#enum = n,
        'f' => c.construct_indents.if_ = n,
        'F' => c.construct_indents.forall = n,
        'j' => c.construct_indents.interface = n,
        'm' => c.construct_indents.module = n,
        'r' => c.construct_indents.procedure = n,
        's' => c.construct_indents.select = n,
        't' => c.construct_indents.r#type = n,
        'w' => c.construct_indents.where_ = n,
        'x' => c.construct_indents.critical = n,
        _ => return Err(FormatError::InvalidOption(format!("-{ch}"))),
    }
    Ok(())
}
fn set_construct(c: &mut FormatConfig, n: &str, v: usize) -> Result<(), FormatError> {
    match n {
        "associate" => c.construct_indents.associate = v,
        "block" => c.construct_indents.block = v,
        "case" => c.case_indent = v,
        "contains" => c.contains_indent = v,
        "do" => c.construct_indents.do_ = v,
        "entry" => c.entry_indent = v,
        "enum" => c.construct_indents.r#enum = v,
        "forall" => c.construct_indents.forall = v,
        "if" => c.construct_indents.if_ = v,
        "interface" => c.construct_indents.interface = v,
        "module" => c.construct_indents.module = v,
        "procedure" => c.construct_indents.procedure = v,
        "select" => c.construct_indents.select = v,
        "type" => c.construct_indents.r#type = v,
        "where" => c.construct_indents.where_ = v,
        "critical" => c.construct_indents.critical = v,
        _ => return Err(FormatError::InvalidOption(format!("--indent-{n}"))),
    }
    Ok(())
}

pub fn usage() -> &'static str {
    "Usage: findent [OPTIONS] < input > output\n\n\
Free-form Fortran formatter.\n\
  -i<n>, --indent=<n>                 global indentation (default 3)\n\
  -i-, --indent=none                  leave indentation unchanged\n\
  -I<n>, --start-indent=<n>           starting indentation\n\
  -Ia, --start-indent=a               infer starting indentation\n\
  -M<n>, --max-indent=<n>             maximum indentation (0 = unlimited)\n\
  -k<n>, --indent-continuation=<n>    continuation indentation\n\
  -K, --indent-ampersand              indent leading continuation ampersands\n\
  --align-paren[=<n>]                align continuation lines at parentheses\n\
  --include-left=<0|1>               put INCLUDE at the starting indent\n\
  -Rr, -RR, --refactor-end[=upcase]  complete END definition statements\n\
  --ws-remred[=<n>]                  reduce redundant whitespace\n\
  -lastindent, -lastusable           print query result instead of source\n\
  -h, --help                         show this help\n\
  -v, --version                      show version\n\
Fixed-form input/output and automatic format detection are intentionally unsupported."
}

#[cfg(test)]
mod tests {
    use super::{parse, Command};

    fn run(args: &[&str]) -> crate::config::FormatConfig {
        let mut argv = vec!["findent".to_string()];
        argv.extend(args.iter().map(|arg| (*arg).to_string()));
        match parse(argv).unwrap() {
            Command::Run(config) => config,
            _ => panic!("expected a formatting command"),
        }
    }

    #[test]
    fn overloaded_short_options_accept_attached_and_separated_values() {
        let attached = run(&["-i4", "-C-", "-k5", "-M9"]);
        let separated = run(&["-i", "4", "-C", "-", "-k", "5", "-M", "9"]);
        assert_eq!(attached, separated);
        assert!(!run(&["-i-"]).apply_indent);
        assert!(run(&["-Ia"]).auto_start_indent);
        assert!(parse(["findent".to_string(), "-iauto".to_string()].into_iter()).is_err());
    }

    #[test]
    fn optional_values_are_not_taken_from_the_next_argument() {
        let bare = run(&["--align_paren"]);
        assert!(bare.align_paren);
        assert_eq!(bare.align_paren_value, 1);
        assert!(!run(&["--align-paren=0"]).align_paren);
        assert_eq!(run(&["--align_paren=4"]).align_paren_value, 4);
        assert!(run(&["--ws-remred"]).ws_remred);
        assert_eq!(run(&["--ws_remred=0"]).ws_remred_value, 0);
    }

    #[test]
    fn format_aliases_and_option_termination_are_explicit() {
        assert!(matches!(
            parse(["findent".to_string(), "--input_format=free".to_string()].into_iter()).unwrap(),
            Command::Run(_)
        ));
        assert!(matches!(
            parse(["findent".to_string(), "--output-format=same".to_string()].into_iter()).unwrap(),
            Command::Run(_)
        ));
        assert!(
            parse(["findent".to_string(), "--".to_string(), "-i4".to_string()].into_iter())
                .is_err()
        );
    }

    #[test]
    fn long_and_short_construct_aliases_produce_the_same_config() {
        assert_eq!(
            run(&[
                "-a5", "-b6", "-c2", "-d7", "-e4", "-E8", "-F9", "-f5", "-j6", "-m7", "-r8", "-s9",
                "-t4", "-w5", "-x6"
            ]),
            run(&[
                "--indent-associate=5",
                "--indent-block=6",
                "--indent-case=2",
                "--indent-do=7",
                "--indent-entry=4",
                "--indent-enum=8",
                "--indent-forall=9",
                "--indent-if=5",
                "--indent-interface=6",
                "--indent-module=7",
                "--indent-procedure=8",
                "--indent-select=9",
                "--indent-type=4",
                "--indent-where=5",
                "--indent-critical=6",
            ])
        );
        assert_eq!(run(&["-C-"]), run(&["--indent-contains=restart"]));
        assert_eq!(run(&["-K"]), run(&["--indent-ampersand"]));
        assert_eq!(run(&["-Rr"]), run(&["--refactor-end"]));
        assert_eq!(run(&["-RR"]), run(&["--refactor-end=upcase"]));
    }

    #[test]
    fn long_alias_spellings_and_optional_values_have_a_matrix() {
        let aliases = [
            ("--indent_associate=5", "--indent-associate=5"),
            ("--indent_contains=restart", "--indent-contains=restart"),
            ("--include_left=1", "--include-left=1"),
            ("--label_left=0", "--label-left=0"),
            ("--input_format=free", "--input-format=free"),
            ("--output_format=same", "--output-format=same"),
        ];
        for (underscore, hyphen) in aliases {
            assert_eq!(
                run(&[underscore]),
                run(&[hyphen]),
                "{underscore} != {hyphen}"
            );
        }

        assert_eq!(run(&["--align_paren"]).align_paren_value, 1);
        assert!(!run(&["--align-paren=0"]).align_paren);
        assert!(run(&["--align_paren=1"]).align_paren);
        assert_eq!(run(&["--align-paren=7"]).align_paren_value, 7);
        assert_eq!(run(&["--ws_remred"]).ws_remred_value, 1);
        assert!(!run(&["--ws-remred=0"]).ws_remred);
        assert!(run(&["--ws_remred=1"]).ws_remred);
    }

    #[test]
    fn every_documented_long_option_family_parses_with_a_value() {
        let options = [
            "--indent=4",
            "--start-indent=2",
            "--indent-contains=4",
            "--include-left=1",
            "--label-left=0",
            "--max-indent=12",
            "--openmp=0",
            "--indent-ampersand",
            "--indent-continuation=7",
            "--align-paren=3",
            "--ws-remred=1",
            "--indent-changeteam=4",
            "--indent-associate=4",
            "--indent-block=4",
            "--indent-case=4",
            "--indent-contains=4",
            "--indent-do=4",
            "--indent-entry=4",
            "--indent-enum=4",
            "--indent-forall=4",
            "--indent-if=4",
            "--indent-interface=4",
            "--indent-module=4",
            "--indent-procedure=4",
            "--indent-select=4",
            "--indent-type=4",
            "--indent-where=4",
            "--refactor-end",
            "--refactor-end=upcase",
            "--last-indent",
            "--last-usable",
            "--input-format=free",
            "--output-format=free",
            "--output-format=same",
        ];
        for option in options {
            assert!(
                parse(["findent".to_string(), option.to_string()].into_iter()).is_ok(),
                "{option}"
            );
        }
    }

    #[test]
    fn missing_and_invalid_long_values_have_stable_diagnostics() {
        for option in [
            "--indent",
            "--start-indent",
            "--indent-contains",
            "--include-left",
            "--label-left",
            "--max-indent",
            "--openmp",
            "--indent-continuation",
            "--indent-changeteam",
            "--indent-if",
        ] {
            match parse(["findent".to_string(), option.to_string()]) {
                Err(crate::error::FormatError::InvalidOption(message)) => {
                    assert_eq!(message, "missing option value", "{option}")
                }
                _ => panic!("unexpected result for {option}"),
            }
        }
        for option in ["--include-left=2", "--label-left=maybe", "--openmp=maybe"] {
            assert!(matches!(
                parse(["findent".to_string(), option.to_string()].into_iter()),
                Err(crate::error::FormatError::InvalidOption(_))
            ));
        }
    }

    #[test]
    fn rejected_long_values_have_stable_diagnostics() {
        for (args, expected) in [
            (&["--input-format=unknown"][..], "--input-format=unknown"),
            (&["--output_format=unknown"][..], "--output-format=unknown"),
            (
                &["--align_paren=-1"][..],
                "expected non-negative integer, got -1",
            ),
            (
                &["--ws_remred=no"][..],
                "expected non-negative integer, got no",
            ),
        ] {
            match parse(
                std::iter::once("findent".to_string())
                    .chain(args.iter().map(|arg| (*arg).to_string())),
            ) {
                Err(crate::error::FormatError::InvalidOption(value)) => {
                    assert_eq!(value, expected)
                }
                _ => panic!("unexpected result for {args:?}"),
            }
        }
    }

    #[test]
    fn unsupported_and_invalid_cli_paths_have_stable_categories() {
        assert!(matches!(
            parse(["findent".to_string(), "-ifixed".to_string()].into_iter()),
            Err(crate::error::FormatError::Unsupported(_))
        ));
        assert!(matches!(
            parse(["findent".to_string(), "--not-an-option".to_string()].into_iter()),
            Err(crate::error::FormatError::InvalidOption(_))
        ));
        assert!(matches!(
            parse(["findent".to_string(), "-i".to_string()].into_iter()),
            Err(crate::error::FormatError::InvalidOption(_))
        ));
        assert!(matches!(
            parse(["findent".to_string(), "--include-left=maybe".to_string()].into_iter()),
            Err(crate::error::FormatError::InvalidOption(_))
        ));
    }
}
