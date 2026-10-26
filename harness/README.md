# Acceptance harness

Black-box acceptance tests for climon2 (`features/<id>/tests/`) use this harness. It is owned by the factory: agents can read it but not change it. The repository's root `conftest.py` loads its pytest fixtures for every test, on every gate (Linux container, Windows CI, native macOS).

Tests run from the repository root with `CLIMON2_BIN_DIR` pointing at the release build of `climon2/` (`climon2`, `climon2-cli`, `climon2-daemon`, `climon2-client`, with `.exe` on Windows).

## Fixtures

| Fixture | What it gives you |
|---|---|
| `climon2` | An `Install`: a fresh install layout in a temporary path containing a space and non-ASCII characters. `.root`, `.launcher` (`climon2[.exe]`), `.current` (the `current` file), `.version` (default `0.1.0`), `.version_dir`, `.companion("climon2-daemon")`. |
| `climon2_home` | An empty climon2 home directory for the test. |
| `base_env` | A copy of the environment with inherited `CLIMON2_*` variables removed, the install first on `PATH`, and `CLIMON2_HOME` set to `climon2_home`. Copy it before changing it. |
| `terminal` | `terminal(argv, env=None, cwd=None, cols=80, rows=24) -> Terminal`: starts `argv` in a real terminal (a PTY that is the controlling terminal on Unix, ConPTY on Windows). Each one is closed after the test, killing its process if it is still running. |
| `fixture_program` | The argv prefix (`[python, fixture_program.py]`) for the generic terminal fixture program. |
| `bin_dir` | `Path` of `$CLIMON2_BIN_DIR`. Fails the test if it is not set. |

## `Terminal`

- `send(text)` types text. `send_line(text)` types text and Enter (`\r`).
- `expect(regex, timeout=DEFAULT_TIMEOUT) -> re.Match` waits for the pattern in the plain-text output after the previous match, and raises `TerminalTimeout` (an `AssertionError`) with the unmatched output if it doesn't appear. Remember that the terminal echoes what you type: expect output that differs from the typed text. String patterns are multiline, so `^` and `$` match at line boundaries, including where the previous match ended. After sending Ctrl+C (`\x03`), expect the program's response (such as a new prompt) before typing more: the terminal discards typed-ahead input on an interrupt.
- `text` is all output as plain text (see `clean`); `output` is raw, including escape sequences.
- `resize(cols, rows)`, `alive`, `pid`.
- `wait(timeout=DEFAULT_TIMEOUT) -> int` waits for the program to exit and returns its exit code (a negative signal number on Unix if it was killed by a signal).
- `close()` kills the program if it is still running.

`DEFAULT_TIMEOUT` (15 s) is the approved deadline for any single wait. Never sleep and hope; wait on something observable.

## The generic terminal fixture program

`fixture_program.py [--marker TEXT] [args…]` prints `FIXTURE READY <marker> <json>` on start, where the JSON has `pid`, `tty` (whether stdin, stdout and stderr are terminals) and `size` (`<cols>x<rows>`). It then answers one line per command:

| Command | Answer |
|---|---|
| `size` | `SIZE <cols>x<rows>` |
| `tty` | `TTY {"stdin": true, "stdout": true, "stderr": true}` |
| `ctty` | `CTTY true` if the controlling terminal (`/dev/tty`, `CONOUT$`) opens |
| `argv` | `ARGV <json list of the extra args>` |
| `cwd` | `CWD <json path>` |
| `env NAME` | `ENV {"NAME": value or null}` |
| `echo TEXT` | `ECHO <json TEXT>` |
| `exit N` | exits with code N |

All values are JSON with ASCII escapes, so answers are identical on every platform. Run it inside a session (for example `climon2 exec <python> <fixture_program.py> --marker m1 "a b"`) to prove arguments, environment, working directory, terminal size and exit codes pass through.

## Shells and other helpers (`from harness import …`)

- `required_shells()`: this platform's required shells (`REQUIRED_SHELLS`): Windows `powershell.exe`, `pwsh.exe`, `cmd.exe`; Linux `bash`, `sh`; macOS `zsh`, `bash`, `sh`. Parametrize shell tests over it.
- `controlled_shell(name) -> argv`: starts the shell interactively without startup files. A missing shell fails the test; it is never skipped or substituted.
- `controlled_env(name, env) -> env`: removes `ENV`/`BASH_ENV` and sets an empty `ZDOTDIR` for zsh.
- `install_layout(bin_dir, root, version="0.1.0") -> Install` builds another install layout (for example a second version).
- `wait_until(condition, message, timeout=DEFAULT_TIMEOUT)` polls until `condition()` is truthy.
- `pid_alive(pid)` checks whether a process exists. Use it to prove processes (such as session daemons) don't leak.
- `clean(raw)` turns terminal output into plain text.

## Rules

- Test through climon2's public interface only: its commands, output, exit codes, files and processes.
- Isolate every test: use `climon2_home` and `base_env`, and never touch the real `~/.climon2`.
- Clean up everything a test starts. A leaked process fails the test.
- Tests are deterministic. A test that passes only on rerun counts as failed.
