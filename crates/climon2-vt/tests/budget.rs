//! Models per memory budget at the PERF-01 geometries (#166), the figures
//! ADR 0007 records. A change to what a model charges fails here and prints
//! the measured table, to be recorded in the ADR in its place.

use climon2_vt::{Geometry, Limits, Model, Parser};

/// The default `memory.total_bytes`.
const BUDGET: usize = 512 << 20;

/// More than any of these models charges.
const ROOMY: Limits = Limits {
    bytes: 1 << 30,
    history_rows: 10_000,
};

/// What one model at a geometry charges, in bytes.
#[derive(Debug, PartialEq, Eq)]
struct Cost {
    cols: u16,
    rows: u16,
    /// Before its normal screen first scrolls.
    before: usize,
    /// Once it holds its first history chunk.
    after: usize,
    /// That model with a restoration staged beside it, which has scrolled
    /// too, as after a geometry change.
    staged: usize,
    /// With 10,000 history rows.
    full: usize,
}

fn line_feeds(count: usize) -> Vec<u8> {
    vec![b'\n'; count]
}

fn cost(cols: u16, rows: u16) -> Cost {
    let at = Geometry::for_host(cols, rows).unwrap();
    let screen = usize::from(rows);
    let mut parser = Parser::new();
    let mut model = Model::new(at, ROOMY).unwrap();
    let before = model.footprint();
    model.feed(&mut parser, &line_feeds(screen)).unwrap();
    let after = model.footprint();
    model.stage(at).unwrap();
    model.feed(&mut parser, &line_feeds(screen)).unwrap();
    let staged = model.footprint();
    let mut full = Model::new(at, ROOMY).unwrap();
    full.feed(&mut parser, &line_feeds(10_000 + screen - 1))
        .unwrap();
    assert_eq!(full.history().len(), 10_000);
    Cost {
        cols,
        rows,
        before,
        after,
        staged,
        full: full.footprint(),
    }
}

/// LIM-04 Contract and PERF-01 (sizing input): one model's charges at
/// 80x24, 200x60 and 500x200, before and after its first scroll, with a
/// scrolled restoration staged beside it and with 10,000 history rows, as
/// ADR 0007 records them; and how many models the default budget holds
/// beside one shared parser, before and after their first scroll.
#[test]
fn models_per_budget_are_those_adr_0007_records() {
    let measured: Vec<Cost> = [(80, 24), (200, 60), (500, 200)]
        .into_iter()
        .map(|(cols, rows)| cost(cols, rows))
        .collect();
    let fit = |model: usize| (BUDGET - Parser::BYTES) / model;
    let counts: Vec<(usize, usize)> = measured
        .iter()
        .map(|cost| (fit(cost.before), fit(cost.after)))
        .collect();
    println!("{measured:#?}\n{counts:?}");

    assert_eq!(measured, RECORDED);
    assert_eq!(counts, [(4206, 249), (870, 96), (110, 31)]);
}

/// ADR 0007's table, in bytes: 0.12 / 2.05 / 4.09 / 19.35 MiB at 80x24,
/// 0.59 / 5.26 / 10.52 / 47.28 MiB at 200x60 and 4.63 / 16.18 / 32.36 /
/// 120.00 MiB at 500x200.
const RECORDED: [Cost; 3] = [
    Cost {
        cols: 80,
        rows: 24,
        before: 127139,
        after: 2145443,
        staged: 4290886,
        full: 20289443,
    },
    Cost {
        cols: 200,
        rows: 60,
        before: 614267,
        after: 5516027,
        staged: 11032054,
        full: 49580027,
    },
    Cost {
        cols: 500,
        rows: 200,
        before: 4850887,
        after: 16966087,
        staged: 33932174,
        full: 125830087,
    },
];
