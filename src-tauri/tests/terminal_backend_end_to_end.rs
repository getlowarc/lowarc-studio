// Drives the real terminal backend over its real wire protocol, with a real shell behind a real
// pty. Nothing else in the suite reaches ConPTY: the plugin-host tests cover the process plumbing
// around this binary, not whether a shell actually starts, echoes, resizes or exits through it.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const ESC: char = '\u{1b}';

struct Backend {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
}

impl Backend {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_terminal_backend"))
            .arg("powershell")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("terminal_backend should start");

        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, messages) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    let _ = tx.send(v);
                }
            }
        });

        Self { child, stdin, messages }
    }

    fn send(&mut self, message: Value) {
        let _ = writeln!(self.stdin, "{message}");
        let _ = self.stdin.flush();
    }

    fn input(&mut self, text: &str) {
        self.send(json!({"type": "input", "data": text}));
    }

    /// Collects output until `predicate` is happy or the time runs out, answering ConPTY's
    /// cursor-position request along the way. That request (ESC[6n) BLOCKS the shell until a
    /// terminal answers it; xterm.js does so on its own, so a test standing in for xterm.js has to
    /// as well, or no shell ever reaches a prompt.
    fn pump(&mut self, limit: Duration, mut predicate: impl FnMut(&str, &[Value]) -> bool) -> (String, Vec<Value>) {
        let deadline = Instant::now() + limit;
        let mut text = String::new();
        let mut others = Vec::new();
        while Instant::now() < deadline {
            let Ok(msg) = self.messages.recv_timeout(Duration::from_millis(200)) else {
                if predicate(&text, &others) {
                    break;
                }
                continue;
            };
            match msg.get("type").and_then(Value::as_str) {
                Some("output") => {
                    let data = msg.get("data").and_then(Value::as_str).unwrap_or("");
                    text.push_str(data);
                    if data.contains(&format!("{ESC}[6n")) {
                        self.input(&format!("{ESC}[1;1R"));
                    }
                }
                _ => others.push(msg),
            }
            if predicate(&text, &others) {
                break;
            }
        }
        (text, others)
    }

    /// Asks the shell for the value of a PowerShell expression, wrapped in angle markers.
    ///
    /// The wrapper is what separates the answer from the shell echoing the command back. Both
    /// travel down the same stream and the echo arrives FIRST, so a test looking for a bare "120"
    /// finds it inside the echo, or inside an escape sequence like ESC[8;40;120t, and stops before
    /// the real answer exists. The echo always contains the unexpanded "$(", the output never does.
    fn value_of(&mut self, expression: &str) -> String {
        self.input(&format!("Write-Output \"<<$({expression})>>\"\r"));
        let (text, _) = self.pump(Duration::from_secs(20), |t, _| answer_in(t).is_some());
        answer_in(&text).unwrap_or_else(|| format!("(no answer; saw {text:?})"))
    }
}

/// The first marker pair holding a real value rather than the unexpanded expression.
fn answer_in(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(open) = rest.find("<<") {
        let after = &rest[open + 2..];
        let close = after.find(">>")?;
        let inner = strip_escapes(&after[..close]);
        if !inner.is_empty() && !inner.contains('$') {
            return Some(inner);
        }
        rest = &after[close + 2..];
    }
    None
}

/// Drops ANSI escape sequences: ESC, then everything up to the letter that terminates them.
fn strip_escapes(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == ESC {
            for e in chars.by_ref() {
                if e.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Waits for a prompt, which is the first point at which the shell is actually usable.
fn started() -> Backend {
    let mut backend = Backend::start();
    backend.pump(Duration::from_secs(20), |t, _| t.contains("PS "));
    backend
}

#[test]
fn a_shell_starts_takes_input_and_answers() {
    if !cfg!(windows) {
        return; // the fixture shell is PowerShell, same scoping as the other end-to-end tests
    }
    let mut backend = started();
    // Built from pieces so the echoed command cannot contain the answer.
    assert_eq!(backend.value_of("'LOWARC' + '_PTY_OK'"), "LOWARC_PTY_OK");
}

#[test]
fn a_resize_reaches_the_shell() {
    if !cfg!(windows) {
        return;
    }
    let mut backend = started();

    // The pty opens at 80 columns; see terminal_backend's openpty call.
    assert_eq!(backend.value_of("$Host.UI.RawUI.WindowSize.Width"), "80");

    backend.send(json!({"type": "resize", "cols": 120, "rows": 40}));

    // A resize is not instant: ConPTY has to hand the new size to the shell, and asking the
    // instant after sending it reliably still reports the old width. Polled rather than slept on,
    // so this does not turn into a fixed delay that is either flaky or wasteful.
    let mut width = String::new();
    for _ in 0..10 {
        width = backend.value_of("$Host.UI.RawUI.WindowSize.Width");
        if width == "120" {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    assert_eq!(width, "120", "a resize should reach the shell");
}

/// The regression this file exists for. On Windows, ConPTY's master reader never returns EOF when
/// the child exits, so an exit reported off the back of reader EOF never fires: the panel is left
/// with a dead shell and no way to know, and the backend process lingers. Reporting it off
/// `child.wait()` instead is what makes this pass.
#[test]
fn the_shell_exiting_is_reported_rather_than_leaving_the_session_hanging() {
    if !cfg!(windows) {
        return;
    }
    let mut backend = started();
    // Proves the shell is genuinely up first. Without this the test would also pass when the shell
    // failed to launch, since a failure to spawn reports an exit too.
    assert_eq!(backend.value_of("'ALIVE'"), "ALIVE");

    backend.input("exit\r");
    let (_, others) = backend.pump(Duration::from_secs(20), |_, msgs| {
        msgs.iter().any(|m| m.get("type").and_then(Value::as_str) == Some("exit"))
    });

    assert!(
        others.iter().any(|m| m.get("type").and_then(Value::as_str) == Some("exit")),
        "the backend has to say the shell is gone; nothing else tells the panel. Got {others:?}"
    );
}
