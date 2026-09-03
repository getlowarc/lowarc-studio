// A process module providing real audio playback. Reads a declarative "what should be playing"
// list every frame from an optional, GENERIC convention dependency named "director" — any
// project's own game-logic module (whatever it's actually built as) can fill that role just by
// being installed with that id, same pattern as node_graph_runtime's optional "input" dependency.
// This module doesn't know or care whether "director" happens to be node-graph-runtime, a
// hand-written module, or anything else — same reasoning input stays ignorant of node graphs.
//
// Inbound (shared.director.play, read every frame): a list of
// {"handle", "file", "volume" (0.0-1.0, default 1.0), "loop" (default false), "paused" (default
// false)} — "this is what should be playing right now", not a one-shot command queue. Reconciled
// each frame: a handle newly present starts playing; a handle no longer present stops (from
// scratch — pausing is what "paused": true is for, not omission); an existing handle's volume and
// paused state can both change live without restarting it or losing its position. This
// declarative shape is idempotent by construction — a director re-publishing the same list every
// frame (the normal case, since publish() carries no memory of what it said last time) never
// restarts anything already playing.
//
// Outbound (published every frame): {"playing": [handles...], "justFinished": [handles...]} — the
// latter is how a director learns a one-shot (non-looping) sound reached its natural end, without
// needing any other way to poll for it.
//
// `file` is resolved relative to the PROJECT root, captured from the "compile" phase's own
// sourcePath — every module gets that on compile, not just whichever one is actually driving the
// project's entry file (see runtime::process_module's request-building) — so this needs nothing
// project-specific hardcoded to find it.
//
// Cross-platform via rodio (Windows/macOS/Linux, itself built on cpal). A platform or environment
// with no real audio output device at all (a locked-down or headless CI runner, say) doesn't fail
// the whole module — playing/justFinished just stays empty every frame, the same "missing
// hardware degrades gracefully" shape input_runtime.rs already uses for an unavailable gamepad
// backend.
//
// Speaks the standard process-module wire protocol (compile/start/frame/stop — see
// runtime::process_module's own header comment for the shared/publish half of it).

use rodio::{mixer::Mixer, source::Source, Decoder, DeviceSinkBuilder, Player};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

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

#[derive(Debug, Deserialize, Clone, PartialEq)]
struct PlayRequest {
    handle: String,
    file: String,
    #[serde(default = "default_volume")]
    volume: f32,
    #[serde(default, rename = "loop")]
    looped: bool,
    /// Pausing is a live update (rodio's Player.pause()/play(), keeping position) same as volume
    /// — not a stop-and-restart. A paused sound stays in `active` and never counts toward
    /// justFinished (an untouched, non-empty queue just sits there, whether or not it's paused).
    #[serde(default)]
    paused: bool,
}

fn default_volume() -> f32 {
    1.0
}

struct ActiveSound {
    player: Player,
    looped: bool,
    // The exact request that started this sound — so a later frame can tell "this handle's
    // request is unchanged" (nothing to do) apart from "only volume/paused changed" (update in
    // place) apart from "the file/loop changed" (only volume/paused are adjustable live; anything
    // else means restarting the sound fresh, since rodio has no way to swap a Player's source
    // mid-flight).
    request: PlayRequest,
}

/// Decodes `file` (resolved against `project_root`, if it isn't already absolute) fresh and starts
/// it playing on `mixer` — `.repeat_infinite()` before ever handing the source to the Player is
/// what makes looping gapless (rodio buffers it in memory rather than this module re-decoding the
/// file from disk every time it would otherwise reach the end).
fn start_sound(mixer: &Mixer, project_root: &Option<PathBuf>, req: &PlayRequest) -> Result<Player, String> {
    let path = PathBuf::from(&req.file);
    let resolved = if path.is_absolute() { path } else { project_root.as_deref().map(|root| root.join(&path)).unwrap_or(path) };
    let file = std::fs::File::open(&resolved).map_err(|e| format!("could not open {}: {e}", resolved.display()))?;
    let decoder = Decoder::try_from(BufReader::new(file)).map_err(|e| format!("could not decode {}: {e}", resolved.display()))?;

    let player = Player::connect_new(mixer);
    if req.looped {
        player.append(decoder.repeat_infinite());
    } else {
        player.append(decoder);
    }
    player.set_volume(req.volume);
    Ok(player)
}

fn main() {
    // A platform/environment with no real audio output at all shouldn't fail the whole module —
    // see this file's own header comment. Every "frame" reply below just reports nothing playing
    // in that case, same shape input_runtime.rs uses for an unavailable gamepad backend.
    let stream = DeviceSinkBuilder::open_default_sink().ok();
    let mixer = stream.as_ref().map(|s| s.mixer());

    let mut project_root: Option<PathBuf> = None;
    let mut active: HashMap<String, ActiveSound> = HashMap::new();

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        let phase = msg.get("phase").and_then(|p| p.as_str()).unwrap_or("");

        match phase {
            "compile" => {
                project_root = msg.get("sourcePath").and_then(|p| p.as_str()).map(PathBuf::from).and_then(|p| p.parent().map(|p| p.to_path_buf()));
                reply_ok(Map::new());
            }
            "start" => reply_ok(Map::new()),
            "frame" => {
                let requested: Vec<PlayRequest> = msg
                    .pointer("/shared/director/play")
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()).collect())
                    .unwrap_or_default();
                let requested_by_handle: HashMap<&str, &PlayRequest> = requested.iter().map(|r| (r.handle.as_str(), r)).collect();

                // Stop anything no longer requested.
                active.retain(|handle, _| requested_by_handle.contains_key(handle.as_str()));

                let mut just_finished = Vec::new();
                if let Some(mixer) = mixer {
                    for req in &requested {
                        match active.get_mut(&req.handle) {
                            // Same file/loop as before — volume and paused can both change live,
                            // in either direction, without restarting the sound (position is kept
                            // exactly as rodio's Player already does internally).
                            Some(sound) if sound.request.file == req.file && sound.request.looped == req.looped => {
                                if sound.request.volume != req.volume {
                                    sound.player.set_volume(req.volume);
                                }
                                if sound.request.paused != req.paused {
                                    if req.paused {
                                        sound.player.pause();
                                    } else {
                                        sound.player.play();
                                    }
                                }
                                sound.request = req.clone();
                            }
                            // Either genuinely new, or the file/loop changed under an existing
                            // handle — restart it fresh either way, since a Player's source can't
                            // be swapped after the fact.
                            _ => match start_sound(mixer, &project_root, req) {
                                Ok(player) => {
                                    if req.paused {
                                        player.pause();
                                    }
                                    active.insert(req.handle.clone(), ActiveSound { player, looped: req.looped, request: req.clone() });
                                }
                                Err(e) => {
                                    // Not fatal to the module — one bad file shouldn't take down
                                    // every other sound already playing.
                                    reply_err(&e);
                                }
                            },
                        }
                    }

                    active.retain(|handle, sound| {
                        if !sound.looped && sound.player.empty() {
                            just_finished.push(handle.clone());
                            false
                        } else {
                            true
                        }
                    });
                }

                let playing: Vec<Value> = active.keys().cloned().map(Value::String).collect();
                let publish = json!({ "playing": playing, "justFinished": just_finished });
                let mut extra = Map::new();
                extra.insert("publish".into(), publish);
                reply_ok(extra);
            }
            "stop" => {
                active.clear();
                reply_ok(Map::new());
                break;
            }
            other => reply_err(&format!("unknown phase \"{other}\"")),
        }
    }
}
