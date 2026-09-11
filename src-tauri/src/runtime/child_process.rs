// Shared low-level process-spawning utilities for anything that runs a plugin/module as its own
// child process (plugin_host::protocol::invoke, plugin_session::SessionRegistry, this crate's own
// process_module::ProcessModule) — command resolution, the hidden-window spawn boilerplate, and
// parsing a {"log":{"severity":..}} notification's severity string. One copy rather than three,
// and the Windows .exe fallback in particular has to apply everywhere: a module whose manifest
// omits the extension would otherwise fail to launch on Windows,
// where a plugin backend with the same manifest shape wouldn't.
//
// Lives under runtime/, not plugin_host/, on purpose — plugin_host already depends on runtime::
// runtime_loader for LogLevel/LogFn, so this follows that same existing direction rather than
// introducing a new one; the module and plugin systems otherwise stay deliberately independent of
// each other (see plugin_host's own module-level comment).

use crate::runtime::runtime_loader::LogLevel;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// Resolves `command` against `folder`: an exact local file first, then (Windows only) the same
/// name with `.exe` appended, then falls back to treating it as a bare command to resolve on
/// PATH — for a manifest that names a real system command (`"node"`, `"python"`) rather than a
/// file that ships alongside the plugin/module itself.
pub fn resolve_command(folder: &Path, command: &str) -> PathBuf {
    let local = folder.join(command);
    if local.is_file() {
        return local;
    }
    if cfg!(windows) {
        let with_exe = folder.join(format!("{command}.exe"));
        if with_exe.is_file() {
            return with_exe;
        }
    }
    PathBuf::from(command)
}

/// Spawns `exe` with `args`, working directory `folder`, stdin/stdout/stderr all piped: the one
/// shape every caller here needs, since each reads/writes those pipes itself. Hides the console
/// window Windows would otherwise pop up for a spawned CLI process (CREATE_NO_WINDOW); a no-op on
/// every other platform.
pub fn spawn_piped(exe: PathBuf, args: &[String], folder: &Path) -> std::io::Result<Child> {
    let mut cmd = Command::new(exe);
    cmd.args(args).current_dir(folder).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
}

/// Maps a `{"log":{"severity":"error"|"warn"|"warning"|...}}` notification's severity string to
/// this app's own LogLevel — case-insensitive, defaulting to Info for anything unrecognized
/// (including a plugin/module that never sends one at all).
pub fn parse_log_severity(severity: &str) -> LogLevel {
    match severity.to_ascii_lowercase().as_str() {
        "error" => LogLevel::Error,
        "warn" | "warning" => LogLevel::Warn,
        _ => LogLevel::Info,
    }
}

/// A spawned process's stderr goes straight into a log line/error message — capped so one runaway
/// process spamming stderr cannot flood the log with a single giant line. One shared constant even
/// though the two callers truncate slightly different things, one whole accumulated stderr buffer
/// for an error message and one line at a time as it streams in: same budget, same reasoning, worth
/// keeping in sync.
pub const STDERR_LOG_TRUNCATE_CHARS: usize = 300;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lowarc_studio_child_process_test_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_command_prefers_a_local_exact_match() {
        let dir = temp_dir("exact");
        std::fs::write(dir.join("mycmd"), b"").unwrap();
        assert_eq!(resolve_command(&dir, "mycmd"), dir.join("mycmd"));
    }

    #[test]
    #[cfg(windows)]
    fn resolve_command_falls_back_to_a_local_exe_on_windows() {
        let dir = temp_dir("exe_fallback");
        std::fs::write(dir.join("mycmd.exe"), b"").unwrap();
        assert_eq!(resolve_command(&dir, "mycmd"), dir.join("mycmd.exe"));
    }

    #[test]
    fn resolve_command_falls_back_to_bare_command_when_nothing_local_matches() {
        let dir = temp_dir("bare_fallback");
        assert_eq!(resolve_command(&dir, "node"), PathBuf::from("node"));
    }

    #[test]
    fn parse_log_severity_is_case_insensitive_and_defaults_to_info() {
        assert!(matches!(parse_log_severity("ERROR"), LogLevel::Error));
        assert!(matches!(parse_log_severity("Warning"), LogLevel::Warn));
        assert!(matches!(parse_log_severity("warn"), LogLevel::Warn));
        assert!(matches!(parse_log_severity("info"), LogLevel::Info));
        assert!(matches!(parse_log_severity("whatever"), LogLevel::Info));
    }
}
