// A process module providing real audio playback. Reads a declarative "what should be playing"
// list every frame from the optional "audio-cues" contract, whose specification ships beside this
// module in module-sources/audio-cues. Any number of modules may provide that contract, and this
// one gathers: every provider's list is concatenated in run order, so several modules can ask for
// sound at once without one of them being elected to speak for the others. This module has no idea
// what any of them are.
//
// Inbound (shared["audio-cues"], read every frame): an array with one entry per provider, each
// tagged with the id it came from and carrying a "play" list of
// {"handle", "file", "volume" (0.0-1.0, default 1.0), "loop" (default false), "paused" (default
// false)}. That list is "this is what should be playing right now", not a one-shot command queue.
// Reconciled each frame: a handle newly present starts playing; a handle no longer present stops
// from scratch (pausing is what "paused": true is for, not omission); an existing handle's volume
// and paused state can both change live without restarting it or losing its position. The
// declarative shape is idempotent by construction, so a provider re-publishing the same list every
// frame (the normal case, since publish() carries no memory of what it said last time) never
// restarts anything already playing.
//
// Outbound (published every frame): {"playing": [handles...], "justFinished": [handles...]}. The
// latter is how a provider learns a one-shot (non-looping) sound reached its natural end, without
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
// hardware degrades gracefully" shape device_input_runtime.rs already uses for an unavailable gamepad
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
    // in that case, same shape device_input_runtime.rs uses for an unavailable gamepad backend.
    //
    // The tricky part: on some environments with no real device (a locked-down or headless CI
    // runner, confirmed live — this is not a hypothetical) opening the default sink doesn't fail
    // quickly, it just hangs indefinitely instead. A bare `.ok()` on that call would never even
    // get the chance to turn an Err into a graceful None — this module's own "start" reply, and
    // every module after it in the same run (spawn_and_run starts modules one at a time), would
    // sit frozen on it forever. Racing it against a bounded timeout on a background thread is
    // what actually makes the "missing hardware degrades gracefully" promise true rather than
    // aspirational; a device open that eventually succeeds after the deadline is simply never
    // collected — one leaked thread for the life of this otherwise-short-lived process is a fine
    // price for never hanging. (No named type for the channel here on purpose — DeviceSinkBuilder
    // ::open_default_sink()'s own return type is verbose to spell out; letting the compiler infer
    // it from the closure below is simpler and just as correct.)
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(DeviceSinkBuilder::open_default_sink().ok());
    });
    let stream = rx.recv_timeout(std::time::Duration::from_secs(3)).ok().flatten();
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
            // No mixer means no output device was available (see main's own comment on racing that
            // open against a timeout). Playback silently doing nothing for a whole run used to be
            // indistinguishable from a project that never asked for a sound; this says which.
            "start" => {
                let mut extra = Map::new();
                if mixer.is_none() {
                    extra.insert("degraded".into(), json!("no audio output device is available — nothing will play"));
                }
                reply_ok(extra);
            }
            "frame" => {
                // Gathered across every provider of the contract, in run order, rather than read
                // from one privileged module's key — see the audio-cues README. Several modules
                // can ask for sound at once without one of them being elected to speak for the
                // others; the lists simply concatenate.
                let requested: Vec<PlayRequest> = msg
                    .pointer("/shared/audio-cues")
                    .and_then(|v| v.as_array())
                    .map(|providers| {
                        providers
                            .iter()
                            .filter_map(|p| p.get("play").and_then(|v| v.as_array()))
                            .flat_map(|list| list.iter().filter_map(|v| serde_json::from_value(v.clone()).ok()))
                            .collect()
                    })
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
