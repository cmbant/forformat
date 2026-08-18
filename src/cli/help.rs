use super::options::OPTIONS;
use std::sync::OnceLock;

pub fn usage() -> &'static str {
    static USAGE: OnceLock<String> = OnceLock::new();
    USAGE.get_or_init(render_usage).as_str()
}

fn render_usage() -> String {
    let mut output =
        String::from("Usage: forformat [OPTIONS] [PATH ...]\n\nFree-form Fortran formatter.\n");
    for option in OPTIONS {
        let Some(help) = option.help else {
            continue;
        };
        if help.syntax.len() >= 38 {
            output.push_str("  ");
            output.push_str(help.syntax);
            output.push('\n');
            output.push_str("                                          ");
            output.push_str(help.description);
            output.push('\n');
        } else {
            output.push_str(&format!("  {:<38} {}\n", help.syntax, help.description));
        }
    }
    output.push_str(
        "  Query modes cannot be combined with path-update, --check, or --diff.\n\
Automatic fixed/free input detection is enabled by default; use -ifree or\n\
--input-format=free to force free-form input. Fixed-form output remains unsupported.\n\
A single directory PATH behaves like --all-files DIR.",
    );
    output
}
