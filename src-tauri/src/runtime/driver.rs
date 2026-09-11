// Same paced default loop as Bootstrap's driver.rs, adapted for one thing: it checks a
// caller-supplied stop flag instead of a process-wide global, since a dev-run session isn't the
// whole process's lifetime here: the IDE can start, stop, and start another run again later.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(windows)]
mod winmm {
    #[link(name = "winmm")]
    extern "system" {
        pub fn timeBeginPeriod(period: u32) -> u32;
        pub fn timeEndPeriod(period: u32) -> u32;
    }
}

#[cfg(windows)]
fn raise_timer_resolution() -> bool {
    unsafe { winmm::timeBeginPeriod(1) == 0 }
}
#[cfg(windows)]
fn restore_timer_resolution() {
    unsafe {
        winmm::timeEndPeriod(1);
    }
}
#[cfg(not(windows))]
fn raise_timer_resolution() -> bool {
    false
}
#[cfg(not(windows))]
fn restore_timer_resolution() {}

/// Runs `tick(delta_seconds)` at `target_fps` until `stop_flag` is set. Same precise-wait shape
/// as Bootstrap's PacedDriver: sleep the bulk of the remaining time, spin the last ~1ms so frames
/// land on the deadline instead of drifting on coarse sleeps.
///
/// `pause_flag` and `step_request` gate that pacing for the debugger. While paused nothing ticks
/// unless a step is requested, in which case exactly one tick fires, still paced against
/// `target_fps` rather than immediately. The step count decrements when that tick actually happens,
/// not when the step is first seen: decrementing early would let another thread read a second
/// in-flight step as already spent. `last` is reset before the first tick after a pause, so
/// `delta` is one interval rather than however long a human stared at a breakpoint.
pub fn run<F: FnMut(f64)>(target_fps: u32, stop_flag: &Arc<AtomicBool>, pause_flag: &Arc<AtomicBool>, step_request: &Arc<AtomicU32>, mut tick: F) {
    let raised = raise_timer_resolution();
    let start = Instant::now();
    let interval = 1.0 / (target_fps.max(1) as f64);
    let mut last = start.elapsed().as_secs_f64();
    let mut was_paused = false;

    while !stop_flag.load(Ordering::SeqCst) {
        let paused = pause_flag.load(Ordering::SeqCst);
        let stepping = paused && step_request.load(Ordering::SeqCst) > 0;

        if paused && !stepping {
            was_paused = true;
            std::thread::sleep(Duration::from_millis(15));
            continue;
        }

        let now = start.elapsed().as_secs_f64();
        if was_paused {
            was_paused = false;
            last = now - interval;
        }
        let delta = now - last;
        if delta >= interval {
            last = now;
            if stepping {
                step_request.fetch_sub(1, Ordering::SeqCst);
            }
            tick(delta);
        } else if interval - delta > 0.002 {
            std::thread::sleep(Duration::from_millis(1));
        } else {
            std::hint::spin_loop();
        }
    }

    if raised {
        restore_timer_resolution();
    }
}
