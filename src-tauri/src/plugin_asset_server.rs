// Serves each installed plugin's static UI assets over plain loopback HTTP, so a panel's content
// loads as an ordinary cross-origin document in a sandboxed iframe. See plugin_assets.rs for why
// this exists instead of a Tauri custom URI scheme (`plugin://...`): on Windows/WebView2, a
// sub-frame navigation to a custom scheme silently never reaches the registered handler.
//
// Bound to an OS-assigned ephemeral port on 127.0.0.1 only — never reachable off the local
// machine. The frontend learns the port once, at startup, via the `plugin_asset_port` command
// (lib.rs), and builds panel/viewer iframe URLs as `http://127.0.0.1:<port>/<plugin_id>/<path>`.
//
// Different plugin_ids all share this one origin (only the path differs), which would normally
// let one plugin's script reach into another's iframe — but every panel/viewer iframe is
// `sandbox="allow-scripts"` with no `allow-same-origin`, so each gets its own opaque origin
// regardless of the URL's real origin. Isolation comes from the sandbox attribute, not from this
// server, so nothing extra is needed here for that.

use crate::plugin_assets::{
    resolve_asset_path, CSP, HARNESS_JS, SHARED_DROPDOWN_JS, SHARED_FORMAT_JS, SHARED_ICONS_CSS, SHARED_ICONS_JS,
    SHARED_ICONS_WOFF, SHARED_PRIMITIVES_CSS, SHARED_STYLE_CSS,
};
use crate::{settings, theme};
use std::sync::atomic::{AtomicU16, Ordering};

static PORT: AtomicU16 = AtomicU16::new(0);

pub fn start() {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("failed to bind plugin asset server");
    let port = server.server_addr().to_ip().expect("loopback server has an ip address").port();
    PORT.store(port, Ordering::SeqCst);
    std::thread::spawn(move || {
        // A thread per request rather than one loop serving them in turn. Monaco alone pulls dozens
        // of chunk files the instant its iframe navigates, and the browser fires most concurrently;
        // behind a single thread each one queues on whatever read is mid-flight, which is a
        // multi-second stall on a file's first open. Every request here is a stateless disk read
        // with no shared mutable state, so unbounded concurrency costs only a thread apiece.
        for request in server.incoming_requests() {
            std::thread::spawn(move || handle(request));
        }
    });
}

pub fn port() -> u16 {
    PORT.load(Ordering::SeqCst)
}

fn respond(request: tiny_http::Request, status: u16, content_type: &str, body: Vec<u8>) {
    respond_cacheable(request, status, content_type, body, false);
}

// cacheable is what a plain `respond()` skips (a 404, or content computed fresh per-request like
// __lowarc-theme.css) — everything a plugin actually renders with is genuinely static for the
// life of one running app, and Monaco alone is dozens of separate chunk files an iframe re-fetches
// in full on every single mount otherwise — reopening a file, or opening a second instance for
// split view, was paying that same multi-second cost again with nothing to show for it. Kept
// short (not the usual immutable-forever a content-hashed bundle would get) specifically because
// this app's own plugins (Monaco's own harness aside) are actively edited during development —
// unhashed filenames mean a stale cache would otherwise hide a just-saved change for however long
// the lifetime is; a minute is enough to absorb rapid reopens/split-toggles in one sitting without
// meaningfully getting in the way of an edit-reload loop.
fn respond_cacheable(request: tiny_http::Request, status: u16, content_type: &str, body: Vec<u8>, cacheable: bool) {
    let mut response = tiny_http::Response::from_data(body)
        .with_status_code(status)
        .with_header(mk_header("Content-Type", content_type))
        .with_header(mk_header("Content-Security-Policy", CSP))
        // Every plugin iframe is sandbox="allow-scripts" with no allow-same-origin, so it has an
        // opaque origin — the browser can never consider a request FROM it "same-origin" with
        // anything, including this literal server, no matter how the URL looks. Most resource
        // types (scripts, stylesheets, plain images) don't enforce CORS for that anyway, but
        // @font-face specifically does, so a font load from a sandboxed iframe fails with a bare
        // NetworkError unless the response carries this. Loopback-only, no real user data ever
        // flows through it, so a blanket allow-all costs nothing.
        .with_header(mk_header("Access-Control-Allow-Origin", "*"));
    if cacheable {
        response = response.with_header(mk_header("Cache-Control", "max-age=60"));
    }
    let _ = request.respond(response);
}

fn mk_header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header is valid")
}

fn handle(request: tiny_http::Request) {
    // request.url() is path + query string — never the fragment (fragments are never sent to a
    // server), which is exactly what the frontend relies on to pass project/file context without
    // it reaching this handler at all.
    let path = request.url().trim_start_matches('/').to_string();
    let mut parts = path.splitn(2, '/');
    let plugin_id = parts.next().unwrap_or_default();
    let rel_path = parts.next().unwrap_or_default();

    // Files the host serves itself, regardless of what's actually on disk for this plugin — the
    // harness script every plugin loads unconditionally, plus two opt-in stylesheets (see
    // plugin_assets.rs's SHARED_STYLE_CSS/SHARED_PRIMITIVES_CSS) a plugin can link if it wants to
    // look like the IDE.
    if rel_path == "__lowarc.js" {
        respond_cacheable(request, 200, "text/javascript", HARNESS_JS.as_bytes().to_vec(), true);
        return;
    }
    if rel_path == "__lowarc.css" {
        respond_cacheable(request, 200, "text/css", SHARED_STYLE_CSS.as_bytes().to_vec(), true);
        return;
    }
    if rel_path == "__lowarc-primitives.css" {
        respond_cacheable(request, 200, "text/css", SHARED_PRIMITIVES_CSS.as_bytes().to_vec(), true);
        return;
    }
    if rel_path == "__lowarc-icons.css" {
        respond_cacheable(request, 200, "text/css", SHARED_ICONS_CSS.as_bytes().to_vec(), true);
        return;
    }
    if rel_path == "__lowarc-icons.js" {
        respond_cacheable(request, 200, "text/javascript", SHARED_ICONS_JS.as_bytes().to_vec(), true);
        return;
    }
    // "seti.woff", not "__lowarc-icons.woff" — icons.css references it by its real vendored
    // filename (see that file's own comment for why), so this is the relative path a plugin
    // iframe's browser actually requests after loading __lowarc-icons.css at .../<plugin_id>/.
    if rel_path == "seti.woff" {
        respond_cacheable(request, 200, "font/woff", SHARED_ICONS_WOFF.to_vec(), true);
        return;
    }
    if rel_path == "__lowarc-dropdown.js" {
        respond_cacheable(request, 200, "text/javascript", SHARED_DROPDOWN_JS.as_bytes().to_vec(), true);
        return;
    }
    if rel_path == "__lowarc-format.js" {
        respond_cacheable(request, 200, "text/javascript", SHARED_FORMAT_JS.as_bytes().to_vec(), true);
        return;
    }
    // Computed fresh every request (settings::load() reads settings.json from disk each time, not
    // a cached value) — a plugin's iframe never re-fetches this on its own just because the user
    // changed Appearance elsewhere, but at least a freshly-mounted or reloaded panel always gets
    // whatever's current, rather than whatever was active the moment the app happened to launch.
    // NOT cacheable, unlike everything else here — the whole point is that it can change.
    if rel_path == "__lowarc-theme.css" {
        let css = theme::resolved_css(&settings::load().theme_mode);
        respond(request, 200, "text/css", css.into_bytes());
        return;
    }

    let Some(resolved) = resolve_asset_path(plugin_id, rel_path) else {
        respond(request, 404, "text/plain", Vec::new());
        return;
    };

    match std::fs::read(&resolved) {
        Ok(data) => {
            let mime = mime_guess::from_path(&resolved).first_or_octet_stream();
            let content_type = mime.essence_str().to_string();
            respond_cacheable(request, 200, &content_type, data, true);
        }
        Err(_) => respond(request, 404, "text/plain", Vec::new()),
    }
}
