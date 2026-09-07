// A process module providing real OS keyboard/mouse/gamepad state — the input capability every
// game needs, with nothing project-specific about it (same "generic, reusable module" reasoning
// as node_graph_runtime). Publishes, every frame:
//   {"keyboard": {"keysDown": [...]},
//    "mouse": {"x": .., "y": .., "buttonsDown": [1, 2, ...]},
//    "gamepads": [{"id": .., "name": .., "buttonsDown": [...], "axes": {"LeftStickX": 0.5, ...}}]}
// A project's OWN entry file/logic decides what any of that MEANS (which key does what) — this
// module only ever reports what's physically true right now, the same "position/state only, no
// interpretation" split node_graph_runtime already draws around what a node's files mean.
//
// Deliberately does NOT cover graphics-tablet/stylus input. That's not a scope cut of
// convenience — pen/tablet input is architecturally different from keyboard/mouse/gamepad: on
// Windows it's delivered through the RealTimeStylus COM API tied to a real window, and on Wayland
// the tablet-v2 protocol is inherently surface-scoped (Wayland has no concept of "global" input
// outside a window at all, by design — the same reason it doesn't have global keyboard/mouse
// hooks either, see below). Every tablet crate surveyed for this (octotablet, in particular) needs
// a real raw_window_handle to attach to. This module, like every other process module in this
// engine, is a headless background process with no window of its own — supporting tablets for
// real would mean this module (or a sibling one) owning an actual OS window, which is a
// meaningfully different, bigger piece of work than what's here, not a small addition to it.
//
// Platform coverage, honestly: keyboard/mouse polling (device_query) works on Windows, macOS, and
// Linux/X11 specifically — NOT Linux/Wayland, for the same "no global input outside a window"
// reason as tablets above. Gamepad polling (gilrs) covers Windows, macOS, Linux/BSD, and Wasm —
// broader, since gamepads are read through a dedicated HID/joystick subsystem on every platform
// rather than through the windowing server itself.
//
// Speaks the standard process-module wire protocol (compile/start/frame/stop — see
// runtime::process_module's own header comment for the shared/publish half of it). "compile" is a
// no-op: this module has nothing to read from a project's entry file, it reports hardware state
// regardless of what's running.

use device_query::{DeviceQuery, DeviceState};
use gilrs::{Axis, Button, Gilrs};
use serde_json::{json, Map, Value};
use std::io::{self, BufRead, Write};

fn write_line(value: &Value) {
    let mut out = io::stdout();
    let _ = writeln!(out, "{value}");
    let _ = out.flush();
}

fn reply_ok(mut extra: Map<String, Value>) {
    extra.insert("ok".into(), Value::Bool(true));
    write_line(&Value::Object(extra));
}

fn reply_err(error: &str) {
    write_line(&json!({"ok": false, "error": error}));
}

fn poll_keyboard_and_mouse(device_state: &DeviceState) -> Map<String, Value> {
    let keys = device_state.get_keys();
    let mouse = device_state.get_mouse();
    // button_pressed is 1-based (index 0 is always false, not a real button) — see MouseState's
    // own doc comment in device_query.
    let buttons_down: Vec<Value> = mouse.button_pressed.iter().enumerate().skip(1).filter(|(_, &down)| down).map(|(i, _)| Value::from(i)).collect();

    let mut out = Map::new();
    out.insert("keyboard".into(), json!({ "keysDown": keys.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>() }));
    out.insert("mouse".into(), json!({ "x": mouse.coords.0, "y": mouse.coords.1, "buttonsDown": buttons_down }));
    out
}

// Every named (non-Unknown) variant, spelled out explicitly rather than attempted generically —
// gilrs doesn't expose an EnumIter-style "give me every variant" itself, and hand-listing these
// once here is simpler and more obviously correct than reaching for a third-party enum-iteration
// crate for two short, stable lists.
const GAMEPAD_BUTTONS: [Button; 19] = [
    Button::South,
    Button::East,
    Button::North,
    Button::West,
    Button::C,
    Button::Z,
    Button::LeftTrigger,
    Button::LeftTrigger2,
    Button::RightTrigger,
    Button::RightTrigger2,
    Button::Select,
    Button::Start,
    Button::Mode,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadDown,
    Button::DPadLeft,
    Button::DPadRight,
];
const GAMEPAD_AXES: [Axis; 8] =
    [Axis::LeftStickX, Axis::LeftStickY, Axis::LeftZ, Axis::RightStickX, Axis::RightStickY, Axis::RightZ, Axis::DPadX, Axis::DPadY];

fn poll_gamepads(gilrs: &mut Gilrs) -> Vec<Value> {
    // gilrs's is_pressed()/value() reflect whatever its internal state was last updated to by
    // processing queued events — draining them here (state-updating is automatic per event,
    // per Gilrs's own docs) is what keeps that state current going into this frame's read below,
    // same "pump events, then query state" shape gilrs's own game-loop example uses.
    while gilrs.next_event().is_some() {}
    gilrs.inc();

    gilrs
        .gamepads()
        .map(|(id, gamepad)| {
            let buttons_down: Vec<Value> =
                GAMEPAD_BUTTONS.iter().filter(|b| gamepad.is_pressed(**b)).map(|b| Value::String(format!("{b:?}"))).collect();
            let mut axes = Map::new();
            for axis in GAMEPAD_AXES {
                let v = gamepad.value(axis);
                if v != 0.0 {
                    axes.insert(format!("{axis:?}"), json!(v));
                }
            }
            json!({
                "id": format!("{id:?}"),
                "name": gamepad.name(),
                "buttonsDown": buttons_down,
                "axes": axes,
            })
        })
        .collect()
}

fn main() {
    let device_state = DeviceState::new();
    // A platform genuinely without a working gamepad backend (rare, but not impossible — a locked-
    // down or headless environment, say) shouldn't take down keyboard/mouse reporting with it;
    // gamepads just reports empty every frame instead.
    let mut gilrs = Gilrs::new().ok();

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let phase = msg.get("phase").and_then(|p| p.as_str()).unwrap_or("");

        match phase {
            "compile" => reply_ok(Map::new()),
            "start" => reply_ok(Map::new()),
            "frame" => {
                let mut publish = poll_keyboard_and_mouse(&device_state);
                let gamepads = match &mut gilrs {
                    Some(g) => poll_gamepads(g),
                    None => Vec::new(),
                };
                publish.insert("gamepads".into(), Value::Array(gamepads));

                let mut extra = Map::new();
                extra.insert("publish".into(), Value::Object(publish));
                reply_ok(extra);
            }
            "stop" => {
                reply_ok(Map::new());
                break;
            }
            other => reply_err(&format!("unknown phase \"{other}\"")),
        }
    }
}
