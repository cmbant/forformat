from pathlib import Path


def replace(path, old, new, count=1):
    p = Path(path)
    text = p.read_text()
    actual = text.count(old)
    if actual != count:
        raise SystemExit(
            f"{path}: expected {count} occurrences, found {actual}: {old[:80]!r}"
        )
    p.write_text(text.replace(old, new, count))


# The CLI query is an output action, not a separate formatting pipeline.
p = Path("src/io/mod.rs")
text = p.read_text()
start = text.index("fn execute_indent_query(")
end = text.index("\nfn source_form_name", start)
text = text[:start] + text[end + 1 :]
text = text.replace(
    "    if let Some(query) = invocation.indent_query {\n"
    "        return execute_indent_query(&invocation, query);\n"
    "    }\n",
    "",
    1,
)
old = """    let result = in_input(format_source(&source, &invocation.config), None, None)?;
    let mut declines = DeclineReporter::default();
    declines.report(&result.meta, None, None);
    declines.finish();
    write_all_stdout(&result.bytes)?;
    Ok(0)
"""
new = """    let result = in_input(format_source(&source, &invocation.config), None, None)?;
    write_stdin_result(invocation, result, None)
"""
if text.count(old) != 1:
    raise SystemExit("src/io/mod.rs: bare-stdin result block changed")
text = text.replace(old, new, 1)
old = """        let mut declines = DeclineReporter::default();
        declines.report(&formatted.meta, None, scope.root.as_deref());
        declines.finish();
        write_all_stdout(&formatted.bytes)?;
        return Ok(Some(0));
"""
new = """        return write_stdin_result(invocation, formatted, scope.root.as_deref()).map(Some);
"""
if text.count(old) != 1:
    raise SystemExit("src/io/mod.rs: stdin shortcut result block changed")
text = text.replace(old, new, 1)
old = """    let mut declines = DeclineReporter::default();
    declines.report(&formatted.meta, None, scope.root.as_deref());
    declines.finish();
    write_all_stdout(&formatted.bytes)?;
    Ok(0)
}

/// Correlated immutable inputs shared by the prepared file-output routes.
"""
new = """    write_stdin_result(invocation, formatted, scope.root.as_deref())
}

/// Deliver one successfully formatted stdin buffer.
///
/// Indentation queries intentionally share every preparation step with ordinary
/// stdin formatting — source-form handling, project/context resolution,
/// normalization, wrapping, and layout. Only the final delivery differs: a
/// query prints one metadata value instead of the formatted bytes.
fn write_stdin_result(
    invocation: &Invocation,
    formatted: crate::FormatResult,
    root: Option<&Path>,
) -> Result<i32, WorkflowError> {
    let mut declines = DeclineReporter::default();
    declines.report(&formatted.meta, None, root);
    declines.finish();
    if let Some(query) = invocation.indent_query {
        let value = match query {
            IndentQuery::LastIndent => formatted.meta.last_indent,
            IndentQuery::LastUsable | IndentQuery::Both => formatted.meta.last_usable,
        };
        write_all_stdout(format!("{value}\\n").as_bytes())?;
    } else {
        write_all_stdout(&formatted.bytes)?;
    }
    Ok(0)
}

/// Correlated immutable inputs shared by the prepared file-output routes.
"""
if text.count(old) != 1:
    raise SystemExit("src/io/mod.rs: project-stdin result block changed")
text = text.replace(old, new, 1)
p.write_text(text)

# Delete the duplicated planner traversal. Normal formatting already computes
# exactly the metadata the CLI query needs.
p = Path("src/format/engine.rs")
text = p.read_text()
start = text.index("pub fn query(")
end = text.index("\npub fn format_buffer", start)
p.write_text(text[:start] + text[end + 1 :])

replace(
    "src/lib.rs",
    """    fn indentation_query_metadata_is_stable() {
        let config = indent_only_config();
        let empty = crate::format::engine::query(b"", &config).unwrap();
        assert_eq!((empty.last_indent, empty.last_usable), (0, 1));

        let meta = crate::format::engine::query(b"program p\\nx=1\\n", &config).unwrap();
        assert_eq!((meta.last_indent, meta.last_usable), (3, 2));
    }
""",
    """    fn formatting_metadata_is_stable() {
        let config = indent_only_config();
        let empty = format_source(b"", &config).unwrap().meta;
        assert_eq!((empty.last_indent, empty.last_usable), (0, 1));

        let meta = format_source(b"program p\\nx=1\\n", &config).unwrap().meta;
        assert_eq!((meta.last_indent, meta.last_usable), (3, 2));
    }
""",
)

# The manifest harness models a query the same way as the real CLI: normal
# formatting first, metadata-only delivery second.
p = Path("tests/manifest.rs")
text = p.read_text()
old = """                if let Some(query) = invocation.indent_query {
                    let meta = forformat::format::engine::query(&input, &invocation.config)
                        .expect("manifest indentation query succeeds");
                    let value = match query {
                        cli::IndentQuery::LastIndent => meta.last_indent,
                        cli::IndentQuery::LastUsable | cli::IndentQuery::Both => meta.last_usable,
                    };
                    (format!("{value}\\n").into_bytes(), String::new(), 0)
                } else {
                    let formatted = match case.project.as_str() {
                        "" => format_source(&input, &invocation.config),
                        "self" => {
                            let project =
                                analyze_project([(input_path.as_path(), input.as_slice())])
                                    .expect("manifest project analyzes");
                            format_source_with_context(&input, &project, &invocation.config)
                        }
                        other => panic!("unknown manifest project {other} in case {}", case.name),
                    };
                    match formatted {
                        Ok(result) => (result.bytes, String::new(), 0),
                        Err(error) => (Vec::new(), format_error(error), 1),
                    }
                }
"""
new = """                let formatted = match case.project.as_str() {
                    "" => format_source(&input, &invocation.config),
                    "self" => {
                        let project = analyze_project([(input_path.as_path(), input.as_slice())])
                            .expect("manifest project analyzes");
                        format_source_with_context(&input, &project, &invocation.config)
                    }
                    other => panic!("unknown manifest project {other} in case {}", case.name),
                };
                match formatted {
                    Ok(result) => {
                        let stdout = match invocation.indent_query {
                            Some(cli::IndentQuery::LastIndent) => {
                                format!("{}\\n", result.meta.last_indent).into_bytes()
                            }
                            Some(cli::IndentQuery::LastUsable | cli::IndentQuery::Both) => {
                                format!("{}\\n", result.meta.last_usable).into_bytes()
                            }
                            None => result.bytes,
                        };
                        (stdout, String::new(), 0)
                    }
                    Err(error) => (Vec::new(), format_error(error), 1),
                }
"""
if text.count(old) != 1:
    raise SystemExit("tests/manifest.rs: query harness block changed")
p.write_text(text.replace(old, new, 1))

# Focused path coverage: one valid context and the reported regression.
p = Path("tests/io_workflow.rs")
text = p.read_text()
marker = """    let both = run_stdin(&repo, &["-lastindent", "-lastusable"], source);
    assert_eq!(both.status.code(), Some(0));
    assert_eq!(both.stdout, b"2\\n");

"""
addition = marker + """    let contextual = run_stdin(
        &repo,
        &["-lastindent", "--project-context", "."],
        source,
    );
    assert_eq!(contextual.status.code(), Some(0));
    assert_eq!(contextual.stdout, b"3\\n");

    let missing_context = run_stdin(
        &repo,
        &["-lastindent", "--project-context", "DOES_NOT_EXIST"],
        source,
    );
    assert_eq!(missing_context.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing_context.stderr)
        .contains("--project-context path does not exist"));

"""
if text.count(marker) != 1:
    raise SystemExit("tests/io_workflow.rs: query test marker changed")
p.write_text(text.replace(marker, addition, 1))

offenders = []
for path in Path(".").rglob("*.rs"):
    if "engine::query" in path.read_text():
        offenders.append(str(path))
if offenders:
    raise SystemExit(f"remaining engine::query references: {offenders}")
