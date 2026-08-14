use forformat::{
    format_source, format_to, format_to_owned,
    source::{regions::regions, LogicalGroup, RegionKind, SourceBuffer},
    FormatConfig, FormatMode,
};

fn indent_only_config() -> FormatConfig {
    FormatConfig {
        mode: FormatMode::IndentOnly,
        ..FormatConfig::default()
    }
}

#[test]
fn default_formatting_is_idempotent_on_malformed_and_lexical_corpus() {
    let corpus: &[&[u8]] = &[
        b"",
        b"program p\nif (x) then\nx = 1\nend if\nend program\n",
        b"program p  \n! caf\xe9\nx = \"!;&\"; 4H;! comment\nend program",
        b"#if X\nif (x) then\n#else\nif (y) then\n#endif\nx=1\nend if\n",
        b"program p\nif (x) then\n",
        &[0, 1, 2, b'\n', 0xff, b'!', b'\n', b')', b'('],
    ];
    for source in corpus {
        let config = FormatConfig {
            mode: FormatMode::IndentOnly,
            ..FormatConfig::default()
        };
        let once = format_source(source, &config).unwrap().bytes;
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "source was not idempotent: {source:?}");
    }
}

#[test]
fn full_mode_chunk_a_preserves_protected_bytes_and_is_idempotent() {
    let source = b"PROGRAM P\nCALL F('IF  THEN  ', 4Hab  c) ! x=1+2\n#define IF_THING 1\nIF (X) THEN\nEND IF\nEND PROGRAM P\n";
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let once = format_source(source, &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(once, twice, "full mode is not a fixed point");

    let protected = |bytes: &[u8]| {
        let mut literals = Vec::new();
        let mut cpp = Vec::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            if line.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'#') {
                cpp.push(line.to_vec());
            }
            for region in regions(line) {
                if matches!(
                    region.kind,
                    RegionKind::StringLiteral | RegionKind::Hollerith
                ) {
                    literals.push(line[region.range].to_vec());
                }
            }
        }
        (literals, cpp)
    };
    assert_eq!(protected(source), protected(&once));
}

#[test]
fn full_mode_fixed_point_and_indent_only_fixed_point_hold_together() {
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    for source in [
        include_bytes!("fixtures/core.f90").as_slice(),
        include_bytes!("fixtures/cpp_continuation.f90").as_slice(),
        include_bytes!("fixtures/array_constructor_multiline.f90").as_slice(),
        b"\n!$ \n".as_slice(),
        b"program p\nif (x) then\ncall f(a, b, c, d, e, f, g, h)\nend if\nend program p\n"
            .as_slice(),
    ] {
        let once = format_source(source, &config).unwrap().bytes;
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "full mode I1 failed for {source:?}");

        let indent = FormatConfig {
            mode: FormatMode::IndentOnly,
            ..FormatConfig::default()
        };
        assert_eq!(format_source(&once, &indent).unwrap().bytes, once, "I2");
    }
}

#[test]
fn a_declared_name_keeps_its_spelling_on_a_continued_entity_list() {
    // Reduced from CP2K, which really does declare a component called `TYPE`.
    // The rule that protects a declared name looks for the `::` on its own
    // line; once the wrapper moved `TYPE` onto a continuation there was no
    // `::` to find, so it read as the keyword and was lowercased.
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: false,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    for (source, expected) in [
        (
            "module m\n   type t\n      integer :: ref_count = -1, &\n         TYPE = -1, other = -1\n   end type t\nend module m\n",
            "         TYPE = -1, other = -1",
        ),
        // An initializer is an expression, not an entity: names in it are
        // resolved as code.
        (
            "module m\n   integer :: a = 1, &\n      b = SIZE(x)\nend module m\n",
            "      b = size(x)",
        ),
        // So is anything inside a group the continuation is still nested in.
        (
            "module m\n   integer :: a = f( &\n      TYPE)\nend module m\n",
            "      type)",
        ),
    ] {
        let once = format_source(source.as_bytes(), &config).unwrap().bytes;
        let text = String::from_utf8(once.clone()).unwrap();
        assert!(
            text.lines().any(|line| line == expected),
            "expected {expected:?} in\n{text}"
        );
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "full mode I1 failed for {source:?}");
    }
}

#[test]
fn a_wrapped_declaration_does_not_repartition_its_alignment_block() {
    // Reduced from SPECFEM3D at `--line-length=80`. The wrapper measures the
    // laid-out width, which step 17 sets from the alignment block a line is in.
    // A continuation used to end that block, so wrapping the first declaration
    // moved every declaration below it into a different block, with a different
    // column and a different width — and the next run measured that width and
    // made a different wrapping decision. The loop only closes if the partition
    // does not depend on where the wrapper broke.
    let source = "\
program p
    integer,                                intent(in)     :: myrank
    ! local
    integer                                                :: ievent, ireceiver, nsta_slice, irec_local, NSTA, NEVENT, ier
    integer                                                :: ispec_selected, islice_selected, idim
    double precision                                       :: xi_receiver, eta_receiver, gamma_receiver
    double precision                                       :: x_found,  y_found,  z_found
    double precision                                       :: x_to_locate, y_to_locate, z_to_locate
    real(kind=CUSTOM_REAL)                                 :: distance_min_glob,distance_max_glob
    real(kind=CUSTOM_REAL)                                 :: elemsize_min_glob,elemsize_max_glob
    real(kind=CUSTOM_REAL)                                 :: x_min_glob,x_max_glob
    real(kind=CUSTOM_REAL)                                 :: y_min_glob,y_max_glob
    real(kind=CUSTOM_REAL)                                 :: z_min_glob,z_max_glob
    integer,                 dimension(NGNOD)              :: iaddx,iaddy,iaddz
    double precision,        dimension(NGLLX)              :: hxis,hpxis
end program p
";
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            line_length: 80,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    let once = format_source(source.as_bytes(), &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&twice),
        "wrapping one declaration changed the block the others are measured in"
    );
}

#[test]
fn a_dotted_operator_before_a_continuation_leaves_the_next_sign_unary() {
    // Reduced from CP2K at `--indent=8`, where the wrapper breaks after
    // `.or. &`. Deciding "the previous line ended on an operand" from its last
    // byte counted the closing `.` of `.or.`, so the leading `-` of the next
    // line was spaced as if it were binary — one run after the wrapper created
    // the break.
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: false,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    for (source, expected) in [
        (
            "program p\nif (a > c .or. &\n   -b > c) then\nx = 1\nend if\nend program p\n",
            "      -b > c) then",
        ),
        // The operand cases this guard has to leave alone: a name, and a
        // decimal point, both of which really do end on an operand.
        ("program p\nx = a &\n   - b\nend program p\n", "      - b"),
        ("program p\nx = 1. &\n   - b\nend program p\n", "      - b"),
    ] {
        let once = format_source(source.as_bytes(), &config).unwrap().bytes;
        let text = String::from_utf8(once.clone()).unwrap();
        assert!(
            text.lines().any(|line| line == expected),
            "expected {expected:?} in\n{text}"
        );
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "full mode I1 failed for {source:?}");
    }
}

#[test]
fn step_17_relayout_keeps_paren_alignment_on_the_width_it_emits() {
    // Reduced from CP2K at `--align-paren`. The engine aligns the continuation
    // under the `[` of the head line; step 17 then compresses that head's `::`
    // and moves the `[` twenty columns left, stranding the continuation where
    // the `[` used to be. The next run reads the compressed head and aligns
    // correctly, so the two runs disagree.
    let source = "\
module m
   character(len=3), DIMENSION(7), &
      PARAMETER, PUBLIC                     :: periodicity_string = [\"  X\", \"  Y\", \"  Z\", &
                                                                     \" XY\", \" XZ\", \" YZ\", &
                                                                     \"XYZ\"]
end module m
";
    let config = FormatConfig {
        mode: FormatMode::Full,
        align_paren: true,
        ..FormatConfig::default()
    };
    let once = format_source(source.as_bytes(), &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&twice),
        "step 17 moved a head line the engine had already aligned against"
    );
    // Paren alignment points at something on the line above, so a continuation
    // can never start past the end of the head it is aligned to. Before the
    // re-layout the head was compressed to 71 columns and the continuation
    // stayed at 72 — aligned to a `[` that had moved.
    let text = String::from_utf8_lossy(&once);
    let head = text
        .lines()
        .find(|line| line.contains("periodicity_string"))
        .expect("head line");
    for line in text.lines().filter(|line| line.contains("\" XY\"")) {
        let indent = line.len() - line.trim_start().len();
        assert!(
            indent > 0 && indent <= head.len(),
            "continuation indent {indent} does not point into the {}-column head",
            head.len()
        );
    }
}

#[test]
fn a_declaration_entity_after_an_array_constructor_is_not_a_named_argument() {
    // Reduced from CP2K. The continuation line carries no statement context, so
    // `, b =` after a closing `]` looked like a keyword argument and was
    // compacted to `b=`. Named arguments live in `(...)`; `[...]` is an array
    // constructor, and after its comma comes the next entity of the
    // declaration list.
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            enabled: false,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    for (source, expected) in [
        (
            "module m\n   real(kind=dp), parameter :: a = [1.0_dp, &\n      2.0_dp], b = &\n      [3.0_dp, 4.0_dp]\nend module m\n",
            "      2.0_dp], b = &",
        ),
        // The rule this guard narrows still has to do its job inside `(...)`.
        (
            "module m\n   real(kind=dp), parameter :: a = f(p=1, &\n      q=2)\nend module m\n",
            "      q=2)",
        ),
        (
            "program p\ncall f(a=1, &\n   b=2, &\n   c=3)\nend program p\n",
            "      b=2, &",
        ),
        // A continuation can close a bracket and land back inside the call, so
        // the decision belongs at the `=`, not to the line as a whole.
        (
            "program p\ncall g(sum([1, &\n   2, 3]), dim=1)\nend program p\n",
            "      2, 3]), dim=1)",
        ),
    ] {
        let once = format_source(source.as_bytes(), &config).unwrap().bytes;
        let text = String::from_utf8(once.clone()).unwrap();
        assert!(
            text.lines().any(|line| line == expected),
            "expected {expected:?} in\n{text}"
        );
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "full mode I1 failed for {source:?}");
    }
}

#[test]
fn wrapping_measures_the_declaration_separator_step_17_will_emit() {
    // Reduced from SPECFEM3D at `--line-length=80`. The author lined these
    // `::` up in a very wide block; step 17 compresses that block, so the line
    // the wrapper reads is 120 columns and the line it emits is 81. Wrapping
    // the authored spelling found no break that left the head inside the
    // budget and declined, and the next run — reading the compressed 81-column
    // line — wrapped it happily.
    // The authored `::` sits past column 80, so no break — not even one
    // immediately after the `::` — leaves the head inside the budget. Step 17
    // moves it to column 29, where a break after `=` fits.
    let source = "\
module m
   type t
      logical                                                                     :: dump_model_at_each_iteration = .true.
      logical                                                                     :: dump_descent_direction_at_each_iteration = .true.
      !! user-defined taper
      real(kind=CUSTOM_REAL)                                                      :: xmin_taper, xmax_taper
   end type t
end module m
";
    let config = FormatConfig {
        mode: FormatMode::Full,
        wrap: forformat::WrapConfig {
            line_length: 80,
            ..FormatConfig::default().wrap
        },
        ..FormatConfig::default()
    };
    let once = format_source(source.as_bytes(), &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&twice),
        "wrapping and step 17 disagreed about the emitted width"
    );
    for line in once.split(|byte| *byte == b'\n') {
        assert!(
            line.len() <= 80 || String::from_utf8_lossy(line).contains("::"),
            "line over budget: {}",
            String::from_utf8_lossy(line)
        );
    }
}

#[test]
fn end_keyword_spacing_stops_at_the_statement_it_owns() {
    // Both spellings reduced from SPECFEM3D. A compound rewrite (`endif` ->
    // `end if`) hands the next pass two tokens where there was one, so the
    // `end <keyword>` spacing rule saw a line it had not seen before and
    // reached past the keyword: into rule 5's preserved `!!` gap, and into an
    // empty gap in front of `;`, which turned a collapse into an insertion.
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    for (source, expected) in [
        (
            "program p\nif (x) then\ny = 1\nendif  !! trailing doc\nend program p\n",
            "   end if  !! trailing doc",
        ),
        (
            "program p\nif (x) then\ny = 1\nend if  !! trailing doc\nend program p\n",
            "   end if  !! trailing doc",
        ),
        (
            "program p\ndo i = 1, 2\ndo j = 1, 2\ny = 1\nenddo; enddo\nend program p\n",
            "      end do; enddo",
        ),
        (
            "program p\ndo i = 1, 2\ndo j = 1, 2\ny = 1\nend do; enddo\nend program p\n",
            "      end do; enddo",
        ),
        // The collapse this rule does own must survive the narrowing.
        (
            "module m\ncontains\nsubroutine s\ny = 1\nend subroutine    s\nend module m\n",
            "   end subroutine s",
        ),
    ] {
        let once = format_source(source.as_bytes(), &config).unwrap().bytes;
        let text = String::from_utf8(once.clone()).unwrap();
        assert!(
            text.lines().any(|line| line == expected),
            "expected {expected:?} in\n{text}"
        );
        let twice = format_source(&once, &config).unwrap().bytes;
        assert_eq!(once, twice, "full mode I1 failed for {source:?}");
    }
}

#[test]
fn full_mode_protected_spans_are_byte_exact() {
    let source = b"program p\ncharacter(len=20) :: s = 'IF  THEN  ' ! body  x = 1\nx = 4Hab  c\n#if defined(X)\nIF (X) THEN\n#endif\nend program p\n";
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let once = format_source(source, &config).unwrap().bytes;
    let collect = |bytes: &[u8]| {
        let mut strings = Vec::new();
        let mut hollerith = Vec::new();
        let mut cpp = Vec::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            if line.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'#') {
                cpp.push(line.to_vec());
            }
            for region in regions(line) {
                match region.kind {
                    RegionKind::StringLiteral => strings.push(line[region.range].to_vec()),
                    RegionKind::Hollerith => hollerith.push(line[region.range].to_vec()),
                    _ => {}
                }
            }
        }
        (strings, hollerith, cpp)
    };
    assert_eq!(collect(source), collect(&once));
}

#[test]
fn default_formatting_preserves_line_bodies_except_trailing_horizontal_space() {
    let source = b"program p  \r\n  x = \"a  b\"  \n! comment  \r\nend program";
    let output = format_source(source, &indent_only_config()).unwrap().bytes;
    assert_eq!(trimmed_line_bodies(source), line_bodies(&output));
}

#[test]
fn default_formatting_allows_only_label_padding_to_change() {
    let source = b"  program p\n10      continue ! keep  \n  end program p\n";
    let output = format_source(source, &indent_only_config()).unwrap().bytes;
    assert_eq!(
        normalized_line_bodies(source),
        normalized_line_bodies(&output)
    );
}

#[test]
fn whitespace_reduction_bypasses_hollerith_payloads() {
    let source = b"program p\nx = 4Ha  b ! comment\nend program p\n";
    let config = FormatConfig {
        ws_remred: true,
        mode: FormatMode::IndentOnly,
        ..FormatConfig::default()
    };
    let output = format_source(source, &config).unwrap().bytes;
    assert!(
        output
            .windows(b"4Ha  b ! comment".len())
            .any(|window| window == b"4Ha  b ! comment"),
        "Hollerith payload was changed: {output:?}"
    );
}

#[test]
fn streaming_api_matches_owned_api() {
    let source = b"program p\nif (x) then\nx = 1\nend if\nend program\n";
    let config = FormatConfig::default();
    let owned = format_source(source, &config).unwrap();
    let mut output = Vec::new();
    let meta = format_to(source, &config, &mut output).unwrap();
    assert_eq!(output, owned.bytes);
    assert_eq!(meta, owned.meta);

    let mut owned_output = Vec::new();
    let owned_meta = format_to_owned(source.to_vec(), &config, &mut owned_output).unwrap();
    assert_eq!(owned_output, owned.bytes);
    assert_eq!(owned_meta, owned.meta);
}

#[test]
fn full_streaming_api_matches_owned_api() {
    let source = b"PROGRAM p\nIF (x) THEN\nCALL f(a, b, c, d)\nEND IF\nEND PROGRAM p\n";
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let owned = format_source(source, &config).unwrap();
    let mut output = Vec::new();
    let meta = format_to(source, &config, &mut output).unwrap();
    assert_eq!(output, owned.bytes);
    assert_eq!(meta, owned.meta);

    let mut owned_output = Vec::new();
    let owned_meta = format_to_owned(source.to_vec(), &config, &mut owned_output).unwrap();
    assert_eq!(owned_output, owned.bytes);
    assert_eq!(owned_meta, owned.meta);
}

#[test]
fn unknown_statements_do_not_invent_structural_depth() {
    let source = b"program p\nif (x) then\neditor ???\ncontinue\nend if\nend program\n";
    let output = format_source(source, &indent_only_config()).unwrap().bytes;
    assert_eq!(
        output,
        b"program p\n   if (x) then\n      editor ???\n      continue\n   end if\nend program\n"
    );
}

#[test]
fn full_mode_unknown_statements_still_have_stable_structure() {
    let source = b"PROGRAM p\nIF (x) THEN\neditor ???\nCONTINUE\nEND IF\nEND PROGRAM p\n";
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    let once = format_source(source, &config).unwrap().bytes;
    let twice = format_source(&once, &config).unwrap().bytes;
    assert_eq!(once, twice);
    assert!(once
        .windows(b"editor ???".len())
        .any(|w| w == b"editor ???"));
}

#[test]
fn keyword_case_changes_spelling_but_not_indent_depth() {
    let lower = b"program p\nif (x) then\ncontinue\nelse\ncontinue\nend if\nend program\n";
    let upper = b"PROGRAM p\nIF (x) THEN\ncontinue\nELSE\ncontinue\nEND IF\nEND PROGRAM\n";
    let lower_output = format_source(lower, &FormatConfig::default())
        .unwrap()
        .bytes;
    let upper_output = format_source(upper, &FormatConfig::default())
        .unwrap()
        .bytes;
    assert_eq!(indent_columns(&lower_output), indent_columns(&upper_output));
}

#[test]
fn keyword_case_mutations_preserve_fixture_indent_depth() {
    let fixtures: &[&[u8]] = &[
        include_bytes!("fixtures/core.f90"),
        include_bytes!("fixtures/constructs.f90"),
        include_bytes!("fixtures/advanced_constructs.f90"),
        include_bytes!("fixtures/cpp_nested.f90"),
        include_bytes!("fixtures/legacy_controls.f90"),
    ];
    for source in fixtures {
        let upper: Vec<u8> = source.iter().map(u8::to_ascii_uppercase).collect();
        let original = format_source(source, &FormatConfig::default())
            .expect("original fixture formats")
            .bytes;
        let mutated = format_source(&upper, &FormatConfig::default())
            .expect("case-mutated fixture formats")
            .bytes;
        assert_eq!(indent_columns(&original), indent_columns(&mutated));
    }
}

#[test]
fn arbitrary_byte_inputs_are_total_without_utf8_assumptions() {
    for seed in 0u8..128 {
        let mut source = Vec::with_capacity(384);
        for index in 0..384u16 {
            let byte = seed
                .wrapping_mul(31)
                .wrapping_add(index as u8)
                .rotate_left((index % 8) as u32);
            source.push(if index % 29 == 0 { b'\n' } else { byte });
        }
        format_source(&source, &FormatConfig::default()).expect("arbitrary bytes are total");
    }
}

#[test]
fn arbitrary_non_ascii_bytes_in_comments_and_strings_are_transparent() {
    for value in 0x80u8..=0xff {
        let mut source = b"program p\n! comment ".to_vec();
        source.push(value);
        source.extend_from_slice(b"\nx = \"");
        source.push(value);
        source.extend_from_slice(b"\"  \nend program\n");
        let output = format_source(&source, &indent_only_config())
            .expect("non-UTF-8 source remains formatable")
            .bytes;
        assert_eq!(
            trimmed_line_bodies(&source),
            line_bodies(&output),
            "byte {value:#x}"
        );
    }
}

#[test]
fn source_and_logical_group_spans_stay_inside_the_input() {
    let fixtures: &[&[u8]] = &[
        include_bytes!("fixtures/core.f90"),
        include_bytes!("fixtures/lexical.f90"),
        include_bytes!("fixtures/align_nested.f90"),
        include_bytes!("fixtures/align_legacy_full.f90"),
        include_bytes!("fixtures/cpp_continuation.f90"),
        include_bytes!("fixtures/malformed_end.f90"),
        include_bytes!("fixtures/malformed_end_matrix.f90"),
        include_bytes!("fixtures/labeled_cpp_do.f90"),
        include_bytes!("fixtures/legacy_free_matrix.f90"),
    ];
    for source in fixtures {
        assert_valid_spans(source);
        for end in 0..=source.len() {
            assert_valid_spans(&source[..end]);
        }
    }
}

#[test]
fn fixture_prefixes_are_total_and_idempotent() {
    let fixtures: &[&[u8]] = &[
        include_bytes!("fixtures/core.f90"),
        include_bytes!("fixtures/lexical.f90"),
        include_bytes!("fixtures/align.f90"),
        include_bytes!("fixtures/align_nested.f90"),
        include_bytes!("fixtures/constructs.f90"),
        include_bytes!("fixtures/construct_options.f90"),
        include_bytes!("fixtures/advanced_constructs.f90"),
        include_bytes!("fixtures/benchmark.f90"),
        include_bytes!("fixtures/benchmark_continuation.f90"),
        include_bytes!("fixtures/benchmark_preprocessor.f90"),
        include_bytes!("fixtures/cli_layout.f90"),
        include_bytes!("fixtures/cpp_continuation.f90"),
        include_bytes!("fixtures/cpp_continuation_indent.f90"),
        include_bytes!("fixtures/cpp_nested.f90"),
        include_bytes!("fixtures/engine_options.f90"),
        include_bytes!("fixtures/fortran2023.f90"),
        include_bytes!("fixtures/label_matrix.f90"),
        include_bytes!("fixtures/legacy_controls.f90"),
        include_bytes!("fixtures/malformed_end.f90"),
        include_bytes!("fixtures/legacy_recovery.f90"),
        include_bytes!("fixtures/labeled_cpp_do.f90"),
        include_bytes!("fixtures/legacy_free_matrix.f90"),
        include_bytes!("fixtures/openmp_continuation.f90"),
        include_bytes!("fixtures/procedure_decl.f90"),
        include_bytes!("fixtures/procedure_matrix.f90"),
        include_bytes!("fixtures/refactor.f90"),
        include_bytes!("fixtures/query.f90"),
        include_bytes!("fixtures/structures.f90"),
        include_bytes!("fixtures/ws_full.f90"),
        include_bytes!("fixtures/ws_remred.f90"),
    ];
    for source in fixtures {
        let cuts = [
            0,
            source.len() / 3,
            source.len() / 2,
            source.len().saturating_sub(1),
            source.len(),
        ];
        for cut in cuts {
            let prefix = &source[..cut];
            let buffer = SourceBuffer::new(prefix).expect("prefix is within the byte-size limit");
            for line in &buffer.lines {
                assert!(line.span.start <= line.span.end);
                assert!(line.code_span.start <= line.code_span.end);
                assert!(line.span.end as usize <= prefix.len());
                assert!(line.code_span.end as usize <= prefix.len());
                if let Some(comment) = &line.comment_span {
                    assert!(comment.start <= comment.end);
                    assert!(comment.end as usize <= prefix.len());
                }
            }
            for group in LogicalGroup::assemble(&buffer) {
                assert!(group.lines.start <= group.lines.end);
                assert!(group.lines.end <= buffer.lines.len());
            }
            let once = format_source(prefix, &indent_only_config()).expect("formatter is total");
            let twice = format_source(&once.bytes, &indent_only_config())
                .expect("formatted prefix remains total");
            assert_eq!(
                once.bytes, twice.bytes,
                "prefix at {cut} was not idempotent"
            );
        }
    }
}

fn line_bodies(source: &[u8]) -> Vec<Vec<u8>> {
    let mut bodies = Vec::new();
    let mut start = 0;
    for (index, &byte) in source.iter().enumerate() {
        if byte == b'\n' {
            let mut end = index;
            if end > start && source[end - 1] == b'\r' {
                end -= 1;
            }
            bodies.push(body(&source[start..end]));
            start = index + 1;
        }
    }
    if start < source.len() {
        bodies.push(body(&source[start..]));
    }
    bodies
}

fn trimmed_line_bodies(source: &[u8]) -> Vec<Vec<u8>> {
    line_bodies(source)
        .into_iter()
        .map(|line| {
            let end = line
                .iter()
                .rposition(|byte| *byte != b' ' && *byte != b'\t')
                .map_or(0, |index| index + 1);
            line[..end].to_vec()
        })
        .collect()
}

fn assert_valid_spans(source: &[u8]) {
    let buffer = SourceBuffer::new(source).expect("source buffer accepts byte input");
    for line in &buffer.lines {
        assert!(line.span.end as usize <= source.len());
        assert!(line.code_span.start >= line.span.start);
        assert!(line.code_span.end <= line.span.end);
        if let Some(comment) = &line.comment_span {
            assert!(comment.start >= line.code_span.start);
            assert!(comment.end <= line.span.end);
        }
        let _ = buffer.line_bytes(line);
        let _ = buffer.code_bytes(line);
    }
    for group in LogicalGroup::assemble(&buffer) {
        assert!(group.lines.start <= group.lines.end);
        assert!(group.lines.end <= buffer.lines.len());
        for statement in group.statements {
            assert!(!statement.text.is_empty());
        }
    }
}

fn normalized_line_bodies(source: &[u8]) -> Vec<Vec<u8>> {
    trimmed_line_bodies(source)
        .into_iter()
        .map(|line| normalize_label_padding(&line))
        .collect()
}

fn body(line: &[u8]) -> Vec<u8> {
    let start = line
        .iter()
        .position(|byte| *byte != b' ' && *byte != b'\t')
        .unwrap_or(line.len());
    line[start..].to_vec()
}

fn indent_columns(source: &[u8]) -> Vec<usize> {
    let mut result = Vec::new();
    let mut start = 0;
    for (index, byte) in source.iter().enumerate() {
        if *byte == b'\n' {
            let end = if index > start && source[index - 1] == b'\r' {
                index - 1
            } else {
                index
            };
            let line = &source[start..end];
            result.push(
                line.iter()
                    .take_while(|byte| **byte == b' ' || **byte == b'\t')
                    .count(),
            );
            start = index + 1;
        }
    }
    if start < source.len() {
        result.push(
            source[start..]
                .iter()
                .take_while(|byte| **byte == b' ' || **byte == b'\t')
                .count(),
        );
    }
    result
}

fn normalize_label_padding(line: &[u8]) -> Vec<u8> {
    let digits = line.iter().take_while(|byte| byte.is_ascii_digit()).count();
    if digits == 0 || digits == line.len() || !line[digits].is_ascii_whitespace() {
        return line.to_vec();
    }
    let after_label = line[digits..]
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map(|offset| digits + offset)
        .unwrap_or(line.len());
    let mut normalized = line[..digits].to_vec();
    normalized.extend_from_slice(&line[after_label..]);
    normalized
}
