# climon2-measure

The test-only PERF-01 measurement harness
([#121](https://github.com/jackgeek/climon2/issues/121)), never shipped. It
measures how the monitor uses resources on the **exact candidate archive**,
against standalone clients doing equivalent work, and records each platform's
qualified library requirements. See the acceptance matrix's
[PERF-01](../../docs/monitor-acceptance.md) workload.

```sh
cargo xtask dist
cargo xtask measure --archive target/dist/climon2-0.1.0-linux-x86_64-gnu.tar.gz \
  --out target/measure/local --geometry 80x24
```

`cargo xtask measure` builds this package in release mode and runs
`climon2-measure run` with the same flags:

| Flag | Meaning |
|---|---|
| `--archive <file>` | The exact `.tar.gz` release archive for this host's platform. It is installed by its own `install --dir`; nothing is rebuilt or substituted. |
| `--out <dir>` | Where `report.json` (every sample) and `report.md` (the summary) are written, after every case. |
| `--seconds <1..=600>` | Each scenario's window, 60 by default: the owner's decision. **A shorter window is for local proof only**; the report's environment then says `REDUCED ... not qualification evidence`. |
| `--geometry <80x24\|200x60\|500x200>` | Only these geometries (repeatable; default all three). The workflow runs one per job. |
| `--only <text>` | Only cases whose ID contains the text, such as `/10/one-release/`. |

Its exit status is 0 when every selected case ran, **whether or not targets were
met**: a missed target is a failed Measure case in the report, which needs the
owner's written disposition, and #122 blocks qualification until then (owner,
2026-11-16). A harness error (a client that never starts, typed input that
never echoes during setup, an archive that does not install) leaves that case
`not run` with the error and makes the exit status 1.

## Where it runs

The [Measure workflow](../../.github/workflows/measure.yml) runs on hosted
`macos-15`, `macos-15-intel` and `ubuntu-24.04`, one job per runner and
geometry, on release candidates (the Release workflow calls it for `-rc.N`
tags, without gating publication) and on manual dispatch, which builds the
candidate artifacts itself. It never runs on pull requests. Each job uploads
`measure-<runner>-<geometry>` and appends its `report.md` to the job summary.

## The matrix

Cases (`matrix.rs`) are `PERF-01/<geometry>/<sessions>/<releases>/<scenario>`:
geometries 80x24, 200x60 and 500x200; 1, 10 and 100 sessions, except that at
500x200 the largest case offers 100 and records the admitted count as the
measured capacity (owner, 2026-11-15, #166); one release and ten mixed
releases; eight scenarios. Each case runs:

1. one `climon2 monitor` at the default limits;
2. if the defaults admit fewer sessions than an exact population offers, the
   monitor again with the disclosed sufficient configuration
   `{"memory":{"total_bytes":2147483648}}` (the record keeps both);
3. the owner's baseline: one `climon2 attach` per session, each in its own
   terminal of the same geometry.

Sessions run this package's `workload` command: titled `s001`..., reporting
OSC 7501 `working`, writing lowercase lines at the scenario's fixed rate (720
B/s per unselected session for background output, 262144 B/s from the selected
`s001` for sustained output, otherwise silent), discarding typed input. Typed
probe tokens are uppercase, so only the terminal's echo can show them.

| Scenario | What happens in the window (once a second unless stated) |
|---|---|
| passive-idle | Nothing; only sampling. |
| background-output | Input probe on `s001`; selector and Escape every 5 s; every 5 s the next unselected session that has not yet done so reports attention (bell, toast text shown, toast gone), then works again: a monitor toasts each attention episode once. |
| cold-warm-loading | The client(s) start, until every session is connected, then exit (prefix+x) or detach, repeatedly; the first start is cold. |
| restoration-switching | Select the next session through the selector's search and Enter, until its chrome title and restored start line show; selector and Escape every 5 s. |
| selected-sustained-output | Selector and Escape; at the start a heuristic session `h001` arrives, titled, and falls silent, until its attention toast shows (STA-02). Typed input is not probed: its echo scrolls away between frames. |
| reconnect-storm | Every 15 s from the fifth: every daemon stopped for 3 s (selector probed meanwhile), then resumed, until typed input echoes (recovery); input probes otherwise. |
| continuous-arrivals | Every 2 s a new session `aNNN`, until a client is connected to it (standalone: a new attach for it); input probes. |
| stalled-newer-starts | A tenth more newer sessions (at least one) are stopped from the start; input probes; selector every 5 s. |

Ten mixed releases are recorded `not run`: no ten-release mixed-version
fixture exists yet (owner question on #121).

## What it records

`report.md` declares the environment (OS, architecture, hardware, runner,
candidate archive and its SHA-256, release, configuration and descriptor
limit, workload and rates, window, sampling and latency methods), then the
failed Measure cases with each finding, the qualified requirements, and one row
per run. `report.json` keeps every latency sample.

- **Processor and memory.** Every 250 ms (`sampler.rs`): processor time,
  resident memory, threads and descriptors of the client process trees (the
  monitor, or every attach client) and, separately, of every session daemon;
  procfs on Linux, `proc_pidinfo` on macOS. Usage is processor time over wall
  time, and summed resident memory's mean and maximum.
- **Attributable components.** On Linux, the monitor's resident memory by
  mapping at the end of the window: its image, client library images, other
  libraries, anonymous memory (heap and stacks) and other files, and the
  number of client library files mapped (retained releases).
- **Admission and backlog.** A session counts as connected while its daemon
  holds more descriptors than with no client; the admitted count once no
  further client connects for 5 s, and the most sessions still waiting.
- **Latency** (`evaluate.rs`), from the probe's keys or trigger to the first
  host bytes or screen state that show the result on an emulated host terminal
  (`climon2-test-term`): input (echo round trip, an upper bound on dispatch),
  selector, global action (Escape closing it), attention (until the toast's
  host bell), exit start (prefix+x until the host leaves the alternate
  screen), switch, readiness, arrival, recovery, toast text and toast gone
  (NOT-02), and the status sampler (STA-02). Input, selector, global action
  and attention are held to 100 ms at the 99th percentile; any sample of
  250 ms or more, and any try that never completed within 5 s, is a finding
  for diagnosis.
- **Display.** Bytes the host terminals received over the window.
- **Qualified requirements.** Before the cases, the monitor's descriptors and
  threads with 0, 1 and 11 connected sessions give `workers`,
  `instance_descriptors` and `load_descriptors` per session, checked against
  the values embedded in the candidate's library description (read through
  the shared loader) with 25% headroom: measured × 1.25 ≤ embedded (owner,
  2026-11-16). The requirements stay in source; a miss reports the least value
  to embed. Allocation and stack sizes cannot be observed outside the monitor
  process and are reported as not measured.

## Scenario-to-test map

| Case | Test |
|---|---|
| PERF-01 matrix: geometries, populations, capacity at 500x200, releases, scenarios, rates | `matrix::tests::the_matrix_runs_every_evidence_point_and_capacity_at_the_boundary`, `a_geometry_selects_only_its_own_cases`, `output_scenarios_write_at_their_fixed_rates_and_the_rest_are_silent` |
| PERF-01 targets: 100 ms p99 at the limit and one past; stalls of 250 ms; timeouts | `evaluate::tests::a_targeted_probe_misses_only_above_100_ms_at_the_99th_percentile`, `any_stall_of_250_ms_or_more_needs_diagnosis`, `tries_that_never_completed_are_reported`, `a_distribution_ranks_its_samples_by_nearest_rank` |
| PERF-01 qualified requirements: 25% headroom at the limit and one past | `evaluate::tests::a_requirement_needs_25_percent_headroom_below_the_embedded_value` |
| PERF-01 Measure case record: a miss is a failed case for disposition; defaults that cannot admit | `report::tests::a_missed_target_fails_the_case_and_standalone_runs_set_none`, `defaults_admitting_fewer_than_offered_are_a_finding_for_exact_populations_only`, `the_report_lists_failed_cases_for_disposition_and_every_run` |
| PERF-01 sampling and attributable components | `sample::tests::usage_sums_processor_time_and_each_ticks_resident_memory`, `a_stat_line_yields_processor_ticks_threads_and_resident_pages`, `smaps_attributes_resident_memory_to_its_components`, `the_running_process_samples_itself` |
| PERF-01 workload rates; NOT-02 and STA-02 triggers | `workload::tests::*` |
| Command line | `args::tests::*`; xtask `measure::tests::measure_runs_the_release_harness_with_the_given_flags` |
| Workflow placement (release candidates and dispatch, three runners, not on pull requests, not gating publication) | `xtask/tests/workflows.rs` |

The measurements themselves (NOT-02, STA-02, LIM-06 and PERF-01 Measure
cases) are the workflow's reports on the candidate artifacts, not workspace
tests.
