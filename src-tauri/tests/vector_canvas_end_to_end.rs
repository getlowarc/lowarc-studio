// Drives the real vector_canvas_runtime binary through the process-module wire protocol the way the
// engine does (compile, start, frames, stop), and asserts on what it actually replies.
//
// The point is that a drawing module is easy to "verify" by looking at it and hard to verify
// honestly. These tests don't claim pixels are correct; they pin the things that would silently
// break a run: that it answers every phase, that a frame's draw commands don't wedge or crash it,
// that a bad command is survivable, and that what it publishes has the shape a director will read.
//
// Where a display exists this opens a real window for a moment. Where one doesn't (a headless CI
// runner) the module's own no-display path takes over and every assertion below still has to hold,
// which is the property that matters most here, since that's the environment that has broken this
// project before.

use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};

fn canvas_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_vector_canvas_runtime"))
}

struct Module {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    /// Only read when the module dies unexpectedly, and it must actually be read: piping stderr
    /// and never draining it leaves a panic message sitting in the pipe, so the failure reports
    /// "the module closed its output before replying" and nothing else. On a CI runner that message
    /// is the whole answer.
    stderr: Option<std::process::ChildStderr>,
}

impl Module {
    fn start() -> Self {
        let mut child = Command::new(canvas_bin())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("vector_canvas_runtime should start");
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
        let stderr = child.stderr.take();
        Self { child, stdin, stdout, stderr }
    }

    /// Whatever the module wrote to stderr before dying: its panic message, in practice.
    fn drain_stderr(&mut self) -> String {
        let Some(mut err) = self.stderr.take() else { return "<stderr already taken>".into() };
        let mut buf = String::new();
        use std::io::Read;
        let _ = err.read_to_string(&mut buf);
        if buf.trim().is_empty() {
            "<nothing on stderr>".into()
        } else {
            buf
        }
    }

    fn send(&mut self, msg: Value) {
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Reads until an actual reply arrives, skipping the `{"log":...}` and `{"requestStop":...}`
    /// notifications a module may emit unprompted at any time: exactly what the engine's own
    /// stdout reader does with them.
    fn reply(&mut self) -> Value {
        loop {
            let mut line = String::new();
            let read = self.stdout.read_line(&mut line).expect("reading the module's stdout");
            if read == 0 {
                panic!("the module closed its output before replying. Its stderr:\n{}", self.drain_stderr());
            }
            let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
            if value.get("log").is_some() || value.get("requestStop").is_some() {
                continue;
            }
            return value;
        }
    }

    /// Returns the start reply, since whether it carries a `degraded` marker is itself part of the
    /// contract. See the degraded-marker assertion in the first test.
    fn compile_and_start(&mut self, settings: Value) -> Value {
        self.send(serde_json::json!({"phase": "compile", "sourceCode": "", "sourcePath": "project/main.txt"}));
        assert_eq!(self.reply().get("ok").and_then(Value::as_bool), Some(true), "compile should succeed");
        self.send(serde_json::json!({"phase": "start", "settings": settings, "items": []}));
        let reply = self.reply();
        assert_eq!(reply.get("ok").and_then(Value::as_bool), Some(true), "start should succeed even with no display: {reply}");
        reply
    }

    /// One frame carrying `draw` as a single provider's contribution to the draw-commands
    /// contract: the gathered array shape the canvas reads now that any number of modules may
    /// draw (see module-sources/draw-commands/README.md).
    fn frame(&mut self, draw: Value) -> Value {
        self.send(serde_json::json!({"phase": "frame", "delta": 0.016, "shared": {"draw-commands": [{"from": "test-director", "draw": draw}]}}));
        self.reply()
    }

    fn stop(mut self) {
        self.send(serde_json::json!({"phase": "stop"}));
        assert_eq!(self.reply().get("ok").and_then(Value::as_bool), Some(true), "stop should succeed");
        let _ = self.child.wait();
    }
}

#[test]
fn it_answers_every_phase_and_publishes_a_surface_a_director_can_read() {
    let mut module = Module::start();
    let start = module.compile_and_start(serde_json::json!({"title": "test", "width": 320, "height": 240, "vsync": false}));

    let reply = module.frame(serde_json::json!([
        {"op": "clear", "color": "#101820"},
        {"op": "rect", "x": 10, "y": 10, "w": 40, "h": 40, "fill": "#ff8800"},
    ]));

    assert_eq!(reply.get("ok").and_then(Value::as_bool), Some(true), "a frame with valid commands should succeed: {reply}");
    let published = reply.get("publish").expect("every frame must publish its surface state");

    // The contract a director actually codes against. Sizes are whatever the environment gave us
    // (zero on a headless runner), so this pins presence and type, not values.
    assert!(published.get("width").and_then(Value::as_u64).is_some(), "width must be published: {published}");
    assert!(published.get("height").and_then(Value::as_u64).is_some(), "height must be published: {published}");
    assert!(published.get("keys").and_then(Value::as_array).is_some(), "keys must be published as an array: {published}");
    assert!(published.get("focused").and_then(Value::as_bool).is_some(), "focused must be published: {published}");
    assert_eq!(
        published.get("closeRequested").and_then(Value::as_bool),
        Some(false),
        "nothing has asked to close: {published}"
    );

    // The degraded marker has to agree with reality, and this holds on BOTH kinds of machine
    // without the test needing to know which it is on: a developer box reports a real window and
    // no marker, a display-less CI runner reports neither and must say so. Pinning the pair is what
    // makes "it degrades" a checked claim rather than an intention.
    let width = published.get("width").and_then(Value::as_u64).unwrap_or(0);
    let degraded = start.get("degraded").and_then(Value::as_str);
    if width > 0 {
        assert!(degraded.is_none(), "a real window was created, so nothing should be reported as degraded: {start}");
    } else {
        let reason = degraded.expect("no window means the start reply must say why, rather than leaving it silent");
        assert!(!reason.trim().is_empty(), "the degraded reason must actually say something: {start}");
    }

    let mouse = published.get("mouse").expect("mouse state must be published");
    for key in ["x", "y", "windowX", "windowY", "scroll"] {
        assert!(mouse.get(key).and_then(Value::as_f64).is_some(), "mouse.{key} must be a number: {mouse}");
    }
    for key in ["left", "right", "middle", "inside"] {
        assert!(mouse.get(key).and_then(Value::as_bool).is_some(), "mouse.{key} must be a bool: {mouse}");
    }

    module.stop();
}

#[test]
fn one_bad_command_does_not_take_down_the_frame_around_it() {
    // The whole reason unknown ops are skipped rather than fatal: a director with a typo, or built
    // against a newer canvas than the one installed, must not lose everything else it drew.
    let mut module = Module::start();
    module.compile_and_start(serde_json::json!({"width": 320, "height": 240, "vsync": false}));

    let reply = module.frame(serde_json::json!([
        {"op": "clear", "color": "#000"},
        {"op": "there-is-no-such-op", "x": 1},
        {"op": "image", "file": "does/not/exist.png", "x": 0, "y": 0},
        {"op": "text", "text": "no font given", "x": 0, "y": 0},
        {"op": "rect", "x": 0, "y": 0, "w": 10, "h": 10, "fill": "#fff"},
    ]));
    assert_eq!(reply.get("ok").and_then(Value::as_bool), Some(true), "a frame with bad commands must still succeed: {reply}");

    // And it keeps working afterwards, rather than being left in a broken state.
    let reply = module.frame(serde_json::json!([{"op": "rect", "x": 0, "y": 0, "w": 5, "h": 5, "fill": "#0f0"}]));
    assert_eq!(reply.get("ok").and_then(Value::as_bool), Some(true), "the next frame must still work: {reply}");

    module.stop();
}

#[test]
fn a_frame_with_no_director_at_all_is_a_normal_empty_frame() {
    // The state on first run: the canvas installed, nothing publishing draw commands yet. It has to
    // be a quiet no-op, not an error. Otherwise every run would fail until a director exists.
    let mut module = Module::start();
    module.compile_and_start(serde_json::json!({"width": 320, "height": 240, "vsync": false}));

    module.send(serde_json::json!({"phase": "frame", "delta": 0.016, "shared": {}}));
    let reply = module.reply();
    assert_eq!(reply.get("ok").and_then(Value::as_bool), Some(true), "a frame with no director should succeed: {reply}");
    assert!(reply.get("publish").is_some(), "it should still publish its surface state: {reply}");

    module.stop();
}

#[test]
fn an_unknown_phase_is_a_visible_error_rather_than_silence() {
    let mut module = Module::start();
    module.send(serde_json::json!({"phase": "nonsense"}));
    let reply = module.reply();
    assert_eq!(reply.get("ok").and_then(Value::as_bool), Some(false), "an unknown phase must report failure: {reply}");
    assert!(
        reply.get("error").and_then(Value::as_str).unwrap_or("").contains("nonsense"),
        "the error should name the phase it didn't understand: {reply}"
    );
    module.stop();
}
