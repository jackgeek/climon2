#!/usr/bin/env bash
# Which own-hardware gates one run of quality.yml has, and the commit they
# mutate against (docs/for-humans/quality-runners.md). Prints `sha=` and
# `gates=` lines for $GITHUB_OUTPUT.
#
#   REF     the run's ref (github.ref)
#   EVENT   the run's event (github.event_name)
#   BEFORE  the commit a push replaced (github.event.before), or empty
set -euo pipefail

gates=linux,windows,macos
against=origin/main
# The monitor links feature (#488) lands on one feature branch through ticket
# branches named monitor-links/*. A ticket branch is gated on Linux alone,
# against the feature branch; the feature branch on every platform, against
# main. Neither is gated on a push: one manual run gates a whole stack of
# ticket branches from its top (owner decisions, 2026-11-27).
case "$REF" in
  refs/heads/monitor-links/*)
    gates=linux
    against=origin/feature/monitor-links
    ;;
esac
case "$REF" in
  refs/heads/monitor-links/* | refs/heads/feature/monitor-links)
    if [[ "$EVENT" != workflow_dispatch ]]; then gates=; fi
    ;;
esac

# main mutates what this push changed; any other branch what it changes
# against the branch it will merge into, as a pull request's gate did.
if [[ "$REF" == refs/heads/main ]]; then
  base="$BEFORE"
  if [[ -z "$base" || "$base" == 0000000000000000000000000000000000000000 ]]; then
    base=HEAD~1
  fi
else
  base="$(git merge-base "$against" HEAD)"
fi
echo "sha=$(git rev-parse "$base")"
echo "gates=$gates"
