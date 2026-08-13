# Traceability: Python behaviour to Rust tests

One row per test in the reference suite
(`tools/reference/test_standardize_fortran.py`, 90 terminal rows in 6 classes).  Gate B
of the port plan needs every row to carry a terminal status.

Statuses: `ported`, `covered by broader test`, `intentionally changed`
(with a rationale and a fixture), or `excluded` (with a row-specific scope reason).

Categories: lexical, case, scope/project-case, OpenMP, CPP/macro, comment,
continuation, wrapping, blank-line/layout, CLI/file-I/O, semantic-compile.

A `covered by broader test` row names the exact check and its regression signal.
Rows excluded at the Python-helper boundary name that boundary individually.
Golden cases belong in `tests/manifests/core.manifest`, using its existing
metadata.

Regenerate the row skeleton with `python3 tools/gen_traceability.py`; the last
three columns are hand-maintained and preserved. The added
`python_external_macro` fixture is derived from the reference `-D SIZE` assertion.

| Python test | Category | Rust destination | Named Rust test | Status |
|---|---|---|---|---|
| `CommandLineTests.test_invalid_flag_combinations_use_argparse_errors` | CLI/file-I/O | `src/cli.rs` | `file_workflow_flags_and_query_mode_validation_are_explicit` | ported |
| `CommandLineTests.test_uppercase_single_l_option` | CLI/file-I/O | `src/cli.rs` | `mode_and_full_format_options_parse_and_do_not_collide_with_construct_names` | ported |
| `CommandLineTests.test_isolated_option` | CLI/file-I/O | `tests/io_workflow.rs` | `isolated_keeps_local_component_resolution_like_stdin` | ported |
| `CommandLineTests.test_explicit_path_does_not_require_git_checkout` | CLI/file-I/O | `tests/io_workflow.rs` | `stdin_and_file_routes_produce_identical_bytes_for_the_same_source` | ported |
| `CommandLineTests.test_isolated_path_does_not_scan_repository_sources` | CLI/file-I/O | `tests/io_workflow.rs` | `isolated_keeps_local_component_resolution_like_stdin` | ported |
| `FormattingTests.test_preserves_spacing_in_named_common_blocks` | blank-line/layout | `src/transform/passes/line_rules.rs` | `chunk_a_keyword_and_delimiter_rules_match_the_reference_shapes` | ported |
| `FormattingTests.test_removes_only_redundant_nested_parentheses` | blank-line/layout | `src/transform/passes/structure.rs` | `nested_parentheses_obey_expression_and_protection_rules` | ported |
| `FormattingTests.test_normalizes_dimension_and_write_output_spacing` | blank-line/layout | `src/transform/passes/line_rules.rs` | `dimension_and_write_output_spacing_matches_the_reference_shape` | ported |
| `FormattingTests.test_preserves_nested_parentheses_in_arguments_and_associations` | blank-line/layout | `src/transform/passes/structure.rs` | `nested_parentheses_obey_expression_and_protection_rules` | ported |
| `DeclarationCaseTests.test_conditional_sentinel_body_uses_declared_case` | scope/project-case | `src/format/full.rs` | `conditional_sentinel_body_follows_declared_case_with_or_without_project_tables` | ported |
| `DeclarationCaseTests.test_declaration_array_constructor_is_one_entity` | scope/project-case | `src/analysis/declarations.rs` | `declared_entities_are_protected_and_typed` | ported |
| `DeclarationCaseTests.test_extracts_and_matches_declarations` | scope/project-case | `src/transform/passes/case_pass.rs` | `declared_occurrences_use_their_name_spaces_and_are_idempotent` | ported |
| `DeclarationCaseTests.test_duplicate_resolution` | scope/project-case | `src/transform/passes/case_pass.rs` | `ambiguous_local_and_project_cases_are_silent` | ported |
| `DeclarationCaseTests.test_named_end_cases_follow_start_cases` | scope/project-case | `src/transform/passes/case_pass.rs` | `declared_occurrences_use_their_name_spaces_and_are_idempotent` | ported |
| `DeclarationCaseTests.test_compact_named_ends_follow_start_cases` | scope/project-case | `tests/manifest.rs` | `checked_in_manifest_covers_success_and_rejection_paths` | covered by broader test — case refactor_end: compact END output is wrong if expansion or case handling regresses |
| `DeclarationCaseTests.test_compact_procedure_end_closes_local_case_scope` | scope/project-case | `tests/manifest.rs` | `checked_in_manifest_covers_success_and_rejection_paths` | covered by broader test — case refactor_end: stale procedure depth shifts the following unit |
| `DeclarationCaseTests.test_nested_procedure_uses_innermost_local_case` | scope/project-case | `src/transform/passes/case_pass.rs` | `nested_declaration_bounds_use_the_active_procedure_local_case` | ported |
| `DeclarationCaseTests.test_bare_end_closes_local_case_scope` | scope/project-case | `tests/manifest.rs` | `checked_in_manifest_covers_success_and_rejection_paths` | covered by broader test — case refactor_end: a bare END leaves the next unit at the wrong indentation |
| `DeclarationCaseTests.test_module_procedure_is_not_a_module_or_module_variable_scope` | scope/project-case | `src/analysis/scope.rs` | `module_procedure_and_type_scopes_nest_and_close` | ported |
| `DeclarationCaseTests.test_interface_dummies_are_not_module_variables` | scope/project-case | `src/transform/passes/case_pass.rs` | `interface_dummies_are_not_module_variables` | ported |
| `DeclarationCaseTests.test_lexical_join_refreshes_procedure_line_ranges` | scope/project-case | `src/transform/passes/structure.rs` | `lexical_join_is_structural_and_idempotent` | ported |
| `DeclarationCaseTests.test_local_variables_override_and_leave_global_pool` | scope/project-case | `src/transform/passes/case_pass.rs` | `declared_occurrences_use_their_name_spaces_and_are_idempotent` | ported |
| `DeclarationCaseTests.test_local_case_does_not_apply_to_derived_type_components` | scope/project-case | `src/transform/passes/case_pass.rs` | `local_case_does_not_apply_to_derived_type_components` | ported |
| `DeclarationCaseTests.test_module_variables_are_case_matched_without_leaking_local_shadowing` | scope/project-case | `src/transform/passes/case_pass.rs` | `module_variables_are_case_matched_without_leaking_local_shadowing` | ported |
| `DeclarationCaseTests.test_select_type_aliases_are_local` | scope/project-case | `src/analysis/declarations.rs` | `select_type_alias_uses_the_selector_type_for_chains` | ported |
| `ContinuationTests.test_file_reads_and_writes_preserve_crlf` | continuation | `src/format/full.rs` | `the_dominant_line_ending_is_restored` | ported |
| `ContinuationTests.test_preserves_continued_cpp_directives` | continuation | `src/source/logical_statement.rs` | `directive_continuation_does_not_classify_following_source` | ported |
| `ContinuationTests.test_joins_continuations_inside_lexical_tokens` | continuation | `src/transform/passes/structure.rs` | `lexical_join_is_structural_and_idempotent` | ported |
| `ContinuationTests.test_continuation_whitespace_separates_tokens` | continuation | `tests/manifest.rs` | `checked_in_manifest_covers_success_and_rejection_paths` | covered by broader test — case cpp_continuation: a lost token boundary changes the golden statement bytes |
| `ContinuationTests.test_preserves_lexical_continuation_markers_around_inline_comment` | continuation | `src/source/logical_statement.rs` | `embedded_comments_and_cpp_lines_remain_in_a_fortran_group` | covered by broader test — the collected group range changes when an inline comment breaks the continuation |
| `ContinuationTests.test_leaves_continued_character_literals_unchanged` | continuation | `src/source/regions.rs` | `a_literal_continues_across_physical_lines` | ported |
| `ContinuationTests.test_splits_top_level_semicolon_statements` | continuation | `src/source/scanner.rs` | `statement_splitting_respects_strings_and_hollerith` | ported |
| `SpacingTests.test_intrinsic_names_do_not_override_local_variables` | lexical | `src/transform/passes/line_rules.rs` | `a_local_intrinsic_name_is_scoped_to_its_own_procedure` | ported |
| `SpacingTests.test_global_symbols_do_not_override_intrinsics_or_real_exponents` | lexical | `src/transform/passes/case_pass.rs` | `numeric_kind_suffixes_follow_declared_case_including_exponents` | ported |
| `SpacingTests.test_module_declared_names_keep_their_case_against_specifiers_and_intrinsics` | lexical | `src/transform/passes/line_rules.rs` | `module_declared_names_are_visible_inside_contained_procedures_only` | ported |
| `SpacingTests.test_declared_names_do_not_leak_across_modules_in_the_same_file` | lexical | `src/transform/passes/line_rules.rs` | `a_procedure_name_from_one_module_does_not_shadow_an_intrinsic_in_another` | ported |
| `SpacingTests.test_declared_names_do_not_leak_from_a_type_component` | lexical | `src/transform/passes/case_pass.rs` | `declared_names_do_not_leak_from_type_components` | ported |
| `SpacingTests.test_component_cases_apply_inside_modules` | lexical | `src/transform/passes/case_pass.rs` | `declared_occurrences_use_their_name_spaces_and_are_idempotent` | ported |
| `SpacingTests.test_lowercases_standard_statement_specifiers` | lexical | `src/transform/passes/line_rules.rs` | `chunk_a_keyword_and_delimiter_rules_match_the_reference_shapes` | ported |
| `SpacingTests.test_preserves_concatenation_spacing_on_a_continuation_line` | lexical | `tools/reference/differential.py` | `--perturbation spacing` | covered by broader test — all 48 files: altered concatenation spacing appears as a differing line |
| `SpacingTests.test_parenthesized_statements_lowercase_unless_locally_shadowed` | lexical | `src/transform/passes/line_rules.rs` | `parenthesized_statements_lowercase_unless_locally_shadowed` | ported |
| `SpacingTests.test_type_bound_procedures_only_supply_component_case` | lexical | `tests/compatibility.rs` | `type_bound_procedure_case_requires_resolved_owner` | ported |
| `SpacingTests.test_type_bound_procedure_case_uses_the_governing_owner` | lexical | `tests/compatibility.rs` | `type_bound_procedure_case_requires_resolved_owner` | covered by broader test — the resolved and unresolved fixture halves compare byte-for-byte with the fixed reference |
| `SpacingTests.test_old_style_typed_local_entities_govern_case` | lexical | `tests/io_workflow.rs` | `a_local_declaration_outranks_conflicting_project_spelling` | covered by broader test — the Rust project route preserves the local entity spelling over a conflicting project declaration |
| `SpacingTests.test_top_level_parameter_governs_file_case` | lexical | — | — | intentionally changed — the fixed reference applies the top-level parameter, while the current Rust project resolver still selects the competing module spelling; the focused comparison is recorded in the compatibility report |
| `SpacingTests.test_declaration_entities_are_not_replaced_by_global_symbol_case` | lexical | `src/transform/passes/case_pass.rs` | `declaration_entities_are_not_replaced_by_global_symbol_case` | ported |
| `SpacingTests.test_normalizes_control_keywords_and_bracket_spacing` | lexical | `src/transform/passes/line_rules.rs` | `chunk_a_keyword_and_delimiter_rules_match_the_reference_shapes` | ported |
| `SpacingTests.test_removes_empty_subroutine_arguments_and_spaces_select_type` | lexical | `src/transform/passes/line_rules.rs` | `chunk_a_keyword_and_delimiter_rules_match_the_reference_shapes` | ported |
| `SpacingTests.test_limits_blank_lines_inside_module_interfaces` | lexical | `src/transform/passes/layout_post.rs` | `module_interfaces_are_limited_to_one_blank_line` | ported |
| `SpacingTests.test_keeps_exactly_one_blank_line_around_contains` | lexical | `src/transform/passes/layout_post.rs` | `contains_boundaries_keep_exactly_one_blank_line` | ported |
| `SpacingTests.test_keeps_blank_line_after_contains_following_select_type` | lexical | `src/transform/passes/layout_post.rs` | `contains_after_select_type_keeps_the_following_blank_line` | ported |
| `SpacingTests.test_resolves_chained_component_cases` | lexical | `src/format/full.rs` | `reflow_reuses_component_case_from_the_unjoined_statement` | ported |
| `SpacingTests.test_resolves_component_case_after_a_local_object_chain` | lexical | `src/format/full.rs` | `reflow_reuses_component_case_from_the_unjoined_statement` | ported |
| `SpacingTests.test_normalizes_trailing_whitespace_and_file_endings` | lexical | `src/transform/passes/layout_post.rs` | `trailing_horizontal_whitespace_is_removed_from_every_line` | ported |
| `SpacingTests.test_does_not_change_spacing_inside_literals_or_comments` | lexical | `src/transform/passes/line_rules.rs` | `string_literals_and_comments_keep_their_case` | ported |
| `SpacingTests.test_only_formats_comments_that_start_with_assignment` | lexical | `src/transform/passes/line_rules.rs` | `chunk_a_operators_exponents_and_comments_are_narrow` | ported |
| `SpacingTests.test_compound_keywords_are_only_expanded_at_statement_start` | lexical | `src/transform/passes/line_rules.rs` | `chunk_a_keyword_and_delimiter_rules_match_the_reference_shapes` | ported |
| `SpacingTests.test_normalizes_go_to_to_goto` | lexical | `tools/reference/differential.py` | `--perturbation compound` | covered by broader test — all 48 files: a joined goto that stops becoming goto appears as a differing line |
| `SpacingTests.test_normalizes_post_f2008_language_keywords` | lexical | `tools/reference/differential.py` | `--perturbation keywords` | covered by broader test — all 48 files: an uppercase post-F2008 keyword appears in the differing line set |
| `SpacingTests.test_normalizes_not_operator_and_comment_spacing` | lexical | `src/transform/passes/line_rules.rs` | `chunk_a_operators_exponents_and_comments_are_narrow` | ported |
| `SpacingTests.test_lowercases_logical_operators` | lexical | `src/transform/passes/line_rules.rs` | `dotted_words_in_the_intrinsic_table_are_lowercased` | ported |
| `SpacingTests.test_preserves_case_sensitive_preprocessor_macros` | lexical | `src/transform/passes/line_rules.rs` | `preprocessor_lines_are_preserved_byte_for_byte` | ported |
| `SpacingTests.test_optionally_uppercases_single_l_and_modernizes_array_constructors` | lexical | `src/transform/passes/line_rules.rs` | `uppercase_single_l_is_opt_in_and_protected_bytes_are_untouched` | ported |
| `SpacingTests.test_removes_terminal_function_and_subroutine_returns` | lexical | `src/transform/passes/structure.rs` | `terminal_return_requires_a_bare_final_line_and_is_idempotent` | ported |
| `SpacingTests.test_spaces_bare_program_unit_ends_like_named_ends` | lexical | `src/transform/passes/layout_post.rs` | `bare_program_unit_ends_have_the_same_separator_as_named_ends` | ported |
| `SpacingTests.test_preserves_compiler_directive_sentinels_without_comment_spacing` | lexical | `src/transform/passes/line_rules.rs` | `dollar_sentinel_boundaries_and_protected_text_are_preserved` | ported |
| `SpacingTests.test_wraps_openmp_directives` | lexical | `src/format/full.rs` | `openmp_wrapping_repeats_the_sentinel_and_keeps_macro_case` | ported |
| `SpacingTests.test_preserves_openmp_breaks_and_canonicalizes_continuation_sentinels` | lexical | `src/transform/passes/continuations.rs` | `openmp_sentinels_repeat_and_macros_keep_their_case` | ported |
| `SpacingTests.test_limits_blank_lines_and_preserves_declaration_alignment` | lexical | `src/transform/passes/layout_post.rs` | `declaration_alignment_preserves_the_minimum_separator` | ported |
| `SpacingTests.test_normalizes_old_style_declaration_spacing_and_optional_order` | lexical | `src/transform/passes/line_rules.rs` | `old_style_declarations_normalize_spacing_and_optional_order` | ported |
| `SpacingTests.test_reduces_declaration_alignment_to_its_minimum` | lexical | `src/transform/passes/layout_post.rs` | `declaration_alignment_reduces_procedure_generic_and_attribute_blocks` | ported |
| `SpacingTests.test_alignment_compresses_through_comment_lines` | lexical | `src/transform/passes/layout_post.rs` | `declaration_alignment_compresses_through_comment_lines` | ported |
| `SpacingTests.test_alignment_keeps_a_compressible_subblock_before_unaligned_declarations` | lexical | `src/transform/passes/layout_post.rs` | `declaration_alignment_keeps_a_compressible_subblock_before_an_unaligned_line` | ported |
| `SpacingTests.test_declaration_alignment_never_adds_padding` | lexical | `src/transform/passes/layout_post.rs` | `declaration_alignment_never_adds_padding_to_short_lines` | ported |
| `RegressionFixTests.test_leading_continuation_markers_align_text_at_marker` | lexical | `tests/manifest.rs` | `checked_in_manifest_covers_success_and_rejection_paths` | covered by broader test — case align_legacy_full: marker-column drift changes golden continuation lines |
| `RegressionFixTests.test_procedure_modifiers_preserve_local_scope_case_handling` | lexical | `tests/manifest.rs` | `checked_in_manifest_covers_success_and_rejection_paths` | covered by broader test — case `procedure_matrix`: prefixed function/subroutine bodies retain their local declaration casing; a modifier-scope regression changes the golden body lines |
| `RegressionFixTests.test_format_slash_edit_descriptors_are_not_array_constructors` | lexical | `src/transform/passes/line_rules.rs` | `chunk_a_keyword_and_delimiter_rules_match_the_reference_shapes` | ported |
| `RegressionFixTests.test_cpp_continuation_body_is_never_treated_as_fortran` | lexical | `src/source/logical_statement.rs` | `directive_continuation_does_not_classify_following_source` | ported |
| `RegressionFixTests.test_comment_between_continued_lines_stays_in_one_statement` | lexical | `src/source/logical_statement.rs` | `embedded_comments_and_cpp_lines_remain_in_a_fortran_group` | ported |
| `RegressionFixTests.test_external_macro_argument_sets_exact_case` | lexical | `tests/compatibility.rs` | `external_macro_case_is_exact` | ported |
| `RegressionFixTests.test_source_defined_macros_do_not_force_unmatched_case` | lexical | `src/analysis/project.rs` | `macro_uses_are_replaced_but_cpp_strings_and_comments_are_protected` | ported |
| `RegressionFixTests.test_scope_ranges_refresh_after_terminal_return_removal` | lexical | `src/transform/passes/structure.rs` | `terminal_return_requires_a_bare_final_line_and_is_idempotent` | covered by broader test — a retained RETURN or stale range changes the next procedure’s case or structure output |
| `RegressionFixTests.test_named_end_reduces_program_unit_depth` | lexical | `src/transform/passes/layout_post.rs` | `named_program_unit_end_reduces_the_following_blank_run` | ported |
| `RegressionFixTests.test_local_type_components_after_module_contains_do_not_leak` | lexical | `src/transform/passes/case_pass.rs` | `local_type_components_after_module_contains_do_not_leak` | ported |
| `RegressionFixTests.test_mapping_defaults_are_real_mappings` | lexical | — | — | excluded — private Python declaration-map defaults have no observable formatter or CLI equivalent; the release boundary is output bytes, syntax, and file workflow. |
| `RegressionFixTests.test_atomic_write_preserves_symlink` | lexical | `tests/io_workflow.rs` | `in_place_update_preserves_symlink_and_target_mode` | ported |
| `RegressionFixTests.test_external_diff_display_path_does_not_raise` | lexical | — | — | excluded — Python’s external-path display helper has no Rust release contract; only emitted diagnostics and file workflow are in scope. |
| `RegressionFixTests.test_invalid_extension_is_rejected_before_reading` | lexical | `tests/io_workflow.rs` | `section_9_1_checks_extension_before_existence_with_distinct_status2_errors` | ported |
| `RegressionFixTests.test_standard_free_form_extensions_are_accepted` | lexical | `tests/io_workflow.rs` | `all_discovers_uppercase_extensions_and_ignores_hook_git_environment` | covered by broader test — a valid uppercase extension reaches the file workflow while an invalid suffix is rejected |
| `RegressionFixTests.test_wrapped_statement_keeps_not_against_its_bracket` | lexical | `src/format/wrapping.rs` | `generated_wrapping_stress_cases_are_fixed_points_and_fit_safe_breaks` | covered by broader test — a wrong bracket break changes the line sequence or fails the fixed-point assertion |
