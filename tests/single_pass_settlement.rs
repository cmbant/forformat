//! A rule must not leave work for the next run of the formatter.
//!
//! Every case below was an I1 break of the same shape: one rule finished a
//! rewrite that a second rule was waiting on, and the second rule only ran over
//! the result on the *next* invocation. They are not one defect -- the three
//! groups live in the operator rules, the number-case rule and the declared-name
//! pass -- but they share a tell worth recognizing, which is two pieces of code
//! reading the same construct through different evidence: raw bytes against
//! tokens, a split spelling against a joined one.
//!
//! Each test therefore pins both halves: the input settles in one pass, *and*
//! the spelling it settles on is the one the old two-pass path converged to. A
//! test that only asserted idempotence would pass just as well if the rule had
//! been turned off.

use forformat::{format_source, FormatConfig, FormatMode};

fn full(source: &[u8]) -> Vec<u8> {
    let config = FormatConfig {
        mode: FormatMode::Full,
        ..FormatConfig::default()
    };
    format_source(source, &config).unwrap().bytes
}

/// Format once, require a fixed point, and return the settled bytes.
fn settled(source: &[u8]) -> Vec<u8> {
    let once = full(source);
    let twice = full(&once);
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(&twice),
        "{:?} did not settle in one pass",
        String::from_utf8_lossy(source),
    );
    once
}

fn settles_to(source: &[u8], expected: &[u8]) {
    let once = settled(source);
    assert_eq!(
        String::from_utf8_lossy(&once),
        String::from_utf8_lossy(expected),
        "{:?}",
        String::from_utf8_lossy(source),
    );
}

/// Operator spacing read the bytes beside a token to decide it was glued to its
/// neighbour -- and those bytes are what the same pass rewrites. Spacing the
/// `=` in `x=<1` unglued the `<` for the *next* run to space, so a glued run
/// came apart one operator per run.
#[test]
fn a_glued_operator_run_is_separated_in_one_pass() {
    for (source, expected) in [
        (b"x=<1\n".as_slice(), b"x = < 1\n".as_slice()),
        (b"x ==<1\n", b"x == < 1\n"),
        (b"x<<=1\n", b"x < <= 1\n"),
        (b"x>>=1\n", b"x > >= 1\n"),
        (b"x->1\n", b"x - > 1\n"),
        (b"x===1\n", b"x == = 1\n"),
    ] {
        settles_to(source, expected);
    }
}

/// The guard itself is kept: a run whose members all decline to be spaced stays
/// glued, because the pair is likelier to be one thing these rules do not model
/// than two things they do.
#[test]
fn a_glued_run_that_nothing_separates_is_left_alone() {
    for source in [b"x<<1\n".as_slice(), b"x<>1\n", b"x><1\n"] {
        settles_to(source, source);
    }
}

/// Only the operators that declined *because* they were glued are reconsidered.
/// An operator that is compact for a reason of its own keeps it, so a spaced
/// neighbour does not pull a unary sign or a keyword argument's `=` open.
#[test]
fn a_compact_operator_is_not_pulled_open_by_a_spaced_neighbour() {
    settles_to(b"x =-1\n", b"x = -1\n");
    settles_to(b"x =*1\n", b"x = *1\n");
    settles_to(b"call f(a=-1)\n", b"call f(a=-1)\n");
}

/// Keeping two tokens apart is about where their boundary falls, not how many
/// tokens there are. `=` and `==` written together spell `===`, still two
/// tokens -- but they are `==` and `=`, a different pair that the next run
/// spaces differently. `*` and `**` are the same trap one operator longer.
#[test]
fn a_join_that_moves_a_token_boundary_is_refused() {
    settles_to(b"call f(a= ==1)\n", b"call f(a= == 1)\n");
    settles_to(b"x * * ** 1\n", b"x* * **1\n");
}

/// `1D+ 1` is one literal with a blank in it. Whitespace reduction closed the
/// blank, but the marker's case rule wanted the number token that closing left
/// behind, so it cased the `D` one run later.
#[test]
fn a_real_exponent_split_by_a_blank_is_cased_in_one_pass() {
    for (source, expected) in [
        (b"a = 1D+ 1\n".as_slice(), b"a = 1d+1\n".as_slice()),
        (b"a = 1D- 1\n", b"a = 1d-1\n"),
        (b"a = 1E+ 1\n", b"a = 1e+1\n"),
        (b"a = 1.5D+ 10\n", b"a = 1.5d+10\n"),
        (b"a = .5D+ 1\n", b"a = .5d+1\n"),
        (b"a = 1D+ 1 + 2\n", b"a = 1d+1 + 2\n"),
    ] {
        settles_to(source, expected);
    }
}

/// The mantissa is not always one token: a trailing `.` with no digits after it
/// is not part of the number, so `1.E- 10` lexes as `1` `.` `E` `-` `10` and
/// the marker is two tokens from the number rather than one.
#[test]
fn a_mantissa_that_ends_in_a_point_is_still_a_real_literal() {
    settles_to(b"a = 1.E- 10\n", b"a = 1.e-10\n");
    settles_to(b"a = 1.E+ 2\n", b"a = 1.e+2\n");
    settles_to(b"a = 1.d- 3\n", b"a = 1.d-3\n");
}

/// The `.` that closes a dotted operator is not a mantissa. `1.and.E-3` is
/// `.and.` between a number and the name `E`, and reading its `.` as a mantissa
/// made the `-` an exponent sign while the operator was glued and an ordinary
/// subtraction once it had been spaced.
#[test]
fn a_dotted_operator_does_not_supply_a_mantissa() {
    settles_to(b"a = 1.and.E-3\n", b"a = 1 .and. E - 3\n");
    settles_to(b"a = 1.and.E- 3\n", b"a = 1 .and. E - 3\n");
    settles_to(b"a = 1.or.D-2\n", b"a = 1 .or. D - 2\n");
}

/// The exponent has to be complete for any of that to apply. Without the
/// digits after the sign there is no exponent and no blank to close, and a
/// `D`-suffixed literal beside a name keeps the spelling the author gave it.
#[test]
fn a_d_suffixed_literal_that_is_no_exponent_keeps_its_case() {
    settles_to(b"a = 1D+ x\n", b"a = 1D+x\n");
    settles_to(b"a = 1D+\n", b"a = 1D+\n");
    settles_to(b"a = 1D + 1\n", b"a = 1D + 1\n");
}

/// The declared-name pass read `end type t` but not `endtype t`, so on the run
/// that split the keyword the statement carried no name for it to recase, and
/// the name was taken from its governing spelling only on the run after.
#[test]
fn a_joined_end_keyword_carries_its_construct_name() {
    settles_to(
        b"type::t_Name\nendtype t_NAME\ntype(t_name\n",
        b"type :: t_Name\nend type t_Name\ntype(t_Name\n",
    );
}

/// A use site says a type exists; the definition says what it is called. When
/// the use site was allowed to say both, one miscased `type(t_name)` made
/// `t_Name` ambiguous for the whole file -- including at its own definition, so
/// `end type t_NAME` could not be corrected. The use site could be, through the
/// declared-types table, which holds definitions only; and with it corrected the
/// ambiguity was gone and the `end type` name moved on the pass after.
#[test]
fn a_miscased_use_site_does_not_veto_a_types_own_definition() {
    settles_to(
        b"type::t_Name\nend type\nend type t_NAME\ntype(t_name) :: v\n",
        b"type :: t_Name\nend type\nend type t_Name\ntype(t_Name) :: v\n",
    );
}

/// The other half of the same rule: a use site is still enough to *know* the
/// name, so a type nothing here defines keeps whatever each site spells it. The
/// file has no authority to make one of them govern the others.
#[test]
fn a_type_no_one_defines_keeps_every_spelling_it_was_given() {
    settles_to(
        b"type(t_name) :: v\ntype(T_NAME) :: w\n",
        b"type(t_name) :: v\ntype(T_NAME) :: w\n",
    );
}

/// The two spellings of a head are the same statement, so they must reach the
/// same answer -- which is the property that makes the fix above a
/// generalization rather than a special case for `endtype`.
#[test]
fn both_spellings_of_a_named_end_agree() {
    for (joined, split) in [
        (
            b"endmodule m_NAME\nmodule m_name\n".as_slice(),
            b"end module m_NAME\nmodule m_name\n".as_slice(),
        ),
        (
            b"type::t_Name\nendtype t_NAME\ntype(t_name\n",
            b"type::t_Name\nend type t_NAME\ntype(t_name\n",
        ),
    ] {
        assert_eq!(
            String::from_utf8_lossy(&settled(joined)),
            String::from_utf8_lossy(&settled(split)),
            "the joined and split spellings of {:?} disagree",
            String::from_utf8_lossy(joined),
        );
    }
}

/// The gap between a compound keyword and the `(` after it is governed by the
/// keyword's *last* word, which the same pass is still writing. Reading only
/// the authored spelling meant `endtype (` had no rule until the run after the
/// split, so `endtype (x)` needed two.
#[test]
fn a_split_keyword_governs_the_gap_before_its_parenthesis() {
    settles_to(b"endtype (x)\n", b"end type(x)\n");
    settles_to(b"endassociate (x)\n", b"end associate(x)\n");
}

/// The same rule keeps the blank where the governing word wants one, so this
/// is not "close every gap before a `(`": `if` takes one space and `endif`
/// ends in `if`.
#[test]
fn a_split_keyword_that_wants_its_gap_keeps_it() {
    settles_to(b"endif (x)\n", b"end if (x)\n");
    settles_to(b"endselect (x)\n", b"end select (x)\n");
    settles_to(b"elseif (x) then\n", b"else if (x) then\n");
}

/// Both spellings of the head are the same statement here too.
#[test]
fn both_spellings_of_a_split_keyword_agree_about_the_gap() {
    for (joined, split) in [
        (b"endtype (x)\n".as_slice(), b"end type (x)\n".as_slice()),
        (b"endif (x)\n", b"end if (x)\n"),
        (b"endselect (x)\n", b"end select (x)\n"),
    ] {
        assert_eq!(
            String::from_utf8_lossy(&settled(joined)),
            String::from_utf8_lossy(&settled(split)),
            "the joined and split spellings of {:?} disagree",
            String::from_utf8_lossy(joined),
        );
    }
}

/// A head that is a name rather than a keyword keeps its own spelling, so the
/// split's own preconditions still gate the substitution.
#[test]
fn a_shadowed_or_assigned_head_is_not_read_as_a_split_keyword() {
    settles_to(b"endtype = 1\n", b"endtype = 1\n");
    settles_to(b"endif = 2\n", b"endif = 2\n");
}

/// A split head owns its whole seam. `selecttype(a)` splits into a selector
/// whose blank belongs to a rule that matches on `select` followed by `type`,
/// and `selectrank(a)` into one that belongs to another such rule; neither can
/// see a seam that the split has not written yet, so both spelt it a pass late.
#[test]
fn a_split_selector_gets_its_blank_in_one_pass() {
    settles_to(b"selecttype(a)\n", b"select type (a)\n");
    settles_to(b"selectrank(a)\n", b"select rank (a)\n");
}

/// `select case` is not a selector: no rule claims that seam, so it is the
/// author's, and both spellings of the head leave it as written.
#[test]
fn a_split_case_keeps_the_authored_gap() {
    settles_to(b"selectcase(x)\n", b"select case(x)\n");
    settles_to(b"select case(x)\n", b"select case(x)\n");
    settles_to(b"selectcase (x)\n", b"select case (x)\n");
}

/// Only the split head is claimed, so the rules that own the authored spelling
/// keep owning it. Two rules inserting one space at one offset would put two
/// spaces in — see [`an_inserted_separator_has_one_owner`].
#[test]
fn an_authored_selector_keeps_its_own_rule() {
    for (joined, split) in [
        (
            b"selecttype(a)\n".as_slice(),
            b"select type (a)\n".as_slice(),
        ),
        (b"selectrank(a)\n", b"select rank (a)\n"),
        (b"selectcase (x)\n", b"select case (x)\n"),
    ] {
        assert_eq!(
            String::from_utf8_lossy(&settled(joined)),
            String::from_utf8_lossy(&settled(split)),
        );
    }
    settles_to(b"select type(a)\n", b"select type (a)\n");
}

/// An `EditBuffer` keeps two zero-width insertions at the same offset, so a
/// gap with two owners gets two spaces. `elseif(a)` had exactly that once the
/// split spelling reached the gap rule: the separator the split creates is part
/// of the spelling and was written unconditionally, and the gap rule wrote it
/// again. It came out `else if  (a)` and needed a second pass to shrink.
#[test]
fn an_inserted_separator_has_one_owner() {
    settles_to(b"elseif(a)\n", b"else if (a)\n");
    settles_to(b"elseif (a)\n", b"else if (a)\n");
    settles_to(b"elseif(a) then\n", b"else if (a) then\n");
}

/// The one break in this crate that never converged. Two rules read the second
/// `)` of `if (a) ) x = 1` differently -- with the blank it looks like the
/// statement the condition guards, without it like a second closing
/// parenthesis -- and each wrote the other's input, so the line alternated for
/// ever. Unlike every other case here there was no later pass whose answer
/// could be adopted as the fixed point.
///
/// The one thing that cannot be the statement a condition guards is a closing
/// delimiter, and refusing that one token is the whole fix.
#[test]
fn an_unbalanced_condition_settles() {
    settles_to(b"if (a) ) x = 1\n", b"if (a)) x = 1\n");
    settles_to(b"if (a) ) then\n", b"if (a)) then\n");
    settles_to(b"if (a) ] x = 1\n", b"if (a)] x = 1\n");
    settles_to(b"if ((a) x = 1\n", b"if ((a) x = 1\n");
    // Already this spelling, and already a fixed point before the guard.
    settles_to(b"x = (a) )\n", b"x = (a))\n");
}

/// The refusal is as narrow as the disagreement. Only the condition's own gap
/// is declined; case, keyword spelling, comma spacing and plain delimiter
/// adjacency all still run over the broken statement and over its neighbours.
///
/// Skipping more than this was tried and cost `a > t <]` its fixed point: the
/// operator rules space a `<` away from the `]` after it and the delimiter rule
/// closes that gap back up in the same pass, so silencing the second one left
/// the first unanswered.
#[test]
fn an_unbalanced_statement_is_still_formatted() {
    settles_to(
        b"IF (a) ) CALL f(1 ,2)\ny = ( 1 )\n",
        b"if (a)) call f(1, 2)\ny = (1)\n",
    );
    settles_to(b"a > t <]\n", b"a > t <]\n");
    settles_to(b"if (a) ) x = 1; y = ( 2 )\n", b"if (a)) x = 1; y = (2)\n");
}

/// Nothing outside the condition is consulted, so nothing outside it can cost
/// the condition its gap. Asking instead whether the *statement*'s delimiters
/// balanced was tried, and it charged a sound condition for a broken
/// neighbour's fault -- and, because a statement continued onto another line
/// cannot balance within it, for its own continuation as well.
#[test]
fn nothing_outside_the_condition_costs_it_its_gap() {
    settles_to(b"if(a)x=1; y=)\n", b"if (a) x = 1; y =)\n");
    settles_to(
        b"if (a)x = f( &\n   1); y =)\n",
        b"if (a) x = f( &\n   1); y =)\n",
    );
    settles_to(b"x=(; if(b)y=1\n", b"x = (; if (b) y = 1\n");
    // The broken condition itself is still declined.
    settles_to(b"if (a) ) x = 1; y = 2\n", b"if (a)) x = 1; y = 2\n");
}

/// A continuation carved out of an argument list is unbalanced by construction,
/// which is why no guard here may ask a physical line whether its delimiters
/// close.
#[test]
fn a_continuation_is_not_read_as_an_unbalanced_statement() {
    settles_to(b"call f( a, &\n     b )\n", b"call f(a, &\n   b)\n");
}

/// The comma rule walks backwards over blanks, and its output buffer holds
/// every region of the line, so the walk ran out of the code region it was
/// given and into the Hollerith payload before it. `5h` still claimed five
/// characters, so the constant silently changed from `abcd ` to `abcd,`.
///
/// Not an idempotence break -- it was a fixed point, and a fixed point is what
/// makes it worse: nothing downstream would ever have noticed.
#[test]
fn a_hollerith_payload_keeps_its_blanks() {
    settles_to(b"call p(5habcd  ,y)\n", b"call p(5habcd , y)\n");
    settles_to(b"call p(3ha  ,y)\n", b"call p(3ha  , y)\n");
    settles_to(b"call p(5hABCD  ,y)\n", b"call p(5hABCD , y)\n");
    // The blanks outside the payload are still the rule's to remove.
    settles_to(b"call f(a  ,b)\n", b"call f(a, b)\n");
}

/// The other way to destroy a Hollerith constant is from outside it. A
/// Hollerith region begins at its own `4h` prefix, so `REAL 4habcd` splits into
/// the code region `REAL ` and the Hollerith `4habcd`, and the old-style
/// declaration squeeze read that region's trailing blank as trailing
/// whitespace. `real4habcd` is one identifier: not a byte of the payload
/// changed, and the constant was gone.
///
/// Found by `FUZZ_TIME=180 ./tools/check_fuzz_regression.sh` as an I3 failure
/// in the `properties` target, reduced from a 505-byte artifact.
#[test]
fn a_declaration_is_not_squeezed_onto_a_hollerith_constant() {
    settles_to(b"REAL 4habcd\n", b"real 4habcd\n");
    settles_to(b"integer 2hab\n", b"integer 2hab\n");
    settles_to(b"REAL 4habcd, y\n", b"real 4habcd, y\n");
    // The squeeze still squeezes: an old-style declaration with no protected
    // region loses its extra blanks and its trailing ones.
    settles_to(b"REAL  A(3), B(4)\n", b"real A(3), B(4)\n");
    settles_to(b"integer i, j  \n", b"integer i, j\n");
    // And a trailing comment is not a region the blank has to reach: the gap in
    // front of one belongs to the comment rule, which sizes it from the code
    // that ends up before it. Keeping a blank for the comment's sake handed
    // that rule one it had not asked for, and it took it back out on the next
    // pass -- so this line, a fixed point before, needed two.
    settles_to(b"REAL 4habcd ! c\n", b"real 4habcd ! c\n");
    settles_to(b"real a   ! c\n", b"real a ! c\n");
    settles_to(b"character(8) rh   ! rah\n", b"character(8) rh ! rah\n");
}
