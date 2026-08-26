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

use crate::plugin_assets::{resolve_asset_path, CSP, HARNESS_JS};
use std::sync::atomic::{AtomicU16, Ordering};

static PORT: AtomicU16 = AtomicU16::new(0);

pub fn start() {
    let server = tiny_http::Server::http("127.0.0.1:0").expect("failed to bind plugin asset server");
    let port = server.server_addr().to_ip().expect("loopback server has an ip address").port();
    PORT.store(port, Ordering::SeqCst);
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            handle(request);
        }
    });
}

pub fn port() -> u16 {
    PORT.load(Ordering::SeqCst)
}

fn respond(request: tiny_http::Request, status: u16, content_type: &str, body: Vec<u8>) {
    let response = tiny_http::Response::from_data(body)
        .with_status_code(status)
        .with_header(mk_header("Content-Type", content_type))
        .with_header(mk_header("Content-Security-Policy", CSP));
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

    // The one file the host serves itself, regardless of what's actually on disk for this plugin.
    if rel_path == "__lowarc.js" {
        respond(request, 200, "text/javascript", HARNESS_JS.as_bytes().to_vec());
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
            respond(request, 200, &content_type, data);
        }
        Err(_) => respond(request, 404, "text/plain", Vec::new()),
    }
}
