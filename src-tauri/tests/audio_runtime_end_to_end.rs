// Proves bin/audio_runtime.rs actually plays real audio through the real runtime and correctly
// detects natural completion — not just that it type-checks. Manually piping the wire protocol
// into the built exe (see this module's own dev notes) already confirmed a real WAV genuinely
// plays; this test additionally proves the justFinished transition, using the same pause+step
// mechanism runtime_end_to_end.rs's own pausing/stepping test already relies on: pausing the
// engine stops it from sending this module any more "frame" phase messages, but does NOT pause
// the actual audio output (rodio plays via its own OS-level callback thread, entirely independent
// of whether this engine's frame loop is ticking) — so a real sleep while paused lets a short test
// tone genuinely finish playing in the background, and a single manual step afterward is enough
// to observe that the module's next real poll of its own playback state correctly reports it.

use lowarc_studio_lib::runtime;
use lowarc_studio_lib::runtime::runtime_loader::{Breakpoint, DebugHooks, FrameTrace, LogFn};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("lowarc_studio_audio_e2e_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A tiny (0.1s, 440Hz) mono 16-bit PCM WAV — just enough to be a real, decodable audio file
/// without vendoring a fixture asset or adding a WAV-writing dependency just for this test.
fn write_test_tone(path: &std::path::Path) {
    let sample_rate: u32 = 44100;
    let duration_secs = 0.1;
    let sample_count = (sample_rate as f32 * duration_secs) as u32;
    let data_size = sample_count * 2;

    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_size.to_le_bytes());
    for i in 0..sample_count {
        let t = i as f32 / sample_rate as f32;
        let sample = (3000.0 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

fn write_audio_module(modules_dir: &std::path::Path) {
    let dir = modules_dir.join("audio");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"audio","name":"Audio","loadOrder":1,"requires":[{"id":"director","version":"*","optional":true}]}"#).unwrap();
    std::fs::write(dir.join("process.json"), r#"{"command":"audio_runtime","args":[],"wantsFrames":true}"#).unwrap();
    let built_exe = std::path::PathBuf::from(env!("CARGO_BIN_EXE_audio_runtime"));
    let file_name = if cfg!(windows) { "audio_runtime.exe" } else { "audio_runtime" };
    std::fs::copy(&built_exe, dir.join(file_name)).unwrap();
}

// Publishes a fixed "play this one file" request every frame — a director's normal, idempotent
// shape (see audio_runtime.rs's own header comment on why re-publishing the same list every frame
// is expected, not something that should restart anything).
fn write_director_module(modules_dir: &std::path::Path, tone_path: &std::path::Path) {
    let dir = modules_dir.join("director");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"director","name":"Test Director","loadOrder":1,"requires":[]}"#).unwrap();
    std::fs::write(
        dir.join("process.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","module.ps1"],"wantsFrames":true}"#,
    )
    .unwrap();
    let tone_path_json = tone_path.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        dir.join("module.ps1"),
        format!(
            r#"
while ($line = [Console]::In.ReadLine()) {{
    if ([string]::IsNullOrWhiteSpace($line)) {{ continue }}
    $msg = $line | ConvertFrom-Json
    switch ($msg.phase) {{
        "compile" {{ $reply = @{{ ok = $true }} }}
        "start" {{ $reply = @{{ ok = $true }} }}
        "frame" {{ $reply = @{{ ok = $true; publish = @{{ play = @(@{{ handle = "tone"; file = "{tone_path_json}"; volume = 0.3; loop = $false }}) }} }} }}
        "stop" {{ $reply = @{{ ok = $true }} }}
        default {{ $reply = @{{ ok = $false; error = "unknown phase" }} }}
    }}
    [Console]::Out.WriteLine(($reply | ConvertTo-Json -Compress -Depth 5)); [Console]::Out.Flush()
    if ($msg.phase -eq "stop") {{ break }}
}}
"#
        ),
    )
    .unwrap();
}

// Same shape as write_director_module, but requests the handle paused for its first
// `paused_frames` frames, then unpaused from then on — for proving pause genuinely holds a
// sound's position rather than just being a slower way to stop it.
fn write_pausing_director_module(modules_dir: &std::path::Path, tone_path: &std::path::Path, paused_frames: u32) {
    let dir = modules_dir.join("director");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("manifest.json"), r#"{"id":"director","name":"Test Director","loadOrder":1,"requires":[]}"#).unwrap();
    std::fs::write(
        dir.join("process.json"),
        r#"{"command":"powershell","args":["-NoProfile","-ExecutionPolicy","Bypass","-File","module.ps1"],"wantsFrames":true}"#,
    )
    .unwrap();
    let tone_path_json = tone_path.to_string_lossy().replace('\\', "\\\\");
    std::fs::write(
        dir.join("module.ps1"),
        format!(
            r#"
$frameCount = 0
while ($line = [Console]::In.ReadLine()) {{
    if ([string]::IsNullOrWhiteSpace($line)) {{ continue }}
    $msg = $line | ConvertFrom-Json
    switch ($msg.phase) {{
        "compile" {{ $reply = @{{ ok = $true }} }}
        "start" {{ $reply = @{{ ok = $true }} }}
        "frame" {{
            $frameCount++
            $isPaused = $frameCount -le {paused_frames}
            $reply = @{{ ok = $true; publish = @{{ play = @(@{{ handle = "tone"; file = "{tone_path_json}"; volume = 0.3; loop = $false; paused = $isPaused }}) }} }}
        }}
        "stop" {{ $reply = @{{ ok = $true }} }}
        default {{ $reply = @{{ ok = $false; error = "unknown phase" }} }}
    }}
    [Console]::Out.WriteLine(($reply | ConvertTo-Json -Compress -Depth 5)); [Console]::Out.Flush()
    if ($msg.phase -eq "stop") {{ break }}
}}
"#
        ),
    )
    .unwrap();
}

fn noop_logger() -> LogFn {
    Arc::new(|_level, _msg| {})
}

#[test]
fn the_audio_module_plays_a_real_file_and_reports_when_it_finishes() {
    if !cfg!(windows) {
        return; // director fixture is a PowerShell script, same scoping as the other e2e tests
    }

    let modules_dir = temp_dir("modules");
    let tone_path = temp_dir("assets").join("tone.wav");
    write_test_tone(&tone_path);
    write_audio_module(&modules_dir);
    write_director_module(&modules_dir, &tone_path);

    let project_dir = temp_dir("project");
    std::fs::write(project_dir.join("project.json"), r#"{"requires":[{"id":"audio","version":"*"},{"id":"director","version":"*"}]}"#).unwrap();
    let entry = project_dir.join("main.txt");
    std::fs::write(&entry, "unused by this module").unwrap();

    let stop_flag = Arc::new(AtomicBool::new(false));
    let pause_flag = Arc::new(AtomicBool::new(false));
    let step_request = Arc::new(AtomicU32::new(0));
    let last_trace: Arc<Mutex<Option<FrameTrace>>> = Arc::new(Mutex::new(None));

    let last_trace_cb = last_trace.clone();
    let on_frame: Arc<dyn Fn(FrameTrace) + Send + Sync> = Arc::new(move |trace| {
        *last_trace_cb.lock() = Some(trace);
    });
    let debug = DebugHooks {
        pause_flag: pause_flag.clone(),
        step_request: step_request.clone(),
        breakpoints: Arc::new(Mutex::new(vec![Breakpoint::FrameCount { count: 1 }])),
        on_frame,
    };

    let entry_c = entry.clone();
    let project_dir_c = project_dir.clone();
    let modules_dir_c = modules_dir.clone();
    let stop_flag_for_thread = stop_flag.clone();
    // High target_fps so the first tick (and each subsequent step) fires as soon as it's allowed
    // to, not gated on a slow frame interval — same reasoning runtime_end_to_end.rs's own
    // pause/step test uses.
    let handle = std::thread::spawn(move || {
        runtime::start_run(&entry_c, &project_dir_c, &modules_dir_c, 200, serde_json::json!({}), stop_flag_for_thread, noop_logger(), debug)
    });

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while last_trace.lock().is_none() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the first frame");
        std::thread::sleep(Duration::from_millis(20));
    }
    let first = last_trace.lock().clone().unwrap();
    let published = first.modules.iter().find(|m| m.id == "audio").and_then(|m| m.reply.get("publish")).cloned().expect("audio module should have published something");
    assert_eq!(published["playing"], serde_json::json!(["tone"]), "should be playing right after the first frame, got {published:?}");

    // The 0.1s tone genuinely finishes playing (for real, via rodio's own background thread) well
    // within this — the engine itself stays paused/idle the whole time, sending audio_runtime no
    // further "frame" messages until the step below.
    std::thread::sleep(Duration::from_millis(500));

    step_request.store(1, Ordering::SeqCst);
    let after_step_deadline = std::time::Instant::now() + Duration::from_secs(10);
    let second = loop {
        assert!(std::time::Instant::now() < after_step_deadline, "timed out waiting for the stepped frame");
        let candidate = last_trace.lock().clone().unwrap();
        if candidate.frame_index != first.frame_index {
            break candidate;
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    let published = second.modules.iter().find(|m| m.id == "audio").and_then(|m| m.reply.get("publish")).cloned().expect("audio module should have published something");
    assert_eq!(published["playing"], serde_json::json!([]), "should no longer be playing after the tone finished, got {published:?}");
    assert_eq!(published["justFinished"], serde_json::json!(["tone"]), "should report the handle that just finished, got {published:?}");

    stop_flag.store(true, Ordering::SeqCst);
    let result = handle.join().unwrap();
    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
}

#[test]
fn pausing_holds_position_and_resuming_continues_it() {
    if !cfg!(windows) {
        return;
    }

    let modules_dir = temp_dir("pause_modules");
    let tone_path = temp_dir("pause_assets").join("tone.wav");
    write_test_tone(&tone_path);
    write_audio_module(&modules_dir);
    // Paused for its first 3 frames — at a real ~30fps free-running pace that's ~100ms of actual
    // elapsed wall-clock time, comfortably past the 0.1s tone's own natural length. If "paused"
    // didn't genuinely hold consumption, this alone would already show it as finished.
    let paused_frames = 3;
    write_pausing_director_module(&modules_dir, &tone_path, paused_frames);

    let project_dir = temp_dir("pause_project");
    std::fs::write(project_dir.join("project.json"), r#"{"requires":[{"id":"audio","version":"*"},{"id":"director","version":"*"}]}"#).unwrap();
    let entry = project_dir.join("main.txt");
    std::fs::write(&entry, "unused by this module").unwrap();

    let stop_flag = Arc::new(AtomicBool::new(false));
    let pause_flag = Arc::new(AtomicBool::new(false));
    let step_request = Arc::new(AtomicU32::new(0));
    let last_trace: Arc<Mutex<Option<FrameTrace>>> = Arc::new(Mutex::new(None));

    let last_trace_cb = last_trace.clone();
    let on_frame: Arc<dyn Fn(FrameTrace) + Send + Sync> = Arc::new(move |trace| {
        *last_trace_cb.lock() = Some(trace);
    });
    let debug = DebugHooks {
        pause_flag: pause_flag.clone(),
        step_request: step_request.clone(),
        breakpoints: Arc::new(Mutex::new(vec![Breakpoint::FrameCount { count: paused_frames as u64 }])),
        on_frame,
    };

    let entry_c = entry.clone();
    let project_dir_c = project_dir.clone();
    let modules_dir_c = modules_dir.clone();
    let stop_flag_for_thread = stop_flag.clone();
    let handle = std::thread::spawn(move || {
        runtime::start_run(&entry_c, &project_dir_c, &modules_dir_c, 30, serde_json::json!({}), stop_flag_for_thread, noop_logger(), debug)
    });

    // Free-runs (engine's own pause_flag starts false) until the breakpoint above fires at
    // frame_count == paused_frames, auto-pausing the ENGINE itself right then.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while last_trace.lock().is_none() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for the breakpoint frame");
        std::thread::sleep(Duration::from_millis(20));
    }
    let paused_trace = last_trace.lock().clone().unwrap();
    let published = paused_trace.modules.iter().find(|m| m.id == "audio").and_then(|m| m.reply.get("publish")).cloned().unwrap();
    assert_eq!(published["playing"], serde_json::json!(["tone"]), "should still be playing (paused, not finished) despite real time exceeding the tone's own length, got {published:?}");

    // One manual step: director's own frame_count becomes paused_frames + 1, crossing its "-le
    // paused_frames" threshold — this is the exact tick where it starts publishing paused:false,
    // and audio_runtime resumes the still-fresh (0 elapsed) sound in the same tick.
    let resumed_trace = step_once(&step_request, &last_trace, paused_trace.frame_index);
    let published = resumed_trace.modules.iter().find(|m| m.id == "audio").and_then(|m| m.reply.get("publish")).cloned().unwrap();
    assert_eq!(published["playing"], serde_json::json!(["tone"]), "should have resumed (still playing, not yet finished) right after unpausing, got {published:?}");

    // The engine stays paused (no more automatic ticks) while this real sleep happens — same
    // "audio plays via its own background thread regardless of engine ticking" reasoning as the
    // other test above — long enough for the now-resumed 0.1s tone to genuinely reach its end.
    std::thread::sleep(Duration::from_millis(500));
    let finished_trace = step_once(&step_request, &last_trace, resumed_trace.frame_index);
    let published = finished_trace.modules.iter().find(|m| m.id == "audio").and_then(|m| m.reply.get("publish")).cloned().unwrap();
    assert_eq!(published["playing"], serde_json::json!([]), "should no longer be playing once the resumed tone actually finished, got {published:?}");
    assert_eq!(published["justFinished"], serde_json::json!(["tone"]), "should report the handle that just finished, got {published:?}");

    stop_flag.store(true, Ordering::SeqCst);
    let result = handle.join().unwrap();
    assert!(result.is_ok(), "expected the run to complete cleanly, got {result:?}");
}

/// Requests exactly one more step and waits for the resulting frame's trace to land — shared by
/// pausing_holds_position_and_resuming_continues_it's two step-and-observe points.
fn step_once(step_request: &Arc<AtomicU32>, last_trace: &Arc<Mutex<Option<FrameTrace>>>, previous_frame_index: u64) -> FrameTrace {
    step_request.store(1, Ordering::SeqCst);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for a stepped frame");
        let candidate = last_trace.lock().clone().unwrap();
        if candidate.frame_index != previous_frame_index {
            return candidate;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
