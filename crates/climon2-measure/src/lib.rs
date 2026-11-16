//! The PERF-01 measurement harness (#121): the workload matrix, the
//! evaluation of its measurements against the declared targets and the
//! embedded qualified requirements, and the Measure-case report. Everything
//! here but the process driving is pure.

pub mod args;
pub mod evaluate;
pub mod harness;
pub mod matrix;
pub mod report;
pub mod sample;
pub mod workload;
