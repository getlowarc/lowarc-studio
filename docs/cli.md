# `lowarc`: the command line

Runs a LowArc project without Studio.

```
lowarc run <project-dir> [--entry <path>] [--fps <n>]
lowarc --help | --version
```

Until this existed the only way to run a project was to click a button in the IDE, which blocked
three unrelated things that share one missing entry point: automated testing of a game, CI for
projects built on LowArc, and a repro command you can paste into a bug report.

## Options

| | |
| --- | --- |
| `--entry <path>` | Entry file, relative to the project directory. Overrides `project.json`'s own `entry`. |
| `--fps <n>` | Target frames per second. Default 60. |

`--entry` exists because a freshly created project has no entry at all — `create_project` writes
`{"requires":[]}`, and Studio fills the rest in through a file picker that a terminal has no
equivalent of. Without the flag such a project would be unrunnable from the command line.

`--fps` is deliberately **not** read from Studio's `devRunTargetFps` setting. That is a preference
for the IDE's dev-run panel, and a headless tool that silently changed behaviour based on it would
be surprising in CI.

## Exit codes

`0` when the run ends cleanly, `1` otherwise: a missing module, an unreadable entry file, a module
that failed to start. This is the part CI depends on, so it is covered by tests rather than assumed.

## What a run prints

Log lines from the modules themselves, in the same `[Info] message` form the exported runtime uses.
**A healthy run is often silent** — modules only log when they have something to say, so no output
is normal rather than a sign that nothing happened.

## How a run ends

Nothing ends a run on a timer. It ends when:

- a module asks to stop (`{"requestStop": true}`) — closing the `vector-canvas` window does this
- **Ctrl+C**, which sets the same stop flag rather than killing the process, so every module still
  gets its `stop` phase and can clean up

Ctrl+C was verified by delivering a real `CTRL_C_EVENT` to a running `lowarc run` with a
`vector-canvas` project: the CLI exits **0** — the graceful path, not the `0xC000013A` a hard
console termination produces, and the canvas child exits with it rather than being orphaned.

It is checked by hand rather than in CI on purpose. Doing it requires attaching to another
process's console (`AttachConsole` + `GenerateConsoleCtrlEvent`), which is Windows-specific and
sensitive to the console the *test runner itself* happens to have: the same test reports a false
failure when its output is piped. A test that fails for reasons unrelated to the code is worse than
no test.

A project with no module that ever asks to stop will run until you interrupt it. That's correct
behaviour, not a hang: a game loop has no natural end.

Note that **hard-killing the process** (`taskkill /F`, a `timeout` wrapper) tears down modules
mid-request, and the run reports `module did not respond in time` on its way out. That message at
the very end of an otherwise clean run means the parent was killed, not that anything was wrong.

## Which modules it runs

The same store Studio uses: the repo's own `/modules/` in a source checkout,
`%APPDATA%\LowArcStudio\modules` in an installed copy. `LOWARC_MODULES_DIR` overrides it, which is
useful for running a project against a different module set without disturbing the installed one.

## Relationship to the other run entry points

Three binaries can run a project, deliberately kept separate:

| | |
| --- | --- |
| `lowarc` | this: a terminal, a project directory, no IDE |
| `dev_run_host` | Studio's child process: the same run plus a control protocol for breakpoints, pause/step and stop |
| `lowarc_runtime` | the exported runtime, shipped inside an exported game; reads `launch.json` from its own directory and knows nothing about a module store |

`lowarc` reuses `runtime::start_run`, which resolves and runs in one call. Studio needs a temporary
`launch.json` only because it drives the run in a *separate* process and has to hand the resolved
module set across a process boundary; the CLI is that process already, so it skips all of it.

## Not built yet

- **Distribution.** The binary currently lives in `target/`. It is not on `PATH`, not bundled, and
  not published. See `docs/updating.md` for the release story it would eventually ride along with.
  Options are a separate release artifact or an "install command line tools" action in Studio, the
  way VS Code installs `code`. Undecided.
- **Per-module settings.** `lowarc` passes an empty settings object, the same gap Studio's own
  dev-run has: nothing sources them from a project yet. This is why a canvas run currently uses its
  built-in defaults regardless of the project.
- `lowarc check` (resolve without running) and `lowarc modules` (list what's installed). Both cheap
  to add; neither needed yet.
