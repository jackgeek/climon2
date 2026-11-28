#!/usr/bin/env bash
# Whether one run of check.yml runs its macOS and Windows jobs
# (docs/for-humans/quality-runners.md, "The monitor links feature's
# branches"). Prints `macos=` and `windows=` lines for $GITHUB_OUTPUT, each
# one of:
#
#   switch  the repository variables decide, as for any other run
#   on      (windows only) every Windows job runs, whatever they say
#   off     the platform's jobs are skipped
#
#   EVENT     the run's event (github.event_name)
#   BASE_REF  a pull request's base branch (github.base_ref), or empty
#   HEAD_REF  a pull request's head branch (github.head_ref), or empty
#   BASE      a pull request's base commit (github.event.pull_request.base.sha)
set -euo pipefail

macos=switch
windows=switch
# A pull request into the monitor links feature branch (#488), or into one of
# its ticket branches, skips macOS (owner decision, 2026-11-27). It runs every
# Windows job from a Stack W ticket branch, monitor-links/win-*, or when it
# changes Windows-only code, and none otherwise (owner decision, 2026-11-28).
if [[ "$EVENT" == pull_request \
  && ("$BASE_REF" == feature/monitor-links || "$BASE_REF" == monitor-links/*) ]]; then
  macos=off
  windows=off
  if [[ "$HEAD_REF" == monitor-links/win-* ]]; then
    windows=on
  else
    changed="$(cargo xtask windows-changed "$BASE")"
    if [[ "$changed" == true ]]; then windows=on; fi
  fi
fi
echo "macos=$macos"
echo "windows=$windows"
