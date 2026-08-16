# Migrating from findent 4.3.7

The Rust binary keeps the stdin/stdout workflow and free-form structural indentation. The default
is now `FormatMode::Full`: it retains findent-compatible indentation and adds lexical normalization
and wrapping. Use `--indent-only` for the findent-compatible indentation contract, or spell
`--full` explicitly when adopting the additions. `-ifree`, `-ofree`, and `-osame` remain accepted as
free-form compatibility no-ops.

| Legacy option family | Rust behavior |
| --- | --- |
| `-i`, `-I`, `-M`, `-l`, `-C`, `-c`, `-e` | supported, including attached and separated short values |
| `-a`, `-b`, `-d`, `-E`, `-F`, `-f`, `-j`, `-m`, `-r`, `-s`, `-t`, `-w`, `-x` | supported |
| `--indent-associate`, `--indent-block`, `--indent-case`, `--indent-contains`, `--indent-do`, `--indent-entry`, `--indent-enum`, `--indent-forall`, `--indent-if`, `--indent-interface`, `--indent-module`, `--indent-procedure`, `--indent-select`, `--indent-type`, `--indent-where`, `--indent-changeteam` | supported; `_` and `-` spellings are equivalent |
| `--start-indent`, `--max-indent`, `--label-left`, `--include-left`, `--openmp` | supported |
| `-k`, `-K`, `--indent-continuation`, `--indent-ampersand`, `--align_paren` | supported for free form |
| `-Rr`, `-RR`, `--ws_remred` | supported as explicit transformations; bare `--ws_remred` enables it |
| `-i-`, `--indent=none` | supported; preserve source indentation while retaining other formatting contracts |
| `-lastindent`, `-lastusable` | supported |
| `-ifixed`, `-ofixed`, `--continuation` | rejected with status 2 |
| `-iauto`, `--input-format=auto` | supported; automatic fixed/free input detection is the default |
| `-ifree`, `--input-format=free` | supported; forces free-form handling, bypassing detection |
| `--query-fix-free` equivalent: `--query-format` | supported; prints `free` or `fixed` per input |
| `--relabel`, `--deps`, editor wrappers, `--safe`, `--selfrep` | not implemented |

Rust intentionally does not read `FINDENT_FLAGS` and rejects unknown options. `--indent-only`
matches the legacy formatter's removal of trailing spaces/tabs while preserving other source
spelling and non-trailing body bytes. Full mode additionally normalizes keywords, separators,
comments, array constructors, declaration-driven names and kind suffixes, and can wrap statements.
Use `--ws_remred` or `--ws_remred=1` for broader redundant-whitespace reduction; `=0` disables it.
The formatter does not accept filenames on stdin; pipe source through stdin and capture stdout.

Full mode's intentional divergences are centralized in [docs/compatibility.md](compatibility.md):
array constructors, conservative comment bodies, kind suffixes on continuation lines, the governing
declaration rule versus the reference ambiguity veto, `!$` sentinel spacing, and preservation of a
valid literal under `--ws_remred`.
