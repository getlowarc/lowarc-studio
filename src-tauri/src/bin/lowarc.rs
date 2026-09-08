// The headless CLI — running a LowArc project without Studio.
//
// Until this existed, the only way to run a project was to click a button, which blocked three
// unrelated things that share one missing entry point: automated testing of a game, CI for projects
// built on LowArc, and a repro command you can paste into a bug report.
//
// Deliberately thin. runtime::start_run already loads the preset, resolves modules against the
// store, picks a loader and runs — Studio only writes a temporary launch.json because it drives the
// run in a SEPARATE process (bin/dev_run_host.rs) and has to hand the resolved set across a process
// boundary. This binary IS the process, so it skips all of that: no scratch folder, no temp file,
// nothing to clean up. What is left is argument parsing, a log callback, and one call.
//
// Distinct from the other two run entry points on purpose:
//   bin/lowarc_runtime.rs   the EXPORTED runtime — reads launch.json from its own directory, ships
//                           inside an exported game, knows nothing about a module store.
//   bin/dev_run_host.rs     Studio's child — same run, plus a control protocol on stdin/stdout for
//                           breakpoints, pause/step and stop.
//   this                    a terminal, a project directory, and no IDE.

use lowarc_studio_lib::app_paths::AppPaths;
use lowarc_studio_lib::runtime::{self, runtime_loader::DebugHooks, runtime_loader::LogFn};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const USAGE: &str = "\
lowarc — run a LowArc project without Studio

USAGE:
    lowarc run <project-dir> [--entry <path>] [--fps <n>]

ARGS:
    <project-dir>     A folder containing project.json

OPTIONS:
    --entry <path>    Entry file, relative to the project directory. Overrides project.json's own
                      `entry`. Required when it doesn't set one — a freshly created project doesn't.
    --fps <n>         Target frames per second (default: 60)
    -h, --help        Print this help
    -V, --version     Print the version

Exits 0 when the run ends cleanly, 1 otherwise.";

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

struct RunArgs {
    project: PathBuf,
    entry: Option<String>,
    fps: u32,
}

/// Hand-rolled rather than pulling in an argument-parsing crate — this is one subcommand and two
/// options, and the error messages matter more than the machinery.
fn parse_run_args(rest: &[String]) -> Result<RunArgs, String> {
    let mut project: Option<PathBuf> = None;
    let mut entry: Option<String> = None;
    let mut fps: u32 = 60;

    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--entry" => {
                let value = rest.get(i + 1).ok_or("--entry needs a path")?;
                entry = Some(value.clone());
                i += 2;
            }
            "--fps" => {
                let value = rest.get(i + 1).ok_or("--fps needs a number")?;
                fps = value.parse::<u32>().map_err(|_| format!("--fps expects a whole number, got \"{value}\""))?;
                if fps == 0 {
                    return Err("--fps must be at least 1".into());
                }
                i += 2;
            }
            other if other.starts_with('-') => return Err(format!("unknown option \"{other}\"")),
            other => {
                if project.is_some() {
                    return Err(format!("unexpected extra argument \"{other}\""));
                }
                project = Some(PathBuf::from(other));
                i += 1;
            }
        }
    }

    Ok(RunArgs { project: project.ok_or("a project directory is required")?, entry, fps })
}

/// project.json's own `entry` unless --entry overrode it. Errors rather than guessing when there is
/// neither: create_project writes a preset with no entry at all, and Studio fills it in through a
/// file picker that a terminal has no equivalent of.
fn resolve_entry(project: &Path, override_entry: Option<String>) -> Result<PathBuf, String> {
    let preset = runtime::project::ProjectPreset::load(project)?;
    let relative = match override_entry {
        Some(e) => e,
        None if !preset.entry.trim().is_empty() => preset.entry,
        None => {
            return Err(format!(
                "{} does not set an \"entry\", so there is nothing to run. Pass --entry <path>, or set one in Studio.",
                project.join("project.json").display()
            ))
        }
    };

    let entry = project.join(&relative);
    if !entry.is_file() {
        return Err(format!("entry file {} does not exist", entry.display()));
    }
    Ok(entry)
}

fn run(args: RunArgs) -> ! {
    let project = match args.project.canonicalize() {
        Ok(p) => p,
        Err(e) => fail(&format!("could not open {}: {e}", args.project.display())),
    };
    let entry = resolve_entry(&project, args.entry).unwrap_or_else(|e| fail(&e));

    // Same shape bin/lowarc_runtime.rs prints, so output reads identically whichever one produced
    // it. Everything goes to stdout; only a failure to run at all goes to stderr, so piping stdout
    // captures the run's own output without the tool's error reporting mixed in.
    let log: LogFn = Arc::new(|level, message| println!("[{level:?}] {message}"));

    let stop_flag = Arc::new(AtomicBool::new(false));
    // Ctrl+C sets the flag rather than killing the process, so the run ends through its normal path
    // and spawn_and_run still gets to send every module its `stop` phase. Without this a module
    // never cleans up — see this crate's Cargo.toml note on the dependency. A failure to install the
    // handler is not worth refusing to run over; it only costs a graceful Ctrl+C.
    let flag_for_handler = stop_flag.clone();
    if ctrlc::set_handler(move || flag_for_handler.store(true, Ordering::SeqCst)).is_err() {
        eprintln!("warning: could not install a Ctrl+C handler — interrupting will not stop modules cleanly");
    }

    // Per-module settings aren't sourced from anywhere yet, the same gap Studio's own dev-run has.
    // DebugHooks::disabled() because there is no debugger attached to a terminal — that is exactly
    // what dev_run_host exists for.
    let result = runtime::start_run(
        &entry,
        &project,
        &AppPaths::modules(),
        args.fps,
        serde_json::json!({}),
        stop_flag,
        log,
        DebugHooks::disabled(),
    );

    match result {
        Ok(()) => std::process::exit(0),
        Err(errors) => {
            for e in &errors {
                eprintln!("{e}");
            }
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        Some("run") => match parse_run_args(&args[1..]) {
            Ok(parsed) => run(parsed),
            Err(e) => fail(&format!("{e}\n\n{USAGE}")),
        },
        Some("-h") | Some("--help") | Some("help") => println!("{USAGE}"),
        Some("-V") | Some("--version") => println!("lowarc {}", env!("CARGO_PKG_VERSION")),
        Some(other) => fail(&format!("unknown command \"{other}\"\n\n{USAGE}")),
        None => fail(USAGE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_cli_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_a_project_directory_with_defaults() {
        let args = parse_run_args(&["some/project".to_string()]).expect("a bare project dir is valid");
        assert_eq!(args.project, PathBuf::from("some/project"));
        assert_eq!(args.fps, 60, "fps should default rather than being required");
        assert!(args.entry.is_none());
    }

    #[test]
    fn parses_options_in_any_position() {
        let args = parse_run_args(&["--fps".into(), "30".into(), "proj".into(), "--entry".into(), "src/main.txt".into()])
            .expect("options should not have to follow the positional argument");
        assert_eq!(args.project, PathBuf::from("proj"));
        assert_eq!(args.fps, 30);
        assert_eq!(args.entry.as_deref(), Some("src/main.txt"));
    }

    #[test]
    fn rejects_bad_input_with_a_reason_rather_than_a_default() {
        // Each of these silently defaulting would run something the caller did not ask for.
        assert!(parse_run_args(&[]).is_err(), "a missing project dir must not be silently accepted");
        assert!(parse_run_args(&["p".into(), "--fps".into(), "soon".into()]).is_err(), "a non-numeric fps must be rejected");
        assert!(parse_run_args(&["p".into(), "--fps".into(), "0".into()]).is_err(), "zero fps would never tick");
        assert!(parse_run_args(&["p".into(), "--entry".into()]).is_err(), "--entry without a value must be rejected");
        assert!(parse_run_args(&["p".into(), "--wat".into()]).is_err(), "an unknown option is a typo, not something to ignore");
        assert!(parse_run_args(&["a".into(), "b".into()]).is_err(), "two project dirs is ambiguous");
    }

    #[test]
    fn entry_comes_from_the_preset_unless_overridden() {
        let dir = temp_dir("entry");
        std::fs::write(dir.join("project.json"), r#"{"requires":[],"entry":"from-preset.txt"}"#).unwrap();
        std::fs::write(dir.join("from-preset.txt"), "x").unwrap();
        std::fs::write(dir.join("override.txt"), "x").unwrap();

        let used = resolve_entry(&dir, None).expect("the preset's own entry should be used");
        assert!(used.ends_with("from-preset.txt"));

        let overridden = resolve_entry(&dir, Some("override.txt".into())).expect("--entry should win");
        assert!(overridden.ends_with("override.txt"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_project_with_no_entry_says_so_instead_of_guessing() {
        // Exactly what create_project writes, and the reason --entry exists: Studio fills this in
        // with a file picker, which a terminal has no equivalent of.
        let dir = temp_dir("no_entry");
        std::fs::write(dir.join("project.json"), r#"{"requires":[]}"#).unwrap();

        let err = resolve_entry(&dir, None).expect_err("an unset entry must be a clear error");
        assert!(err.contains("--entry"), "the error should say how to fix it: {err}");

        let missing = resolve_entry(&dir, Some("nope.txt".into())).expect_err("a named entry that isn't there must fail");
        assert!(missing.contains("does not exist"), "{missing}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
