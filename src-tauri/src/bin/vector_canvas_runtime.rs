// A process module providing a real 2D drawing surface, the thing that makes a LowArc run stop
// being headless. Opens a window, owns an OpenGL context, and draws whatever the project's own
// modules tell it to, read every frame from the optional "draw-commands" contract whose
// specification ships beside this module in module-sources/draw-commands.
//
// Any number of modules may provide that contract and this one gathers: every provider's command
// list is concatenated in run order. Nothing is elected to speak for the rest, so a world module,
// a UI layer and a debug overlay each simply draw. Since a provider always runs before its
// consumers, run order is z-order, and a module that requires the one it annotates draws on top of
// it without anyone arranging that. This module has no idea what any of them are.
//
// Named "vector-canvas", not "canvas", deliberately. It is a VECTOR renderer (antialiased paths and
// strokes, via femtovg), not a sprite blitter, and it is one possible surface rather than the only
// one: a pixel/tile surface or a 3D one should be able to sit beside it as a peer instead of being
// "the other canvas". The command vocabulary below is this module's own interface, not an
// engine-wide drawing protocol; a different surface module is free to speak differently.
//
// Rendering is femtovg (GPU, OpenGL ES 3.0+), whose API is modelled on the HTML5 Canvas API: the
// same drawing model p5.js wraps. That's what makes growing this cheap: every new command is one
// more arm in draw_command()'s match, mapping a JSON object onto a femtovg call. Rasterization,
// antialiasing and glyph shaping are already solved by that crate.
//
// Inbound (shared["draw-commands"], read every frame): an array with one entry per provider, each
// tagged with the id it came from and carrying a "draw" list of {"op": ...} commands. Those are
// executed IN ORDER, which is also the draw order. Unlike audio's declarative "what should be
// playing" list, drawing is immediate-mode by nature: a frame draws exactly what it was handed,
// and a frame handed nothing draws nothing. Images and fonts are the exception and ARE cached by
// path, since decoding a PNG or parsing a TTF every frame would be the obvious performance trap.
//
// Outbound (published every frame): window size, plus mouse/keyboard/focus state. Input is here
// because this module owns the window, so it's the only thing that can report a pointer position in
// CANVAS coordinates (the camera transform inverted). That deliberately overlaps the
// "device-input" module, which polls the OS globally in screen space: that one remains the right
// source for gamepads and for input that isn't about this window.
//
// Closing the window sends {"requestStop":true}, ending the run the same way any module asking to
// stop already does (see process_module.rs's spawn_stdout_reader).
//
// Speaks the standard process-module wire protocol (compile/start/frame/stop — see
// runtime::process_module's own header comment for the shared/publish half of it).

use femtovg::{renderer::OpenGl, Align, Canvas, Color, FontId, ImageFlags, ImageId, Paint, Path};
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::pump_events::EventLoopExtPumpEvents;
use winit::raw_window_handle::HasWindowHandle;
use winit::window::{Window, WindowId};

// ---------- wire protocol ----------

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

fn log(severity: &str, message: &str) {
    write_line(&json!({"log": {"severity": severity, "message": message}}));
}

// ---------- settings ----------

/// Read from the "start" phase's own settings object. Every field has a default, so a project that
/// configures nothing still gets a usable window.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct WindowSettings {
    title: String,
    width: u32,
    height: u32,
    resizable: bool,
    vsync: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self { title: "LowArc".into(), width: 960, height: 640, resizable: true, vsync: true }
    }
}

// ---------- camera ----------

/// Inactive by default, so an unconfigured canvas behaves the way a 2D surface is normally expected
/// to: (0,0) is the top-left pixel and one unit is one pixel. A "camera" command opts into a
/// centred, scalable view instead — world point (x,y) at the middle of the window, scaled by zoom.
#[derive(Debug, Clone, Copy)]
struct Camera {
    x: f32,
    y: f32,
    zoom: f32,
    active: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self { x: 0.0, y: 0.0, zoom: 1.0, active: false }
    }
}

impl Camera {
    /// The inverse of what apply() does to the canvas — used to report the pointer in the same
    /// space the caller is drawing in, which is the whole reason this module publishes input.
    fn to_canvas(self, wx: f32, wy: f32, width: f32, height: f32) -> (f32, f32) {
        if !self.active || self.zoom == 0.0 {
            return (wx, wy);
        }
        ((wx - width / 2.0) / self.zoom + self.x, (wy - height / 2.0) / self.zoom + self.y)
    }

    fn apply(&self, canvas: &mut Canvas<OpenGl>, width: f32, height: f32) {
        canvas.reset_transform();
        if !self.active {
            return;
        }
        canvas.translate(width / 2.0, height / 2.0);
        canvas.scale(self.zoom, self.zoom);
        canvas.translate(-self.x, -self.y);
    }
}

// ---------- input state ----------

/// Accumulated from window events between frames, published on each frame reply. `scroll` is a
/// per-frame delta rather than a running total (it's reset once published), matching how a scroll
/// wheel actually reads; everything else is level state.
#[derive(Debug, Default)]
struct InputState {
    mouse_x: f32,
    mouse_y: f32,
    inside: bool,
    left: bool,
    right: bool,
    middle: bool,
    scroll: f32,
    keys: HashSet<String>,
    focused: bool,
    close_requested: bool,
}

// ---------- the window + GL + canvas, once they exist ----------

struct Gfx {
    window: Window,
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    canvas: Canvas<OpenGl>,
}

struct App {
    gfx: Option<Gfx>,
    /// Set once the "start" phase arrives: the window can't be built before then, since its title
    /// and size come from that phase's settings.
    settings: Option<WindowSettings>,
    /// Latched after a failed attempt so a broken display doesn't get retried on every single pump,
    /// flooding the log. See run_windowed's own comment on degrading rather than failing.
    init_failed: bool,
    input: InputState,
    camera: Camera,
    images: HashMap<String, ImageId>,
    fonts: HashMap<String, FontId>,
    project_root: Option<PathBuf>,
}

impl App {
    fn new() -> Self {
        Self {
            gfx: None,
            settings: None,
            init_failed: false,
            input: InputState::default(),
            camera: Camera::default(),
            images: HashMap::new(),
            fonts: HashMap::new(),
            project_root: None,
        }
    }

    /// Called from both resumed() and new_events() rather than resumed() alone: resumed fires once,
    /// at startup, which on this module is BEFORE the "start" phase has arrived with the settings
    /// the window needs. new_events fires every pump, so the window gets built on whichever of the
    /// two happens to come after settings land.
    fn ensure_window(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() || self.init_failed {
            return;
        }
        let Some(settings) = self.settings.as_ref() else { return };

        // catch_unwind rather than just the Result, because this stack PANICS instead of returning
        // Err on a machine with no usable GL, which is what a CI runner is. Two known panics the
        // Result cannot express: glutin's config picker must produce a Config or panic, and an
        // empty iterator means no matching GL config; and build_surface_attributes panics on a
        // zero-sized window. Guarding the boundary rather than each known cause is what also covers
        // the next driver quirk. This is the module's whole degrade-without-a-display promise.
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build_window(event_loop, settings)));

        self.init_failed = true;
        match built {
            Ok(Ok(gfx)) => {
                self.gfx = Some(gfx);
                self.init_failed = false;
            }
            // These report the CAUSE only. That the module is running without a window is announced
            // once, by the degraded marker on its start reply — saying it here too would be the same
            // news twice, from a less useful place.
            Ok(Err(e)) => log("warn", &format!("vector-canvas: window creation failed — {e}")),
            // The panic's own message has already gone to stderr, which the host relays.
            Err(_) => log("warn", "vector-canvas: window creation panicked"),
        }
    }

}

/// Assets are named relative to the PROJECT root (captured from the compile phase's own sourcePath),
/// the same convention audio_playback_runtime.rs resolves its sound files by, so a director can say
/// "art/player.png" without knowing where the project lives. An already-absolute path is left alone.
/// A free function rather than a method so callers can hold a disjoint borrow of the canvas at the
/// same time; see draw_command's destructuring.
fn resolve_asset(project_root: &Option<PathBuf>, file: &str) -> PathBuf {
    let path = PathBuf::from(file);
    if path.is_absolute() {
        return path;
    }
    project_root.as_deref().map(|root| root.join(&path)).unwrap_or(path)
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.ensure_window(event_loop);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, _cause: winit::event::StartCause) {
        self.ensure_window(event_loop);
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.input.close_requested = true,
            WindowEvent::Focused(focused) => self.input.focused = focused,
            WindowEvent::CursorEntered { .. } => self.input.inside = true,
            WindowEvent::CursorLeft { .. } => self.input.inside = false,
            WindowEvent::CursorMoved { position, .. } => {
                self.input.mouse_x = position.x as f32;
                self.input.mouse_y = position.y as f32;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                self.input.scroll += match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let down = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => self.input.left = down,
                    MouseButton::Right => self.input.right = down,
                    MouseButton::Middle => self.input.middle = down,
                    _ => {}
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // The physical key, not the produced character: a consumer checking for "the W key"
                // wants the same answer on a QWERTY and an AZERTY keyboard, which is what a
                // layout-independent code gives and a text character doesn't.
                let name = format!("{:?}", event.physical_key);
                if event.state == ElementState::Pressed {
                    self.input.keys.insert(name);
                } else {
                    self.input.keys.remove(&name);
                }
            }
            WindowEvent::Resized(size) => {
                if let Some(gfx) = self.gfx.as_mut() {
                    gfx.window.resize_surface(&gfx.surface, &gfx.context);
                    gfx.canvas.set_size(size.width, size.height, gfx.window.scale_factor() as f32);
                }
            }
            _ => {}
        }
    }
}

/// All the glutin/winit bootstrapping in one place. On Windows (WGL) the window attributes MUST go
/// through DisplayBuilder rather than being created separately — glutin-winit's own docs are
/// explicit that modern OpenGL is otherwise unavailable, which is why this builds the window and
/// the GL config together rather than in two steps.
fn build_window(event_loop: &ActiveEventLoop, settings: &WindowSettings) -> Result<Gfx, String> {
    let attributes = Window::default_attributes()
        .with_title(&settings.title)
        .with_inner_size(winit::dpi::LogicalSize::new(settings.width, settings.height))
        .with_resizable(settings.resizable);

    let (window, gl_config) = DisplayBuilder::new()
        .with_window_attributes(Some(attributes))
        .build(event_loop, ConfigTemplateBuilder::new().with_alpha_size(8), |configs| {
            // Most samples wins. Antialiasing is the whole point of a vector renderer.
            configs
                .reduce(|best, config| if config.num_samples() > best.num_samples() { config } else { best })
                .expect("at least one GL config")
        })
        .map_err(|e| e.to_string())?;

    let window = window.ok_or("no window was created")?;
    let raw_handle = window.window_handle().map_err(|e| e.to_string())?.as_raw();
    let gl_display = gl_config.display();

    let context_attributes = ContextAttributesBuilder::new().build(Some(raw_handle));
    let not_current = unsafe { gl_display.create_context(&gl_config, &context_attributes) }.map_err(|e| e.to_string())?;

    let surface_attributes = window
        .build_surface_attributes(SurfaceAttributesBuilder::new())
        .map_err(|e| e.to_string())?;
    let surface = unsafe { gl_display.create_window_surface(&gl_config, &surface_attributes) }.map_err(|e| e.to_string())?;

    let context = not_current.make_current(&surface).map_err(|e| e.to_string())?;

    let interval = if settings.vsync {
        SwapInterval::Wait(NonZeroU32::new(1).expect("1 is non-zero"))
    } else {
        SwapInterval::DontWait
    };
    // Not fatal: a driver refusing the requested interval just means a different frame pace, not a
    // broken canvas.
    let _ = surface.set_swap_interval(&context, interval);

    let renderer = OpenGl::new_from_glutin_display(&gl_display).map_err(|e| e.to_string())?;
    let mut canvas = Canvas::new(renderer).map_err(|e| e.to_string())?;
    let size = window.inner_size();
    canvas.set_size(size.width, size.height, window.scale_factor() as f32);

    Ok(Gfx { window, surface, context, canvas })
}

/// Concatenates `key`'s array from every provider of `contract`, in the order they appear,
/// which the engine guarantees is run order (see mirror_onto_contracts in process_module.rs).
///
/// This is the gathering half of the contract mechanism, and it's what stops this module being
/// something producers must funnel through one privileged "director" to reach. Any number of
/// modules can draw; each publishes its own list, and they land here concatenated. Since draw order
/// is list order, run order is z-order: a debug overlay that requires the module it annotates
/// therefore draws on top of it, for free, with nobody arranging that.
fn gather(msg: &Value, contract: &str, key: &str) -> Vec<Value> {
    let Some(providers) = msg.pointer(&format!("/shared/{contract}")).and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    providers
        .iter()
        .filter_map(|p| p.get(key).and_then(|v| v.as_array()))
        .flat_map(|list| list.iter().cloned())
        .collect()
}

// ---------- draw commands ----------

fn color_of(value: Option<&Value>, fallback: Color) -> Color {
    value.and_then(|v| v.as_str()).map(Color::hex).unwrap_or(fallback)
}

fn f32_of(cmd: &Value, key: &str, fallback: f32) -> f32 {
    cmd.get(key).and_then(Value::as_f64).map(|v| v as f32).unwrap_or(fallback)
}

/// Fills and/or strokes `path` according to whichever of "fill"/"stroke" the command carries. Shared
/// by every shape command, since that half is identical for all of them and only the path differs.
fn paint_path(canvas: &mut Canvas<OpenGl>, cmd: &Value, path: &Path) {
    if let Some(fill) = cmd.get("fill").and_then(|v| v.as_str()) {
        canvas.fill_path(path, &Paint::color(Color::hex(fill)));
    }
    if let Some(stroke) = cmd.get("stroke").and_then(|v| v.as_str()) {
        let paint = Paint::color(Color::hex(stroke)).with_line_width(f32_of(cmd, "strokeWidth", 1.0));
        canvas.stroke_path(path, &paint);
    }
}

/// One command. Unknown ops are logged and skipped rather than failing the frame: one bad command
/// from a director must not take down everything else being drawn, the same way audio treats one
/// unloadable sound file.
fn draw_command(app: &mut App, cmd: &Value, width: f32, height: f32) {
    let op = cmd.get("op").and_then(|v| v.as_str()).unwrap_or("");

    // Destructured rather than reached through `app.` field by field: the image/font caches, the
    // project root and the camera all need to be touched WHILE the canvas (which lives inside
    // app.gfx) is mutably borrowed, and only disjoint field borrows make that legal.
    let App { gfx, images, fonts, project_root, camera, .. } = app;
    let Some(gfx) = gfx.as_mut() else { return };
    let canvas = &mut gfx.canvas;

    match op {
        "clear" => {
            let color = color_of(cmd.get("color"), Color::black());
            canvas.clear_rect(0, 0, width as u32, height as u32, color);
        }
        "rect" => {
            let mut path = Path::new();
            let radius = f32_of(cmd, "radius", 0.0);
            let (x, y, w, h) = (f32_of(cmd, "x", 0.0), f32_of(cmd, "y", 0.0), f32_of(cmd, "w", 0.0), f32_of(cmd, "h", 0.0));
            if radius > 0.0 {
                path.rounded_rect(x, y, w, h, radius);
            } else {
                path.rect(x, y, w, h);
            }
            paint_path(canvas, cmd, &path);
        }
        "ellipse" => {
            let mut path = Path::new();
            path.ellipse(f32_of(cmd, "x", 0.0), f32_of(cmd, "y", 0.0), f32_of(cmd, "rx", 0.0), f32_of(cmd, "ry", 0.0));
            paint_path(canvas, cmd, &path);
        }
        "circle" => {
            let mut path = Path::new();
            path.circle(f32_of(cmd, "x", 0.0), f32_of(cmd, "y", 0.0), f32_of(cmd, "r", 0.0));
            paint_path(canvas, cmd, &path);
        }
        "line" => {
            let mut path = Path::new();
            path.move_to(f32_of(cmd, "x1", 0.0), f32_of(cmd, "y1", 0.0));
            path.line_to(f32_of(cmd, "x2", 0.0), f32_of(cmd, "y2", 0.0));
            let stroke = color_of(cmd.get("stroke"), Color::white());
            canvas.stroke_path(&path, &Paint::color(stroke).with_line_width(f32_of(cmd, "strokeWidth", 1.0)));
        }
        "path" => {
            let points: Vec<(f32, f32)> = cmd
                .get("points")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            let pair = p.as_array()?;
                            Some((pair.first()?.as_f64()? as f32, pair.get(1)?.as_f64()? as f32))
                        })
                        .collect()
                })
                .unwrap_or_default();
            if points.len() < 2 {
                return;
            }
            let mut path = Path::new();
            path.move_to(points[0].0, points[0].1);
            for (x, y) in &points[1..] {
                path.line_to(*x, *y);
            }
            if cmd.get("close").and_then(Value::as_bool).unwrap_or(false) {
                path.close();
            }
            paint_path(canvas, cmd, &path);
        }
        "image" => {
            let Some(file) = cmd.get("file").and_then(|v| v.as_str()) else { return };
            // Cached by path — decoding the file every frame is the trap this exists to avoid.
            let id = match images.get(file) {
                Some(id) => *id,
                None => {
                    let path = resolve_asset(project_root, file);
                    match canvas.load_image_file(&path, ImageFlags::empty()) {
                        Ok(id) => {
                            images.insert(file.to_string(), id);
                            id
                        }
                        Err(e) => {
                            log("error", &format!("vector-canvas could not load image {}: {e}", path.display()));
                            // Cached failures aren't worth the complexity; a missing file logs once
                            // per frame, which is noisy but self-announcing rather than silent.
                            return;
                        }
                    }
                }
            };
            let (nat_w, nat_h) = canvas.image_size(id).unwrap_or((0, 0));
            let x = f32_of(cmd, "x", 0.0);
            let y = f32_of(cmd, "y", 0.0);
            let w = f32_of(cmd, "w", nat_w as f32);
            let h = f32_of(cmd, "h", nat_h as f32);
            let angle = f32_of(cmd, "rotation", 0.0);
            let paint = match cmd.get("tint").and_then(|v| v.as_str()) {
                Some(tint) => Paint::image_tint(id, x, y, w, h, angle, Color::hex(tint)),
                None => Paint::image(id, x, y, w, h, angle, f32_of(cmd, "alpha", 1.0)),
            };
            let mut path = Path::new();
            path.rect(x, y, w, h);
            canvas.fill_path(&path, &paint);
        }
        "text" => {
            let Some(text) = cmd.get("text").and_then(|v| v.as_str()) else { return };
            let Some(font_file) = cmd.get("font").and_then(|v| v.as_str()) else {
                log("error", "vector-canvas: a text command needs a \"font\" file — there is no built-in font");
                return;
            };
            let font = match fonts.get(font_file) {
                Some(id) => *id,
                None => {
                    let path = resolve_asset(project_root, font_file);
                    match canvas.add_font(&path) {
                        Ok(id) => {
                            fonts.insert(font_file.to_string(), id);
                            id
                        }
                        Err(e) => {
                            log("error", &format!("vector-canvas could not load font {}: {e}", path.display()));
                            return;
                        }
                    }
                }
            };
            let align = match cmd.get("align").and_then(|v| v.as_str()).unwrap_or("left") {
                "center" => Align::Center,
                "right" => Align::Right,
                _ => Align::Left,
            };
            let paint = Paint::color(color_of(cmd.get("fill"), Color::white()))
                .with_font(&[font])
                .with_font_size(f32_of(cmd, "size", 16.0))
                .with_text_align(align);
            let _ = canvas.fill_text(f32_of(cmd, "x", 0.0), f32_of(cmd, "y", 0.0), text, &paint);
        }
        "push" => canvas.save(),
        "pop" => canvas.restore(),
        "translate" => canvas.translate(f32_of(cmd, "x", 0.0), f32_of(cmd, "y", 0.0)),
        "rotate" => canvas.rotate(f32_of(cmd, "angle", 0.0)),
        "scale" => canvas.scale(f32_of(cmd, "x", 1.0), f32_of(cmd, "y", 1.0)),
        "camera" => {
            *camera = Camera {
                x: f32_of(cmd, "x", 0.0),
                y: f32_of(cmd, "y", 0.0),
                zoom: f32_of(cmd, "zoom", 1.0),
                active: true,
            };
            camera.apply(canvas, width, height);
        }
        other => log("warn", &format!("vector-canvas: unknown draw op \"{other}\" — skipped")),
    }
}

/// Draws one frame's command list and presents it. Separated from the protocol handling above it so
/// the "what does a frame do" logic reads in one piece.
fn render_frame(app: &mut App, commands: &[Value]) {
    let Some(gfx) = app.gfx.as_ref() else { return };
    let size = gfx.window.inner_size();
    let (width, height) = (size.width as f32, size.height as f32);

    // Each frame starts from a known transform: whatever the last frame's push/translate/camera left
    // behind must not silently carry over into this one.
    let camera = app.camera;
    if let Some(gfx) = app.gfx.as_mut() {
        gfx.canvas.reset();
        camera.apply(&mut gfx.canvas, width, height);
    }

    for cmd in commands {
        draw_command(app, cmd, width, height);
    }

    if let Some(gfx) = app.gfx.as_mut() {
        gfx.canvas.flush();
        let _ = gfx.surface.swap_buffers(&gfx.context);
    }
}

fn publish_state(app: &mut App) -> Value {
    let (width, height) = app
        .gfx
        .as_ref()
        .map(|g| {
            let s = g.window.inner_size();
            (s.width, s.height)
        })
        .unwrap_or((0, 0));

    let (cx, cy) = app.camera.to_canvas(app.input.mouse_x, app.input.mouse_y, width as f32, height as f32);
    let keys: Vec<Value> = app.input.keys.iter().cloned().map(Value::String).collect();
    let state = json!({
        "width": width,
        "height": height,
        "mouse": {
            "x": cx, "y": cy,
            "windowX": app.input.mouse_x, "windowY": app.input.mouse_y,
            "left": app.input.left, "right": app.input.right, "middle": app.input.middle,
            "scroll": app.input.scroll, "inside": app.input.inside,
        },
        "keys": keys,
        "focused": app.input.focused,
        "closeRequested": app.input.close_requested,
    });
    // A wheel delta belongs to the frame it happened in, not to every frame after it.
    app.input.scroll = 0.0;
    state
}

// ---------- protocol driving ----------

enum Outcome {
    Continue,
    Stop,
}

/// The settings half of the "start" phase, without replying. Split out because the reply has to say
/// whether this module is running degraded, and only the caller knows: run_windowed has to pump the
/// event loop once to find out whether a window actually materialised, while run_headless already
/// knows it never will.
fn apply_start_settings(app: &mut App, msg: &Value) {
    app.settings = Some(msg.get("settings").cloned().and_then(|s| serde_json::from_value(s).ok()).unwrap_or_default());
}

fn is_start(msg: &Value) -> bool {
    msg.get("phase").and_then(|p| p.as_str()) == Some("start")
}

/// A "start" reply, carrying the degraded marker when there's no window. See ProcessModule::start
/// in process_module.rs for what the engine does with it.
fn reply_started(degraded: Option<&str>) {
    let mut extra = Map::new();
    if let Some(reason) = degraded {
        extra.insert("degraded".into(), json!(reason));
    }
    reply_ok(extra);
}

fn handle_message(app: &mut App, msg: &Value) -> Outcome {
    match msg.get("phase").and_then(|p| p.as_str()).unwrap_or("") {
        "compile" => {
            app.project_root = msg
                .get("sourcePath")
                .and_then(|p| p.as_str())
                .map(PathBuf::from)
                .and_then(|p| p.parent().map(|p| p.to_path_buf()));
            reply_ok(Map::new());
        }
        // Deliberately not handled here: the reply has to report whether a window was actually
        // obtained, and that isn't known until the event loop has been pumped once. Both run loops
        // handle it themselves; see apply_start_settings.
        "start" => reply_err("internal: start is handled by the run loop, not here"),
        "frame" => {
            let commands = gather(msg, "draw-commands", "draw");
            render_frame(app, &commands);

            let mut extra = Map::new();
            extra.insert("publish".into(), publish_state(app));
            reply_ok(extra);
        }
        "stop" => {
            reply_ok(Map::new());
            return Outcome::Stop;
        }
        other => reply_err(&format!("unknown phase \"{other}\"")),
    }
    Outcome::Continue
}

/// Reads stdin on its own thread so the main thread never blocks on it. That's what keeps the window
/// pumping while no frames are arriving, which is exactly what happens whenever the run is paused
/// at a debugger breakpoint, and without it the OS would mark the window "not responding" every
/// time you paused.
fn spawn_stdin_reader() -> Receiver<Value> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            if tx.send(msg).is_err() {
                break;
            }
        }
    });
    rx
}

fn run_windowed(mut event_loop: EventLoop<()>, rx: Receiver<Value>) {
    let mut app = App::new();
    let mut announced_close = false;

    loop {
        // Zero timeout: drain whatever the OS has queued and come straight back, rather than handing
        // this thread to winit the way run_app() would. A module's frame() has to return promptly.
        let _ = event_loop.pump_app_events(Some(Duration::ZERO), &mut app);

        // The window's own close button ends the run, the same way any module asking to stop does.
        // Announced once: the host sets a run-wide flag off this, so repeating it every frame would
        // be pure noise.
        if app.input.close_requested && !announced_close {
            announced_close = true;
            write_line(&json!({"requestStop": true}));
        }

        // Bounded wait rather than a spin: wakes immediately when the host sends something, and
        // still comes back often enough that the window stays responsive when it doesn't.
        match rx.recv_timeout(Duration::from_millis(4)) {
            Ok(msg) if is_start(&msg) => {
                // Settings first (the window's title and size come from them), then one pump to
                // actually build it, and only then the reply, which is the whole reason start is
                // handled here rather than in handle_message. Answering before the pump would mean
                // always claiming success, including on the machine where it just failed.
                apply_start_settings(&mut app, &msg);
                let _ = event_loop.pump_app_events(Some(Duration::ZERO), &mut app);
                reply_started(match app.gfx {
                    Some(_) => None,
                    None => Some("no window could be created on this machine — nothing will be drawn"),
                });
            }
            Ok(msg) => {
                if let Outcome::Stop = handle_message(&mut app, &msg) {
                    break;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// No display at all: a headless CI runner, a locked-down environment. The module still speaks the
/// protocol, still replies, and just never draws anything; the run continues rather than dying on a
/// machine that was never going to show a window. Same promise audio_playback_runtime.rs makes for a missing
/// audio device, and the reason that one exists is that it has genuinely broken CI here before.
fn run_headless(rx: Receiver<Value>, reason: &str) {
    let mut app = App::new();
    while let Ok(msg) = rx.recv() {
        if is_start(&msg) {
            // No pump to wait on here: this path already knows there will never be a window, so
            // the reply can say so outright.
            apply_start_settings(&mut app, &msg);
            reply_started(Some(&format!("no display is available ({reason}) — nothing will be drawn")));
            continue;
        }
        if let Outcome::Stop = handle_message(&mut app, &msg) {
            break;
        }
    }
}

fn main() {
    let rx = spawn_stdin_reader();
    // Same reasoning as ensure_window's own catch_unwind: creating an event loop can panic as well
    // as fail on a machine without a usable window system, and either way this module still owes
    // the engine a working protocol participant rather than a dead process.
    match std::panic::catch_unwind(EventLoop::new) {
        Ok(Ok(event_loop)) => run_windowed(event_loop, rx),
        Ok(Err(e)) => run_headless(rx, &e.to_string()),
        Err(_) => run_headless(rx, "creating an event loop panicked"),
    }
}
