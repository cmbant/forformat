# Traceability: Python behaviour to Rust tests

One row per test in the frozen reference suite
(`tools/reference/test_standardize_fortran.py`, 86 tests in 6 classes).  Gate B
of the port plan needs every row to carry a terminal status.

Statuses: `todo`, `ported`, `covered by broader test`, `intentionally changed`
(with a rationale and a fixture), `not applicable`.

Categories: lexical, case, scope/project-case, OpenMP, CPP/macro, comment,
continuation, wrapping, blank-line/layout, CLI/file-I/O, semantic-compile.

Golden cases belong in `tests/manifests/python_formatter.manifest`, using the
existing manifest format and its `source_test` / `oracle` / `category` /
`support` / `normalization` / `mode` metadata.  Do not invent a second
mechanism.

Regenerate the row skeleton with `python3 tools/gen_traceability.py`; the last
three columns are hand-maintained and preserved.

| Python test | Category | Rust destination | Named Rust test | Status |
|---|---|---|---|---|
| `CommandLineTests.test_invalid_flag_combinations_use_argparse_errors` | CLI/file-I/O | — | — | todo |
| `CommandLineTests.test_uppercase_single_l_option` | CLI/file-I/O | — | — | todo |
| `CommandLineTests.test_isolated_option` | CLI/file-I/O | — | — | todo |
| `CommandLineTests.test_explicit_path_does_not_require_git_checkout` | CLI/file-I/O | — | — | todo |
| `CommandLineTests.test_isolated_path_does_not_scan_repository_sources` | CLI/file-I/O | — | — | todo |
| `FormattingTests.test_preserves_spacing_in_named_common_blocks` | blank-line/layout | — | — | todo |
| `FormattingTests.test_removes_only_redundant_nested_parentheses` | blank-line/layout | — | — | todo |
| `FormattingTests.test_normalizes_dimension_and_write_output_spacing` | blank-line/layout | — | — | todo |
| `FormattingTests.test_preserves_nested_parentheses_in_arguments_and_associations` | blank-line/layout | — | — | todo |
| `DeclarationCaseTests.test_declaration_array_constructor_is_one_entity` | scope/project-case | — | — | todo |
| `DeclarationCaseTests.test_extracts_and_matches_declarations` | scope/project-case | `src/transform/passes/case_pass.rs` | `declared_occurrences_use_their_name_spaces_and_are_idempotent` | ported |
| `DeclarationCaseTests.test_duplicate_resolution` | scope/project-case | `src/transform/passes/case_pass.rs` | `ambiguous_local_and_project_cases_are_silent` | ported |
| `DeclarationCaseTests.test_named_end_cases_follow_start_cases` | scope/project-case | `src/transform/passes/case_pass.rs` | `declared_occurrences_use_their_name_spaces_and_are_idempotent` | ported |
| `DeclarationCaseTests.test_compact_named_ends_follow_start_cases` | scope/project-case | — | — | todo |
| `DeclarationCaseTests.test_compact_procedure_end_closes_local_case_scope` | scope/project-case | — | — | todo |
| `DeclarationCaseTests.test_nested_procedure_uses_innermost_local_case` | scope/project-case | — | — | todo |
| `DeclarationCaseTests.test_bare_end_closes_local_case_scope` | scope/project-case | — | — | todo |
| `DeclarationCaseTests.test_module_procedure_is_not_a_module_or_module_variable_scope` | scope/project-case | — | — | todo |
| `DeclarationCaseTests.test_interface_dummies_are_not_module_variables` | scope/project-case | — | — | todo |
| `DeclarationCaseTests.test_lexical_join_refreshes_procedure_line_ranges` | scope/project-case | — | — | todo |
| `DeclarationCaseTests.test_local_variables_override_and_leave_global_pool` | scope/project-case | `src/transform/passes/case_pass.rs` | `declared_occurrences_use_their_name_spaces_and_are_idempotent` | covered by broader test |
| `DeclarationCaseTests.test_local_case_does_not_apply_to_derived_type_components` | scope/project-case | `src/transform/passes/case_pass.rs` | `declared_occurrences_use_their_name_spaces_and_are_idempotent` | covered by broader test |
| `DeclarationCaseTests.test_module_variables_are_case_matched_without_leaking_local_shadowing` | scope/project-case | `src/transform/passes/case_pass.rs` | `declared_occurrences_use_their_name_spaces_and_are_idempotent` | covered by broader test |
| `DeclarationCaseTests.test_select_type_aliases_are_local` | scope/project-case | — | — | todo |
| `ContinuationTests.test_file_reads_and_writes_preserve_crlf` | continuation | — | — | todo |
| `ContinuationTests.test_preserves_continued_cpp_directives` | continuation | — | — | todo |
| `ContinuationTests.test_joins_continuations_inside_lexical_tokens` | continuation | — | — | todo |
| `ContinuationTests.test_continuation_whitespace_separates_tokens` | continuation | — | — | todo |
| `ContinuationTests.test_preserves_lexical_continuation_markers_around_inline_comment` | continuation | — | — | todo |
| `ContinuationTests.test_leaves_continued_character_literals_unchanged` | continuation | — | — | todo |
| `ContinuationTests.test_splits_top_level_semicolon_statements` | continuation | — | — | todo |
| `SpacingTests.test_intrinsic_names_do_not_override_local_variables` | lexical | — | — | todo |
| `SpacingTests.test_global_symbols_do_not_override_intrinsics_or_real_exponents` | lexical | — | — | todo |
| `SpacingTests.test_module_declared_names_keep_their_case_against_specifiers_and_intrinsics` | lexical | — | — | todo |
| `SpacingTests.test_declared_names_do_not_leak_across_modules_in_the_same_file` | lexical | — | — | todo |
| `SpacingTests.test_declared_names_do_not_leak_from_a_type_component` | lexical | — | — | todo |
| `SpacingTests.test_component_cases_apply_inside_modules` | lexical | — | — | todo |
| `SpacingTests.test_lowercases_standard_statement_specifiers` | lexical | — | — | todo |
| `SpacingTests.test_preserves_concatenation_spacing_on_a_continuation_line` | lexical | — | — | todo |
| `SpacingTests.test_parenthesized_statements_lowercase_unless_locally_shadowed` | lexical | — | — | todo |
| `SpacingTests.test_type_bound_procedures_only_supply_component_case` | lexical | — | — | todo |
| `SpacingTests.test_declaration_entities_are_not_replaced_by_global_symbol_case` | lexical | — | — | todo |
| `SpacingTests.test_normalizes_control_keywords_and_bracket_spacing` | lexical | — | — | todo |
| `SpacingTests.test_removes_empty_subroutine_arguments_and_spaces_select_type` | lexical | — | — | todo |
| `SpacingTests.test_limits_blank_lines_inside_module_interfaces` | lexical | — | — | todo |
| `SpacingTests.test_keeps_exactly_one_blank_line_around_contains` | lexical | — | — | todo |
| `SpacingTests.test_keeps_blank_line_after_contains_following_select_type` | lexical | — | — | todo |
| `SpacingTests.test_resolves_chained_component_cases` | lexical | — | — | todo |
| `SpacingTests.test_resolves_component_case_after_a_local_object_chain` | lexical | — | — | todo |
| `SpacingTests.test_normalizes_trailing_whitespace_and_file_endings` | lexical | — | — | todo |
| `SpacingTests.test_does_not_change_spacing_inside_literals_or_comments` | lexical | — | — | todo |
| `SpacingTests.test_only_formats_comments_that_start_with_assignment` | lexical | — | — | todo |
| `SpacingTests.test_compound_keywords_are_only_expanded_at_statement_start` | lexical | — | — | todo |
| `SpacingTests.test_normalizes_go_to_to_goto` | lexical | — | — | todo |
| `SpacingTests.test_normalizes_post_f2008_language_keywords` | lexical | — | — | todo |
| `SpacingTests.test_normalizes_not_operator_and_comment_spacing` | lexical | — | — | todo |
| `SpacingTests.test_lowercases_logical_operators` | lexical | — | — | todo |
| `SpacingTests.test_preserves_case_sensitive_preprocessor_macros` | lexical | — | — | todo |
| `SpacingTests.test_optionally_uppercases_single_l_and_modernizes_array_constructors` | lexical | — | — | todo |
| `SpacingTests.test_removes_terminal_function_and_subroutine_returns` | lexical | — | — | todo |
| `SpacingTests.test_spaces_bare_program_unit_ends_like_named_ends` | lexical | — | — | todo |
| `SpacingTests.test_preserves_compiler_directive_sentinels_without_comment_spacing` | lexical | — | — | todo |
| `SpacingTests.test_wraps_openmp_directives` | lexical | — | — | todo |
| `SpacingTests.test_preserves_openmp_breaks_and_canonicalizes_continuation_sentinels` | lexical | — | — | todo |
| `SpacingTests.test_limits_blank_lines_and_preserves_declaration_alignment` | lexical | — | — | todo |
| `SpacingTests.test_normalizes_old_style_declaration_spacing_and_optional_order` | lexical | — | — | todo |
| `SpacingTests.test_reduces_declaration_alignment_to_its_minimum` | lexical | — | — | todo |
| `SpacingTests.test_alignment_compresses_through_comment_lines` | lexical | — | — | todo |
| `SpacingTests.test_alignment_keeps_a_compressible_subblock_before_unaligned_declarations` | lexical | — | — | todo |
| `SpacingTests.test_declaration_alignment_never_adds_padding` | lexical | — | — | todo |
| `RegressionFixTests.test_leading_continuation_markers_align_text_at_marker` | lexical | — | — | todo |
| `RegressionFixTests.test_procedure_modifiers_preserve_local_scope_case_handling` | lexical | — | — | todo |
| `RegressionFixTests.test_format_slash_edit_descriptors_are_not_array_constructors` | lexical | — | — | todo |
| `RegressionFixTests.test_cpp_continuation_body_is_never_treated_as_fortran` | lexical | — | — | todo |
| `RegressionFixTests.test_comment_between_continued_lines_stays_in_one_statement` | lexical | — | — | todo |
| `RegressionFixTests.test_external_macro_argument_sets_exact_case` | lexical | — | — | todo |
| `RegressionFixTests.test_source_defined_macros_do_not_force_unmatched_case` | lexical | — | — | todo |
| `RegressionFixTests.test_scope_ranges_refresh_after_terminal_return_removal` | lexical | — | — | todo |
| `RegressionFixTests.test_named_end_reduces_program_unit_depth` | lexical | — | — | todo |
| `RegressionFixTests.test_local_type_components_after_module_contains_do_not_leak` | lexical | — | — | todo |
| `RegressionFixTests.test_mapping_defaults_are_real_mappings` | lexical | — | — | todo |
| `RegressionFixTests.test_atomic_write_preserves_symlink` | lexical | — | — | todo |
| `RegressionFixTests.test_external_diff_display_path_does_not_raise` | lexical | — | — | todo |
| `RegressionFixTests.test_invalid_extension_is_rejected_before_reading` | lexical | — | — | todo |
| `RegressionFixTests.test_standard_free_form_extensions_are_accepted` | lexical | — | — | todo |
| `RegressionFixTests.test_wrapped_statement_keeps_not_against_its_bracket` | lexical | — | — | todo |
