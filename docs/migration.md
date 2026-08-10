# Migrating from findent 4.3.7

The Rust binary keeps the stdin/stdout workflow and free-form structural indentation. Use
`findent -ifree < source.f90 > formatted.f90`; `-ifree`, `-ofree`, and `-osame` are accepted as
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
| `-ifixed`, `-ofixed`, `-iauto`, `--continuation` | rejected with status 2 |
| `--relabel`, `--deps`, editor wrappers, `--safe`, `--selfrep` | not implemented |

Rust intentionally does not read `FINDENT_FLAGS` and rejects unknown options. By default it matches
the legacy formatter's removal of trailing spaces/tabs while preserving other source spelling and
non-trailing body bytes. Use `--ws_remred` or `--ws_remred=1` for broader redundant-whitespace reduction; `=0`
disables it. The formatter does not accept filenames; pipe source through stdin and capture stdout.
