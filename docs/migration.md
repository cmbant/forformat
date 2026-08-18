# Migrating from findent 4.3.7

The Rust binary keeps findent's stdin/stdout workflow and free-form structural indentation. The
default is `--full`, which adds lexical normalization and wrapping. Use `--indent-only` for the
findent-compatible indentation contract.

`-ofree` and `-osame` remain accepted compatibility no-ops for free-form output. `-ifree` is **not**
a no-op: automatic fixed/free detection is the default, and `-ifree` (or
`--input-format=free`) forces free-form handling and bypasses detection.

| Legacy option family | Rust behavior |
| --- | --- |
| `-i`, `-I`, `-M`, `-l`, `-C`, `-c`, `-e` | supported, including attached and separated short values |
| `-a`, `-b`, `-d`, `-E`, `-F`, `-f`, `-j`, `-m`, `-r`, `-s`, `-t`, `-w`, `-x` | supported |
| `--indent-associate`, `--indent-block`, `--indent-case`, `--indent-contains`, `--indent-do`, `--indent-entry`, `--indent-enum`, `--indent-forall`, `--indent-if`, `--indent-interface`, `--indent-module`, `--indent-procedure`, `--indent-select`, `--indent-type`, `--indent-where`, `--indent-critical`, `--indent-changeteam` | supported; `_` and `-` spellings are equivalent |
| `--start-indent`, `--max-indent`, `--label-left`, `--include-left`, `--openmp` | supported |
| `-k`, `-K`, `--indent-continuation`, `--indent-ampersand`, `--align_paren` | supported for free form |
| `-Rr`, `-RR`, `--refactor-end` | supported; `--refactor-procedures` is an alias |
| `--ws_remred` | supported; bare spelling enables it |
| `-i-`, `--indent=none` | supported; preserve authored leading indentation while retaining the selected mode's other behaviour |
| `-lastindent`, `-lastusable` | supported |
| `-iauto`, `--input-format=auto` | supported; automatic fixed/free input detection is the default |
| `-ifree`, `--input-format=free` | supported; forces free-form handling, bypassing detection |
| `-ofree`, `-osame`, free/same output long forms | accepted; output remains free form |
| `-ifixed`, `-ofixed`, fixed input/output long forms | unsupported; fail with status 2 |
| `--query-fix-free` equivalent: `--query-format` | supported; prints `free` or `fixed` per input |
| `--continuation` | unsupported; fail with status 2 |
| `--relabel`, `--deps`, editor wrappers, `--safe`, `--selfrep` | not implemented |

Rust intentionally does not read `FINDENT_FLAGS` and rejects unknown options. `--indent-only`
matches the legacy formatter's removal of trailing spaces/tabs while preserving other source
spelling and non-trailing body bytes. Full mode additionally normalizes keywords, separators,
comments, array constructors, declaration-driven names and kind suffixes, and can wrap statements.
Use `--ws_remred` or `--ws_remred=1` for broader redundant-whitespace reduction; `=0` disables it.

For the current native option names, defaults, configuration keys, and interactions, see
[options.md](options.md). Full mode's intentional divergences are centralized in
[compatibility.md](compatibility.md).
