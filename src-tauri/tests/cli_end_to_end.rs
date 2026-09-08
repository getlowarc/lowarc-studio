// Drives the real `lowarc` binary the way a person or a CI job would — a project directory, an
// argument list, and whatever it prints and exits with.
//
// Exit codes are the point. A CLI that runs correctly but always exits 0 is useless for CI, and
// that failure is invisible to anyone reading its output, so every test here asserts on status as
// well as on what was printed.
//
// These build their own module store rather than using the machine's: AppPaths::modules() resolves
// to the repo's own /modules/ in a source checkout, which is developer state that varies between
// machines and would make these tests depend on it. LOWARC_MODULES_DIR overrides it — see
// AppPaths::modules.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_cli_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cli_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lowarc"))
}

/// A module that logs once per frame and asks to stop on its third, so a run ends on its own rather
/// than needing to be killed — the CLI has no external stop the way Studio does.
///
/// timeoutMs is generous on purpose: the 10s default is sized for a real module, not for starting a
/// PowerShell interpreter on a contended CI runner, which has already cost this repo a red build.
fn write_self_stopping_module(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"ticker","name":"Ticker","loadOrder":1,"requires":[]}"#).unwrap();
    std::fs::write(
        dir.join("process.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","module.ps1"],"wantsFrames":true,"timeoutMs":30000}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("module.ps1"),
        r#"
$frames = 0
while ($line = [Console]::In.ReadLine()) {
    if ([string]::IsNullOrWhiteSpace($line)) { continue }
    $msg = $line | ConvertFrom-Json
    if ($msg.phase -eq "frame") {
        $frames++
        [Console]::Out.WriteLine((@{ log = @{ severity = "info"; message = "tick $frames" } } | ConvertTo-Json -Compress))
        if ($frames -ge 3) {
            [Console]::Out.WriteLine((@{ requestStop = $true } | ConvertTo-Json -Compress))
        }
    }
    [Console]::Out.WriteLine((@{ ok = $true } | ConvertTo-Json -Compress)); [Console]::Out.Flush()
    if ($msg.phase -eq "stop") { break }
}
"#,
    )
    .unwrap();
}

/// Returns (project_dir, modules_dir).
fn write_project(name: &str, requires: &str, entry: Option<&str>) -> (PathBuf, PathBuf) {
    let root = temp_dir(name);
    let modules = root.join("modules");
    std::fs::create_dir_all(&modules).unwrap();
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();

    let entry_field = entry.map(|e| format!(r#","entry":"{e}""#)).unwrap_or_default();
    std::fs::write(project.join("project.json"), format!(r#"{{"requires":[{requires}]{entry_field}}}"#)).unwrap();
    std::fs::write(project.join("main.txt"), "entry file contents are never parsed by Studio").unwrap();
    (project, modules)
}

fn run_cli(project: &Path, modules: &Path, extra: &[&str]) -> Output {
    let mut cmd = Command::new(cli_bin());
    cmd.arg("run").arg(project).env("LOWARC_MODULES_DIR", modules);
    for a in extra {
        cmd.arg(a);
    }
    cmd.output().expect("the lowarc binary should be runnable")
}

#[test]
fn a_project_runs_to_a_clean_exit_and_prints_its_modules_output() {
    let (project, modules) = write_project("ok", r#"{"id":"ticker","version":"*"}"#, Some("main.txt"));
    write_self_stopping_module(&modules.join("ticker"));

    let out = run_cli(&project, &modules, &[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(out.status.success(), "a run that ends normally must exit 0.\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("tick 1"), "the module's own log lines should reach stdout:\n{stdout}");
}

#[test]
fn a_missing_module_fails_loudly_and_names_it() {
    // The single most likely first-run failure, and the one where a silent exit 0 would be worst:
    // CI would go green having run nothing at all.
    let (project, modules) = write_project("missing", r#"{"id":"not-installed","version":"*"}"#, Some("main.txt"));

    let out = run_cli(&project, &modules, &[]);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(!out.status.success(), "an unresolvable project must not exit 0");
    assert!(stderr.contains("not-installed"), "the error should name the module that is missing:\n{stderr}");
}

#[test]
fn entry_can_be_supplied_for_a_project_that_sets_none() {
    // What `create_project` writes: requires and nothing else. Studio fills the entry in through a
    // file picker, so without --entry this project would be unrunnable from a terminal.
    let (project, modules) = write_project("no_entry", r#"{"id":"ticker","version":"*"}"#, None);
    write_self_stopping_module(&modules.join("ticker"));

    let without = run_cli(&project, &modules, &[]);
    assert!(!without.status.success(), "an unset entry must be an error, not a guess");
    let stderr = String::from_utf8_lossy(&without.stderr);
    assert!(stderr.contains("--entry"), "the error should point at the fix:\n{stderr}");

    let with = run_cli(&project, &modules, &["--entry", "main.txt"]);
    assert!(
        with.status.success(),
        "--entry should make the same project runnable.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&with.stdout),
        String::from_utf8_lossy(&with.stderr)
    );
}

#[test]
fn help_and_version_work_without_a_project() {
    let help = Command::new(cli_bin()).arg("--help").output().unwrap();
    assert!(help.status.success(), "--help should exit 0");
    let help_text = String::from_utf8_lossy(&help.stdout);
    assert!(help_text.contains("lowarc run"), "help should show the run command:\n{help_text}");

    let version = Command::new(cli_bin()).arg("--version").output().unwrap();
    assert!(version.status.success(), "--version should exit 0");
    assert!(String::from_utf8_lossy(&version.stdout).contains(env!("CARGO_PKG_VERSION")));

    // A bare invocation is a usage error, not a silent success — otherwise `lowarc` alone in a
    // script looks like it did something.
    let bare = Command::new(cli_bin()).output().unwrap();
    assert!(!bare.status.success(), "no arguments should be a usage error");
}
