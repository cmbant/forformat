use crate::{
    cli::{parse, Command},
    config::{FormatConfig, FormatMode, FortranStandard, KeywordCase},
};
use std::{fs, path::PathBuf};

fn config_from_text(name: &str, text: &str) -> FormatConfig {
    let path = std::env::temp_dir().join(format!(
        "forformat-config-{name}-{}.toml",
        std::process::id()
    ));
    fs::write(&path, text).unwrap();
    let result = parse([
        "forformat".to_string(),
        format!("--config={}", path.display()),
    ]);
    let _ = fs::remove_file(&path);
    match result.unwrap() {
        Command::Run(invocation) => invocation.config,
        _ => panic!("expected a formatting command"),
    }
}

#[test]
fn specific_indent_config_overrides_the_global_reset() {
    for text in [
        "indent = 4\nindent-select = 2\n",
        "indent-select = 2\nindent = 4\n",
    ] {
        let config = config_from_text("specific-indent", text);
        assert_eq!(config.indent, 4);
        assert_eq!(config.construct_indents.select, 2);
    }
}

#[test]
fn global_indent_config_resets_every_per_construct_indent() {
    let config = config_from_text("global-indent", "indent = 4\n");

    assert_eq!(config.construct_indents.associate, 4);
    assert_eq!(config.construct_indents.block, 4);
    assert_eq!(config.construct_indents.changeteam, 4);
    assert_eq!(config.construct_indents.critical, 4);
    assert_eq!(config.construct_indents.do_, 4);
    assert_eq!(config.construct_indents.r#enum, 4);
    assert_eq!(config.construct_indents.forall, 4);
    assert_eq!(config.construct_indents.if_, 4);
    assert_eq!(config.construct_indents.interface, 4);
    assert_eq!(config.construct_indents.module, 4);
    assert_eq!(config.construct_indents.procedure, 4);
    assert_eq!(config.construct_indents.select, 4);
    assert_eq!(config.construct_indents.r#type, 4);
    assert_eq!(config.construct_indents.where_, 4);
    assert_eq!(config.contains_indent, 4);
    assert_eq!(config.continuation_indent, 4);
    assert_eq!(config.case_indent, 2);
    assert_eq!(config.entry_indent, 2);
}

#[test]
fn project_context_is_not_a_configuration_key() {
    let path = std::env::temp_dir().join(format!(
        "forformat-project-context-config-{}.toml",
        std::process::id()
    ));
    fs::write(&path, "project-context = '.'\n").unwrap();
    let error = match parse([
        "forformat".to_string(),
        format!("--config={}", path.display()),
    ]) {
        Err(error) => error,
        Ok(_) => panic!("workflow-only config key was accepted"),
    };
    let _ = fs::remove_file(&path);
    assert!(error
        .to_string()
        .contains("configuration key `project-context` is a command-line workflow option"));
}

#[test]
fn singular_context_path_is_not_a_configuration_key() {
    let path = std::env::temp_dir().join(format!(
        "forformat-context-path-config-{}.toml",
        std::process::id()
    ));
    fs::write(&path, "context_path = 'configured'\n").unwrap();
    let error = match parse([
        "forformat".to_string(),
        format!("--config={}", path.display()),
    ]) {
        Err(error) => error,
        Ok(_) => panic!("singular context_path config key was accepted"),
    };
    let _ = fs::remove_file(&path);
    assert!(
        error
            .to_string()
            .contains("use `context-paths = [\"...\"]`"),
        "{error}"
    );
}

#[test]
fn query_format_is_not_a_configuration_key() {
    let path = std::env::temp_dir().join(format!(
        "forformat-query-format-config-{}.toml",
        std::process::id()
    ));
    fs::write(&path, "query_format = true\n").unwrap();
    let error = match parse([
        "forformat".to_string(),
        format!("--config={}", path.display()),
    ]) {
        Err(error) => error,
        Ok(_) => panic!("workflow-only config key was accepted"),
    };
    let _ = fs::remove_file(&path);
    assert!(error
        .to_string()
        .contains("configuration key `query-format` is a command-line workflow option"));
}

#[test]
fn canonicalize_mode_loads_from_toml() {
    let config = config_from_text("canonicalize", "mode = 'canonicalize-only'\n");
    assert_eq!(config.mode, FormatMode::CanonicalizeOnly);
    assert!(!config.mode.normalizes_whitespace());
    assert!(config.mode.normalizes());
    assert!(!config.mode.lays_out());
}

#[test]
fn target_standard_loads_from_toml() {
    let config = config_from_text("target-standard", "target_standard = 'f95'\n");
    assert_eq!(config.target_standard, FortranStandard::F95);
}

#[test]
fn rewrap_loads_from_toml_alongside_full_mode() {
    let config = config_from_text("full-rewrap", "mode = 'full'\nrewrap = true\n");
    assert_eq!(config.mode, FormatMode::Full);
    assert!(config.rewrap);
    assert!(config.wrap.enabled);
}

#[test]
fn rewrap_is_rejected_by_a_mode_that_cannot_wrap() {
    let path = std::env::temp_dir().join(format!(
        "forformat-canonicalize-rewrap-{}.toml",
        std::process::id()
    ));
    fs::write(&path, "mode = 'canonicalize-only'\nrewrap = true\n").unwrap();
    let result = parse([
        "forformat".to_string(),
        format!("--config={}", path.display()),
    ]);
    let _ = fs::remove_file(&path);
    let Err(error) = result else {
        panic!("rewrap in a no-layout mode should be rejected");
    };
    assert!(
        error.to_string().contains("--rewrap requires full mode"),
        "{error}"
    );
}

#[test]
fn style_keys_load_from_the_standalone_toml_shape() {
    let config = config_from_text(
        "style-options",
        "keyword_case = 'upper'\nopenmp-case = false\nrelational_symbols = false\ncompact_multiplicative = false\narray-brackets = false\njoin-goto = false\nsplit-compound-keywords = false\nstrip_empty_args = false\nremove-redundant-parens = false\nnormalize-semicolons = false\nremove_terminal_return = false\nprogram_unit_spacing = false\nmax_blank_lines = 'preserve'\ndelimiter-spacing = false\ncomment_spacing = false\ncontinuation-markers = false\n",
    );
    assert_eq!(config.style.keyword_case, KeywordCase::Upper);
    assert!(!config.style.openmp_case);
    assert!(!config.style.relational_symbols);
    assert!(!config.style.compact_multiplicative);
    assert!(!config.style.array_brackets);
    assert!(!config.style.join_goto);
    assert!(!config.style.split_compound_keywords);
    assert!(!config.style.strip_empty_args);
    assert!(!config.style.remove_redundant_parens);
    assert!(!config.style.normalize_semicolons);
    assert!(!config.style.remove_terminal_return);
    assert!(!config.style.program_unit_spacing);
    assert_eq!(config.style.max_blank_lines, None);
    assert!(!config.style.delimiter_spacing);
    assert!(!config.style.comment_spacing);
    assert!(!config.style.continuation_markers);
}

#[test]
fn style_keys_load_from_pyproject_and_cli_scalars_override_them() {
    let directory =
        std::env::temp_dir().join(format!("forformat-pyproject-dir-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let pyproject = directory.join("pyproject.toml");
    fs::write(
        &pyproject,
        "[tool.forformat]\nkeyword-case = 'upper'\ncompact_multiplicative = false\nstrip_empty_args = false\nmax-blank-lines = 1\n",
    )
    .unwrap();
    let result = parse([
        "forformat".to_string(),
        format!("--config={}", pyproject.display()),
        "--keyword-case=preserve".to_string(),
        "--strip-empty-args=1".to_string(),
        "--max-blank-lines=0".to_string(),
    ]);
    let _ = fs::remove_file(&pyproject);
    let _ = fs::remove_dir(&directory);
    let Command::Run(invocation) = result.unwrap() else {
        panic!("expected run")
    };
    assert_eq!(invocation.config.style.keyword_case, KeywordCase::Preserve);
    assert!(!invocation.config.style.compact_multiplicative);
    assert!(invocation.config.style.strip_empty_args);
    assert_eq!(invocation.config.style.max_blank_lines, Some(0));
}

#[test]
fn boolean_switches_keep_false_values_when_loaded_from_toml() {
    let config = config_from_text(
        "boolean-switches",
        "uppercase_single_l = false\nrefactor_end = false\nno_wrap = false\nrewrap = false\n",
    );

    assert!(!config.uppercase_single_l);
    assert!(!config.refactor_end);
    assert!(!config.rewrap);
    assert!(config.wrap.enabled);
}

#[test]
fn cli_replacement_layers_override_configured_collections() {
    let path = std::env::temp_dir().join(format!("forformat-layering-{}.toml", std::process::id()));
    fs::write(
        &path,
        "no_submodules = true\ncontext_paths = ['configured']\nexclude = ['configured/**']\nextend_exclude = ['from-config/**']\n",
    )
    .unwrap();
    let result = parse([
        "forformat".to_string(),
        format!("--config={}", path.display()),
        "--no-submodules=false".to_string(),
        "--context-path=cli".to_string(),
        "--exclude=cli/**".to_string(),
        "--extend-exclude=from-cli/**".to_string(),
    ]);
    let _ = fs::remove_file(&path);
    let Command::Run(invocation) = result.unwrap() else {
        panic!("expected run")
    };
    assert!(!invocation.no_submodules);
    assert_eq!(invocation.context_paths.len(), 1);
    assert_eq!(invocation.context_paths[0].path, PathBuf::from("cli"));
    assert_eq!(
        invocation.exclude.as_deref(),
        Some(&["cli/**".to_string()][..])
    );
    assert_eq!(
        invocation.extend_exclude,
        ["from-config/**".to_string(), "from-cli/**".to_string()]
    );
}
