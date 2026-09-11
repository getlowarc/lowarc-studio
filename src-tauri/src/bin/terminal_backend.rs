// The Terminal plugin's backend — unlike every other plugin backend (see file_explorer_backend.rs
// for the invoke-per-call reference), this one is a *session*-mode plugin (plugin.json's
// "session": true; see plugin_host/protocol.rs and plugin_session.rs). It's spawned once and
// stays running for as long as the terminal panel is open, not spawned fresh per call: a shell
// genuinely needs persistent state (cwd, environment, a running foreground process) that a
// one-shot process has nowhere to keep.
//
// Cross-platform by construction: `portable-pty` (the crate wezterm itself is built on) wraps
// ConPTY on Windows and a real POSIX pty on Linux/macOS behind one uniform API: nothing in this
// file is Windows-only or Unix-only, and nothing here should ever need a #[cfg(windows)]/
// #[cfg(unix)] branch. The one shell-selection concern (Windows PowerShell vs pwsh vs bash/zsh) is
// resolved by the *caller* (plugin_session.rs), not here: this binary just execs whatever command
// string it's given as its first argument.
//
// Wire protocol — JSON lines, both directions, for as long as the process lives (not one-shot):
//   host -> this (stdin):
//     {"type":"input","data":"..."}              raw text to write to the pty
//     {"type":"resize","cols":N,"rows":N}         inform the pty of a new terminal size
//   this -> host (stdout):
//     {"type":"output","data":"..."}              text the shell produced
//     {"type":"exit","code":N|null}                the shell process exited; this process follows

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};

fn main() {
    // argv[1] is the shell command to run (e.g. "powershell", "pwsh", "bash", "zsh") — chosen by
    // plugin_session.rs from Settings, with a platform-appropriate fallback. Nothing in this
    // binary hardcodes a specific shell.
    let shell = std::env::args().nth(1).unwrap_or_else(default_shell);

    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 }) {
        Ok(p) => p,
        Err(e) => {
            println!("{}", json!({"type": "exit", "code": Value::Null, "error": format!("could not open a pty: {e}")}));
            return;
        }
    };

    let mut child = match pair.slave.spawn_command(CommandBuilder::new(&shell)) {
        Ok(c) => c,
        Err(e) => {
            println!("{}", json!({"type": "exit", "code": Value::Null, "error": format!("could not start \"{shell}\": {e}")}));
            return;
        }
    };
    // Must drop our copy of the slave end once the child owns it — on Unix in particular, the
    // master's reader never sees EOF after the child exits as long as any other process (this one
    // included) still holds the slave fd open.
    drop(pair.slave);

    let mut writer = pair.master.take_writer().expect("pty master writer");
    let mut reader = pair.master.try_clone_reader().expect("pty master reader");

    // Forwards shell output as lines of JSON, not lines of terminal output: a read often lands
    // mid-escape-sequence, which is fine, since xterm.js parses a continuous byte stream. EOF is
    // the one reliable "the shell exited" signal, and it cannot come from the stdin loop below,
    // which blocks waiting on the host while the shell can exit on its own. Exits the whole process
    // rather than this thread, which also unblocks the main thread's stdin read.
    std::thread::spawn(move || {
        let mut stdout = std::io::stdout();
        let mut buf = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    let line = json!({"type": "output", "data": text}).to_string();
                    if writeln!(stdout, "{line}").is_err() || stdout.flush().is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let code = child.wait().ok().map(|status| status.exit_code());
        let _ = writeln!(stdout, "{}", json!({"type": "exit", "code": code}));
        let _ = stdout.flush();
        std::process::exit(0);
    });

    // Main thread: read host commands from our own stdin for as long as the process lives — this
    // is the one loop that makes this a *session* rather than a one-shot invocation. Ends when the
    // host closes its end (app quit / plugin_session.rs stopping this session) or the thread above
    // exits the process out from under it.
    let stdin = std::io::stdin();
    for line in BufReader::new(stdin.lock()).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
        match message.get("type").and_then(|t| t.as_str()) {
            Some("input") => {
                if let Some(data) = message.get("data").and_then(|d| d.as_str()) {
                    if writer.write_all(data.as_bytes()).is_err() {
                        break;
                    }
                }
            }
            Some("resize") => {
                let cols = message.get("cols").and_then(|c| c.as_u64()).unwrap_or(80) as u16;
                let rows = message.get("rows").and_then(|r| r.as_u64()).unwrap_or(24) as u16;
                let _ = pair.master.resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 });
            }
            _ => {}
        }
    }
}

fn default_shell() -> String {
    if cfg!(windows) {
        "powershell".to_string()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "bash".to_string())
    }
}
