//! What qualification reads from the repository (#122): the acceptance
//! matrix's scenarios, the crate READMEs' scenario-to-test claims, and which
//! packaged tests only run on hook-enabled debug builds.

use super::Layer;
use std::collections::BTreeSet;

/// One acceptance-matrix scenario: its ID, setup, required result and
/// required layers (docs/for-humans/behaviour/monitor/acceptance.md).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Scenario {
    pub(crate) id: String,
    pub(crate) setup: String,
    pub(crate) expected: String,
    pub(crate) layers: Vec<Layer>,
}

/// Whether `text` is a scenario ID: three or four capitals, a hyphen and two
/// digits, such as `ENT-01` or `PERF-01`.
pub(crate) fn is_id(text: &str) -> bool {
    let Some((prefix, number)) = text.split_once('-') else {
        return false;
    };
    let capitals =
        (3..=4).contains(&prefix.len()) && prefix.bytes().all(|b| b.is_ascii_uppercase());
    capitals && number.len() == 2 && number.bytes().all(|b| b.is_ascii_digit())
}

/// A Markdown table row's cells, trimmed, or none for any other line.
fn cells(line: &str) -> Option<Vec<&str>> {
    let inner = line.strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(str::trim).collect())
}

/// Every scenario of the acceptance matrix, in document order: each table
/// row whose first cell is a scenario ID and which has its four cells. An
/// unknown layer or a repeated ID is an error, never skipped.
pub(crate) fn scenarios(matrix: &str) -> Result<Vec<Scenario>, String> {
    let mut scenarios: Vec<Scenario> = Vec::new();
    for row in matrix.lines().filter_map(cells) {
        let [id, setup, expected, layers] = row[..] else {
            continue;
        };
        if !is_id(id) {
            continue;
        }
        if scenarios.iter().any(|scenario| scenario.id == id) {
            return Err(format!("scenario {id} is listed twice"));
        }
        let layers = layers
            .split(',')
            .map(|name| {
                Layer::parse(name.trim()).ok_or_else(|| format!("{id}: unknown layer {name:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        scenarios.push(Scenario {
            id: id.to_owned(),
            setup: setup.to_owned(),
            expected: expected.to_owned(),
            layers,
        });
    }
    Ok(scenarios)
}

/// What one README row claims for one scenario: the layers its tests
/// prove, or that they only supplement them (a pure supplement proves no
/// layer, docs/for-humans/behaviour/monitor/acceptance.md "Contract"), and the tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Claim {
    pub(crate) id: String,
    pub(crate) layers: Vec<Layer>,
    pub(crate) supplement: bool,
    pub(crate) tests: Vec<String>,
    /// The README the row is in.
    pub(crate) source: String,
}

/// The claims of every scenario-to-test row in `readme` (at `source`) that
/// names a scenario of `ids`. Each ID's layers are named between it and the
/// next ID, before any colon: `DEL-01 (Monitor, Fault: …)` or
/// `LIB-03 Library and Fault: …`. An ID that names none takes the next
/// one's, as in `LIB-03, LIB-04 and LIB-05 Library: …`. Tests are the
/// backticked test names of the row's other cells.
pub(crate) fn claims(readme: &str, source: &str, ids: &BTreeSet<String>) -> Vec<Claim> {
    let mut claims = Vec::new();
    for row in readme.lines().filter_map(cells) {
        let Some((first, rest)) = row.split_first() else {
            continue;
        };
        let tests = tests(&rest.join(" | "));
        if tests.is_empty() {
            continue;
        }
        for (id, layers, supplement) in attributed(first, ids) {
            claims.push(Claim {
                id,
                layers,
                supplement,
                tests: tests.clone(),
                source: source.to_owned(),
            });
        }
    }
    claims
}

/// Each known ID in `cell` with the layers it names (or the next ID's) and
/// whether it is only a supplement.
fn attributed(cell: &str, ids: &BTreeSet<String>) -> Vec<(String, Vec<Layer>, bool)> {
    let found = occurrences(cell, ids);
    let mut attributed: Vec<(String, Vec<Layer>, bool)> = Vec::new();
    let mut inherited = (Vec::new(), false);
    for (index, (id, end)) in found.iter().enumerate().rev() {
        let next = found
            .get(index + 1)
            .map_or(cell.len(), |(_, end)| *end - found[index + 1].0.len());
        let named = named(&cell[*end..next]);
        if !named.0.is_empty() || named.1 {
            inherited = named;
        }
        attributed.push((id.clone(), inherited.0.clone(), inherited.1));
    }
    attributed.reverse();
    attributed
}

/// Every known scenario ID in `cell`, with the byte offset just past it.
fn occurrences(cell: &str, ids: &BTreeSet<String>) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    let bytes = cell.as_bytes();
    for (start, _) in cell.char_indices() {
        let boundary = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let candidate = ids
            .iter()
            .find(|id| boundary && cell[start..].starts_with(id.as_str()));
        let end = candidate.map(|id| start + id.len());
        let after =
            end.is_some_and(|end| bytes.get(end).is_none_or(|b| !b.is_ascii_alphanumeric()));
        if let (Some(id), Some(end), true) = (candidate, end, after) {
            found.push((id.clone(), end));
        }
    }
    found
}

/// The layers `scope` names before its first colon, and whether it calls
/// itself a supplement.
fn named(scope: &str) -> (Vec<Layer>, bool) {
    let head = scope.split(':').next().unwrap_or_default();
    let layers = Layer::ALL
        .into_iter()
        .filter(|layer| head.contains(layer.name()))
        .collect();
    (layers, head.contains("supplement"))
}

/// The backticked test names in `cell`: snake-case paths, optionally ending
/// in `::*`; file names, crate names and other code are not tests.
pub(crate) fn tests(cell: &str) -> Vec<String> {
    cell.split('`')
        .skip(1)
        .step_by(2)
        .filter(|token| is_test(token))
        .map(str::to_owned)
        .collect()
}

fn is_test(token: &str) -> bool {
    let path = token.strip_suffix("::*").unwrap_or(token);
    let snake = |segment: &str| {
        !segment.is_empty()
            && segment.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
            && segment
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    let named = path.contains('_') || path.len() < token.len();
    named && path.split("::").all(snake)
}

/// The tests of `source` (a Rust test file) that return at once when
/// `CLIMON2_E2E_ARCHIVE` names a release archive: they only run on
/// hook-enabled debug builds, so passing against an archive proves nothing.
pub(crate) fn hooked_only(source: &str) -> Result<BTreeSet<String>, String> {
    let file = syn::parse_file(source).map_err(|error| error.to_string())?;
    let tests = file.items.iter().filter_map(|item| match item {
        syn::Item::Fn(function) if is_test_fn(function) => Some(function),
        _ => None,
    });
    Ok(tests
        .filter(|function| returns_for_archive(function))
        .map(|function| function.sig.ident.to_string())
        .collect())
}

fn is_test_fn(function: &syn::ItemFn) -> bool {
    function
        .attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

/// Whether the function's first statement is `if <archive> { return; }`.
fn returns_for_archive(function: &syn::ItemFn) -> bool {
    let Some(syn::Stmt::Expr(syn::Expr::If(first), _)) = function.block.stmts.first() else {
        return false;
    };
    let condition = quote::ToTokens::to_token_stream(&first.cond).to_string();
    let archive = condition.contains("CLIMON2_E2E_ARCHIVE") || condition.ends_with("archive ()");
    let returns = matches!(
        first.then_branch.stmts.as_slice(),
        [syn::Stmt::Expr(syn::Expr::Return(_), _)]
    );
    archive && returns && first.else_branch.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|id| (*id).to_owned()).collect()
    }

    /// The matrix's rows become scenarios with their layers; rows of other
    /// tables, the header and the separator are not scenarios.
    #[test]
    fn matrix_rows_become_scenarios_with_their_layers() {
        let matrix = "\
| Layer | Required meaning |
|---|---|
| Contract | Deterministic. |

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| ENT-01 | Start it. | It opens. | Monitor |
| PERF-01 | Run. | Report. | Measure |
| FOC-02 | Copy. | Recorded. | Monitor, Host manual |
";
        let scenarios = scenarios(matrix).unwrap();
        assert_eq!(
            scenarios,
            [
                Scenario {
                    id: "ENT-01".into(),
                    setup: "Start it.".into(),
                    expected: "It opens.".into(),
                    layers: vec![Layer::Monitor],
                },
                Scenario {
                    id: "PERF-01".into(),
                    setup: "Run.".into(),
                    expected: "Report.".into(),
                    layers: vec![Layer::Measure],
                },
                Scenario {
                    id: "FOC-02".into(),
                    setup: "Copy.".into(),
                    expected: "Recorded.".into(),
                    layers: vec![Layer::Monitor, Layer::HostManual],
                },
            ]
        );
    }

    /// An unknown layer or a scenario listed twice is an error.
    #[test]
    fn unknown_layers_and_repeated_ids_are_errors() {
        let unknown = "| ENT-01 | a | b | Monitor, Smoke |\n";
        assert_eq!(
            scenarios(unknown),
            Err("ENT-01: unknown layer \" Smoke\"".to_owned())
        );
        let twice = "| ENT-01 | a | b | Monitor |\n| ENT-01 | c | d | Fault |\n";
        assert_eq!(
            scenarios(twice),
            Err("scenario ENT-01 is listed twice".to_owned())
        );
    }

    /// IDs are three or four capitals, a hyphen and two digits.
    #[test]
    fn scenario_ids_have_their_exact_shape() {
        for id in ["ENT-01", "PERF-01", "DOC-02"] {
            assert!(is_id(id), "{id}");
        }
        for not in [
            "EN-01", "PERFS-01", "ENT-1", "ENT-001", "ent-01", "ENT01", "ENT-0a",
        ] {
            assert!(!is_id(not), "{not}");
        }
    }

    /// Each ID takes the layers named after it, before its colon, or the
    /// next ID's when it names none; a supplement proves no layer.
    #[test]
    fn readme_rows_claim_layers_per_id() {
        let known = ids(&[
            "DEL-01", "DEL-02", "LIB-03", "LIB-04", "REC-01", "COM-05", "DOC-01",
        ]);
        let readme = "\
| Scenario | Named test |
|---|---|
| DEL-01 (Monitor: default Cancel), DEL-02 (Fault: a rejected Kill) (#117) | `a_held_kill` (e2e, Fault peers) |
| LIB-03, LIB-04 Library and Fault: Monitor-free text | `create_runs`, `snapshot` (`instance.rs`) |
| REC-01, COM-05 (pure supplement over the fake library): Monitor | `retried_after_cleanup` (`src/supervisor.rs`) |
| DOC-01: null handles are rejected | `rejects_nulls` |
| #74 risk proof | `cargo_builds` |
| DEL-01 (Monitor) | no test named |
";
        let claims = claims(readme, "crates/x/README.md", &known);
        let summary: Vec<_> = claims
            .iter()
            .map(|claim| {
                (
                    claim.id.as_str(),
                    claim.layers.clone(),
                    claim.supplement,
                    claim.tests.clone(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "DEL-01",
                    vec![Layer::Monitor],
                    false,
                    vec!["a_held_kill".to_owned()]
                ),
                (
                    "DEL-02",
                    vec![Layer::Fault],
                    false,
                    vec!["a_held_kill".to_owned()]
                ),
                (
                    "LIB-03",
                    vec![Layer::Library, Layer::Fault],
                    false,
                    vec!["create_runs".to_owned()]
                ),
                (
                    "LIB-04",
                    vec![Layer::Library, Layer::Fault],
                    false,
                    vec!["create_runs".to_owned()]
                ),
                (
                    "REC-01",
                    vec![],
                    true,
                    vec!["retried_after_cleanup".to_owned()]
                ),
                (
                    "COM-05",
                    vec![],
                    true,
                    vec!["retried_after_cleanup".to_owned()]
                ),
                ("DOC-01", vec![], false, vec!["rejects_nulls".to_owned()]),
            ]
        );
        assert!(
            claims
                .iter()
                .all(|claim| claim.source == "crates/x/README.md")
        );
    }

    /// An ID inside a longer word, or one the matrix does not list, claims
    /// nothing.
    #[test]
    fn only_whole_known_ids_claim() {
        let known = ids(&["LIM-04"]);
        let readme = "| XLIM-04 (Monitor), LIM-045 (Fault), LIM-09 (Monitor) | `a_test` |\n";
        assert!(claims(readme, "r", &known).is_empty());
        let readme = "| – LIM-04 (Monitor) – | `a_test` |\n";
        assert_eq!(claims(readme, "r", &known).len(), 1);
    }

    /// Test names are backticked snake-case paths, globs included; files,
    /// crates, settings and code are not.
    #[test]
    fn only_snake_case_test_names_are_tests() {
        let cell = "`a_test`, `matrix::tests::the_case`, `workload::tests::*` \
                    (`src/x.rs`, `climon2-e2e`, `tests/build.rs`), `ui.fps`, `Foo_bar`, \
                    `snapshot`, `ESC [ 3 ~`, `a test`";
        assert_eq!(
            tests(cell),
            ["a_test", "matrix::tests::the_case", "workload::tests::*"]
        );
    }

    /// A test whose first statement returns when an archive is named is
    /// hooked only; one that checks later, branches, or is not a test is
    /// not.
    #[test]
    fn tests_returning_at_once_for_an_archive_are_hooked_only() {
        let source = r#"
            #[test]
            fn hooked_by_variable() {
                if std::env::var_os("CLIMON2_E2E_ARCHIVE").is_some() {
                    return;
                }
                run();
            }
            #[test]
            fn hooked_by_helper() {
                if archive() {
                    return;
                }
            }
            #[test]
            fn checks_later() {
                run();
                if archive() {
                    return;
                }
            }
            #[test]
            fn branches() {
                if archive() {
                    return;
                } else {
                    run();
                }
            }
            #[test]
            fn does_more() {
                if archive() {
                    skip();
                    return;
                }
            }
            #[test]
            fn other_condition() {
                if ready() {
                    return;
                }
            }
            fn helper() {
                if archive() {
                    return;
                }
            }
        "#;
        assert_eq!(
            hooked_only(source).unwrap(),
            ids(&["hooked_by_helper", "hooked_by_variable"])
        );
        assert!(hooked_only("fn (").is_err());
    }
}
