# Roadmap

Features in delivery order. Each feature's acceptance tests remain permanent regression gates once delivered.

| # | Feature | Summary | Depends on |
|---|---|---|---|
| 1 | `session-exec-discovery` | `climon2 exec <cmd>` starts a session via a detached session daemon; `climon2 list` shows live sessions; full install layout (launcher, `current`, versioned runtime) | — |
| 2 | `session-detach-survival` | Sessions survive client disconnect; end only on command exit or termination request (amends feature 1) | 1 |
| 3 | `session-attach` | A client attaches to an existing session; the previously controlling client is displaced with a reclaim notice | 2 |
| 4 | `session-reclaim` | A displaced client presses space to reclaim control immediately | 3 |
| 5 | `screen-restore` | Attaching or reclaiming restores the session's current screen | 3, 4 |
| 6 | `install-upgrade` | Install a new runtime version side by side, switch `current`, `climon2 cleanup` of unused versions; cross-version tests begin | 1 |
