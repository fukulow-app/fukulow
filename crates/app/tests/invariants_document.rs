#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "Clippy's test options do not cover integration-test helper functions"
)]

const INVARIANTS: &str = include_str!("../../../docs/architecture/invariants.md");

// This document uses single-line, pipe-bordered tables. Keeping this parser narrow
// makes a format change fail visibly instead of introducing a Markdown dependency.
fn cells(line: &str) -> Option<Vec<&str>> {
    let line = line.trim().strip_prefix('|')?;
    let mut cells = Vec::new();
    let mut start = 0;
    for (index, character) in line.char_indices() {
        if character == '|' && !line[..index].ends_with('\\') {
            cells.push(line[start..index].trim());
            start = index + 1;
        }
    }
    if start < line.len() {
        cells.push(line[start..].trim());
    }
    Some(cells)
}

fn review_debt_errors(document: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let mut tables = 0;
    let mut rows = 0;
    let mut lines = document.lines().enumerate().peekable();
    while let Some((line_number, line)) = lines.next() {
        let Some(header) = cells(line) else {
            continue;
        };
        let Some(enforced_by) = header.iter().position(|cell| *cell == "Enforced by") else {
            continue;
        };
        tables += 1;
        let separator = lines.next().and_then(|(_, line)| cells(line));
        if !separator.is_some_and(|cells| {
            cells.len() == header.len()
                && cells.iter().all(|cell| {
                    let dashes = cell.trim_matches(':');
                    dashes.len() >= 3 && dashes.chars().all(|character| character == '-')
                })
        }) {
            errors.push(format!("line {}: invalid table separator", line_number + 1));
        }
        while let Some(&(line_number, line)) = lines.peek() {
            let Some(row) = cells(line) else {
                break;
            };
            lines.next();
            rows += 1;
            if row.len() != header.len() {
                errors.push(format!(
                    "line {}: row `{line}` has {} cells; expected {}",
                    line_number + 1,
                    row.len(),
                    header.len()
                ));
                continue;
            }
            let enforcement = row[enforced_by];
            let review = enforcement
                .split(|character: char| !character.is_alphanumeric() && character != '_')
                .any(|word| word.eq_ignore_ascii_case("review"));
            let issue = enforcement
                .as_bytes()
                .windows(2)
                .any(|pair| pair[0] == b'#' && pair[1].is_ascii_digit());
            if review && !issue {
                errors.push(format!(
                    "line {}: row `{line}` mentions review without an issue in Enforced by",
                    line_number + 1
                ));
            }
        }
    }
    if tables == 0 {
        errors.push("no invariant table with an Enforced by column found".to_owned());
    }
    if rows == 0 {
        errors.push("no invariant rows found".to_owned());
    }
    errors
}

#[test]
fn review_debt_names_an_issue() {
    let errors = review_debt_errors(INVARIANTS);
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

fn table(row: &str) -> String {
    format!("| Invariant | Enforced by | Lands in |\n|---|---|---|\n{row}\n")
}

#[test]
fn review_anywhere_requires_an_issue_in_the_same_cell() {
    for enforcement in ["Review", "Manual review", "checked in rEvIeW", "**REVIEW**"] {
        for citation in ["", " #", " #N", " # 58"] {
            let document = table(&format!(
                "| Named rule #58 | {enforcement}{citation} | #58 |"
            ));
            let errors = review_debt_errors(&document);
            assert_eq!(errors.len(), 1, "{document}");
            assert!(errors[0].contains("Named rule"), "{errors:?}");
            assert!(errors[0].contains("without an issue"), "{errors:?}");
        }
        let document = table(&format!("| Named rule | {enforcement} (#58) | in place |"));
        assert!(review_debt_errors(&document).is_empty(), "{document}");
    }
}

#[test]
fn other_enforcement_does_not_require_an_issue() {
    for enforcement in ["Constraint", "preview test", "reviewer", "reviews", ""] {
        assert!(
            review_debt_errors(&table(&format!("| Review #58 | {enforcement} | Review |")))
                .is_empty()
        );
    }
}

#[test]
fn escaped_pipes_do_not_shift_the_enforcement_column() {
    for row in [
        r"| Named \| rule | Review (#58) | in place |",
        r"| Named rule | Constraint \| test | in place |",
    ] {
        assert!(review_debt_errors(&table(row)).is_empty(), "{row}");
    }
    let errors = review_debt_errors(&table(r"| Named \| rule | checked in review | #58 |"));
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("without an issue"), "{errors:?}");
}

#[test]
fn every_row_must_match_its_header_width() {
    for row in [
        "| Short rule | Constraint |",
        "| Long rule | Constraint | in place | extra |",
        "| Empty cell rule | Constraint | in place ||",
    ] {
        let errors = review_debt_errors(&table(row));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains(row), "{errors:?}");
        assert!(errors[0].contains("expected 3"), "{errors:?}");
    }
}

#[test]
fn every_invariant_table_uses_its_own_header() {
    let document = format!(
        "{}\n| Unrelated | Table |\n|---|---|\n| Ignore | Review |\n\n\
         | Enforced by | Invariant |\n|:---|---:|\n| Review | Later rule |\n",
        table("| First rule | Constraint | in place |")
    );
    let errors = review_debt_errors(&document);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("Later rule"), "{errors:?}");
}

#[test]
fn missing_tables_rows_or_valid_separators_fail() {
    for document in [
        "",
        "| Invariant | Mechanism |\n|---|---|\n| Rule | Review |",
    ] {
        assert!(
            review_debt_errors(document)
                .iter()
                .any(|error| error.contains("no invariant table"))
        );
    }
    assert!(
        review_debt_errors("| Invariant | Enforced by |\n|---|---|")
            .iter()
            .any(|error| error.contains("no invariant rows"))
    );
    let errors = review_debt_errors("| Invariant | Enforced by |\n|---|\n| Rule | Constraint |");
    assert!(
        errors
            .iter()
            .any(|error| error.contains("invalid table separator"))
    );
}
