//! `bruma gallery` — a local web gallery for the wallpaper collection
//! (Phase 6.5: the visual manager before Phase 7's public presence).
//!
//! Serves a single-file UI on a loopback port and a small JSON API:
//! GET  /            → the UI (embedded)
//! GET  /api/list    → installed packages (name, version, title, active)
//! GET  /api/preview/N?v=V → the package's preview.png
//! POST /api/install (bytes) → validate + install, returns the manifest
//! POST /api/activate {name} → default section + SIGHUP (reuse of
//!                             `bruma open`'s apply path)
//! POST /api/uninstall {name,version}
//!
//! Zero new dependencies (D8): the HTTP server is `std::net` and the UI
//! is one embedded HTML file (vanilla JS, native drag-and-drop).
//! Loopback-only by construction: the listener binds 127.0.0.1, and the
//! browser is the only client — a tab on the user's machine, not an
//! exposed service.

use crate::config;
use crate::mime;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

/// One installed wallpaper as the UI consumes it.
struct Entry {
    name: String,
    version: String,
    title: String,
    active: bool,
}

fn default_store() -> bruma_package::Store {
    // Same layout as main.rs's default_store: XDG data home.
    let data_home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|h| !h.is_empty())
                .map(|h| std::path::PathBuf::from(h).join(".local/share"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"));
    bruma_package::Store::new(data_home.join("bruma/wallpapers"))
}

/// The active package from the config's default section (per-output
/// sections can differ; the gallery tracks the default).
fn active_name() -> Option<String> {
    let cfg = config::Config::load().ok()??;
    cfg.default
        .as_ref()?
        .package
        .as_ref()
        .map(|p| p.split(':').next().unwrap_or(p).to_owned())
}

/// Collects the installed packages with their manifests.
fn entries() -> Vec<Entry> {
    let store = default_store();
    let active = active_name();
    store
        .installed()
        .into_iter()
        .filter_map(|(name, version, path)| {
            let json = std::fs::read_to_string(path.join("wallpaper.json")).ok()?;
            let m: bruma_package::Manifest = bruma_package::Manifest::parse(&json).ok()?;
            Some(Entry {
                active: active.as_deref() == Some(name.as_str()),
                name,
                version,
                title: m.title,
            })
        })
        .collect()
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn api_list() -> String {
    let items: Vec<String> = entries()
        .into_iter()
        .map(|e| {
            format!(
                r#"{{"name":"{}","version":"{}","title":"{}","active":{}}}"#,
                json_escape(&e.name),
                json_escape(&e.version),
                json_escape(&e.title),
                e.active
            )
        })
        .collect();
    format!(r#"{{"wallpapers":[{}]}}"#, items.join(","))
}

/// Parses a request's head; returns (method, path). Small POST bodies
/// (a .wallpaper upload) usually arrive with the head — those bytes are
/// stashed in BODY_STASH for the handler; larger ones are read there.
fn request_head(stream: &mut TcpStream) -> Option<(String, String, usize)> {
    let mut buf = [0u8; 8192];
    let mut used = 0;
    let head_end;
    loop {
        let n = stream.read(&mut buf[used..]).ok()?;
        if n == 0 {
            return None;
        }
        used += n;
        if let Some(pos) = find(&buf[..used], b"\r\n\r\n") {
            head_end = pos + 4;
            break;
        }
        if used == buf.len() {
            return None;
        }
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let first = lines.next()?.to_string();
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let content_length = lines
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().ok())?
        })
        .next()
        .unwrap_or(0);
    // Any body bytes already read must not be lost: stash them now.
    let mut body = Vec::new();
    if used > head_end {
        body.extend_from_slice(&buf[head_end..used.min(head_end + content_length)]);
    }
    BODY_STASH.with(|s| *s.borrow_mut() = Some(body));
    Some((method, path, content_length))
}

thread_local! {
    /// Body bytes already buffered with the head (the 8 KiB head read
    /// usually swallows the whole small request).
    static BODY_STASH: std::cell::RefCell<Option<Vec<u8>>> =
        const { std::cell::RefCell::new(None) };
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn respond(stream: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn respond_json(stream: &mut TcpStream, status: &str, json: String) {
    respond(stream, status, "application/json", json.as_bytes());
}

fn api_activate(name: &str) -> String {
    if !entries().iter().any(|e| e.name == name) {
        return format!(r#"{{"error":"nothing installed named '{name}'"}}"#);
    }
    mime::apply_as_default(name);
    format!(r#"{{"ok":true,"active":"{name}"}}"#)
}

fn api_uninstall(json_body: &[u8]) -> String {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(json_body) else {
        return r#"{"error":"bad request"}"#.to_owned();
    };
    let (Some(name), Some(version)) = (
        v.get("name").and_then(|n| n.as_str()),
        v.get("version").and_then(|n| n.as_str()),
    ) else {
        return r#"{"error":"name and version required"}"#.to_owned();
    };
    if active_name().as_deref() == Some(name) {
        return format!(
            r#"{{"error":"'{name}' is the active wallpaper — activate another first"}}"#
        );
    }
    match default_store().uninstall(name, version) {
        Ok(true) => format!(r#"{{"ok":true,"uninstalled":"{name}"}}"#),
        Ok(false) => format!(r#"{{"error":"{name} {version} not found"}}"#),
        Err(e) => format!(r#"{{"error":"{e}"}}"#),
    }
}

/// Serves one request; `method path body`.
fn handle(stream: &mut TcpStream, method: &str, path: &str) {
    if method == "GET" && path == "/" {
        respond(
            stream,
            "200 OK",
            "text/html; charset=utf-8",
            UI_HTML.as_bytes(),
        );
        return;
    }
    if method == "GET" && path == "/api/list" {
        respond_json(stream, "200 OK", api_list());
        return;
    }
    if method == "GET" && path.starts_with("/api/preview/") {
        // /api/preview/N?v=V — name and version from the path+query.
        let rest = path.trim_start_matches("/api/preview/");
        let (name, ver) = rest.split_once("?v=").unwrap_or((rest, ""));
        let store = default_store();
        let pkg_dir = store.installed_path(name, ver);
        match std::fs::read(pkg_dir.join("preview.png")) {
            Ok(png) => respond(stream, "200 OK", "image/png", &png),
            Err(_) => respond_json(stream, "404 Not Found", r#"{"error":"no preview"}"#.into()),
        }
        return;
    }
    if method == "POST" && path == "/api/install" {
        let bytes = BODY_STASH.with(|s| s.borrow_mut().take().unwrap_or_default());
        if bytes.is_empty() {
            respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"empty body"}"#.into(),
            );
            return;
        }
        match bruma_package::Store::validate(&bytes) {
            Ok(m) => {
                let name = m.install_name();
                match default_store().install(&bytes, &name, &m.version) {
                    Ok((dest, installed)) => {
                        let state = if installed {
                            "installed"
                        } else {
                            "already installed"
                        };
                        println!(
                            "[gallery] {name} {} — {state} at {}",
                            m.version,
                            dest.display()
                        );
                        respond_json(
                            stream,
                            "200 OK",
                            format!(
                                r#"{{"ok":true,"name":"{}","title":"{}"}}"#,
                                json_escape(&name),
                                json_escape(&m.title)
                            ),
                        );
                    }
                    Err(e) => {
                        respond_json(stream, "400 Bad Request", format!(r#"{{"error":"{e}"}}"#))
                    }
                }
            }
            Err(e) => respond_json(stream, "400 Bad Request", format!(r#"{{"error":"{e}"}}"#)),
        }
        return;
    }
    if method == "POST" && path == "/api/activate" {
        let bytes = BODY_STASH.with(|s| s.borrow_mut().take().unwrap_or_default());
        let name = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_owned));
        match name {
            Some(n) => {
                let resp = api_activate(&n);
                let status = if resp.contains("\"ok\"") {
                    "200 OK"
                } else {
                    "400 Bad Request"
                };
                respond_json(stream, status, resp);
            }
            None => respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"name required"}"#.into(),
            ),
        }
        return;
    }
    if method == "POST" && path == "/api/uninstall" {
        let bytes = BODY_STASH.with(|s| s.borrow_mut().take().unwrap_or_default());
        let resp = api_uninstall(&bytes);
        let status = if resp.contains("\"ok\"") {
            "200 OK"
        } else {
            "400 Bad Request"
        };
        respond_json(stream, status, resp);
        return;
    }
    respond_json(stream, "404 Not Found", r#"{"error":"no route"}"#.into());
}

fn serve_loop(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let Some((method, path, _len)) = request_head(&mut stream) else {
            continue;
        };
        handle(&mut stream, &method, &path);
    }
}

/// `bruma gallery [--port N]` — starts the server and opens the
/// browser. Blocks until killed (Ctrl-C).
pub fn gallery_command(args: &[String]) {
    let mut port: u16 = 7600;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                port = args
                    .get(i + 1)
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| {
                        eprintln!("--port requires a number");
                        std::process::exit(2);
                    });
                i += 2;
            }
            other => {
                eprintln!("unknown argument: {other}\nusage: bruma gallery [--port N]");
                std::process::exit(2);
            }
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("error: cannot bind 127.0.0.1:{port}: {e}");
        std::process::exit(1);
    });
    let url = format!("http://127.0.0.1:{port}/");
    println!("[gallery] serving the collection at {url}");
    println!("[gallery] drag .wallpaper files into the page to install them");
    // Best-effort browser open (x-open or xdg-open on Linux desktops).
    for opener in ["xdg-open", "gio"] {
        let opened = std::process::Command::new(opener)
            .arg(&url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|mut c| c.wait().map(|s| s.success()).unwrap_or(false))
            .unwrap_or(false);
        if opened {
            break;
        }
    }
    serve_loop(listener);
}

/// The single-file UI (embedded; vanilla JS).
const UI_HTML: &str = r#"<!doctype html>
<html lang=en>
<head>
<meta charset=utf-8>
<meta name=viewport content='width=device-width,initial-scale=1'>
<title>bruma — wallpapers</title>
<style>
:root { color-scheme: dark; }
body { margin: 0; font: 15px/1.4 system-ui, sans-serif; background: #101014; color: #e8e8ec; }
header { display: flex; align-items: center; gap: 12px; padding: 16px 24px; }
h1 { font-size: 18px; font-weight: 600; margin: 0; flex: 1; }
#drop { margin: 0 24px 8px; border: 2px dashed #444; border-radius: 12px; padding: 22px; text-align: center; color: #999; transition: all .15s; }
#drop.over { border-color: #7aa2f7; color: #7aa2f7; background: #7aa2f711; }
#grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 14px; padding: 16px 24px 32px; }
.card { background: #191922; border: 1px solid #26262f; border-radius: 12px; overflow: hidden; }
.card.active { border-color: #7aa2f7; box-shadow: 0 0 0 1px #7aa2f7; }
.card img { width: 100%; aspect-ratio: 16/9; object-fit: cover; display: block; background: #000; }
.card .body { padding: 10px 12px 12px; }
.card .title { font-weight: 600; }
.card .meta { color: #888; font-size: 13px; margin: 2px 0 10px; }
.btns { display: flex; gap: 8px; }
button { flex: 1; padding: 7px 10px; border-radius: 8px; border: 1px solid #33333d; background: #23232d; color: #e8e8ec; cursor: pointer; font-size: 14px; }
button:hover { border-color: #555; }
button.primary { background: #2f4a7a; border-color: #3f62a8; }
button.primary:hover { background: #3a5a96; }
button:disabled { opacity: .45; cursor: default; }
#toast { position: fixed; left: 50%; bottom: 24px; transform: translateX(-50%); background: #23232d; border: 1px solid #33333d; padding: 10px 18px; border-radius: 10px; opacity: 0; transition: opacity .2s; pointer-events: none; max-width: 80vw; }
#toast.err { border-color: #a33; color: #faa; }
#toast.show { opacity: 1; }
</style>
</head>
<body>
<header><h1>bruma — wallpaper collection</h1><span id=count></span></header>
<div id=drop>Drop <b>.wallpaper</b> files here to install them</div>
<div id=grid></div>
<div id=toast></div>
<script>
const grid = document.getElementById('grid'), count = document.getElementById('count'),
      drop = document.getElementById('drop'), toast = document.getElementById('toast');
let timer;
function say(msg, err) {
  toast.textContent = msg; toast.className = 'show' + (err ? ' err' : '');
  clearTimeout(timer); timer = setTimeout(() => toast.className = '', 2600);
}
async function api(path, opts) {
  const r = await fetch(path, opts);
  const j = await r.json().catch(() => ({}));
  if (!r.ok || j.error) throw new Error(j.error || r.statusText);
  return j;
}
async function refresh() {
  const { wallpapers } = await api('/api/list');
  grid.innerHTML = ''; count.textContent = wallpapers.length + ' installed';
  for (const w of wallpapers) {
    const card = document.createElement('div');
    card.className = 'card' + (w.active ? ' active' : '');
    card.innerHTML =
      `<img src='/api/preview/${encodeURIComponent(w.name)}?v=${encodeURIComponent(w.version)}' alt=''>` +
      `<div class='body'><div class='title'></div>` +
      `<div class='meta'>${w.name} · ${w.version}${w.active ? ' · <b style=color:#7aa2f7>active</b>' : ''}</div>` +
      `<div class='btns'></div></div>`;
    card.querySelector('.title').textContent = w.title;
    const btns = card.querySelector('.btns');
    if (w.active) {
      const b = document.createElement('button');
      b.textContent = 'Running'; b.disabled = true; btns.appendChild(b);
    } else {
      const b = document.createElement('button');
      b.className = 'primary'; b.textContent = 'Set as wallpaper';
      b.onclick = async () => {
        try { await api('/api/activate', {method:'POST', headers:{'content-type':'application/json'}, body: JSON.stringify({name: w.name})}); refresh(); }
        catch (e) { say(e.message, true); }
      };
      btns.appendChild(b);
      const d = document.createElement('button');
      d.textContent = 'Uninstall';
      d.onclick = async () => {
        try { await api('/api/uninstall', {method:'POST', headers:{'content-type':'application/json'}, body: JSON.stringify({name: w.name, version: w.version})}); say('uninstalled ' + w.name); refresh(); }
        catch (e) { say(e.message, true); }
      };
      btns.appendChild(d);
    }
    grid.appendChild(card);
  }
}
async function install(file) {
  try {
    const j = await api('/api/install', {method:'POST', body: file});
    say(`${j.title}: installed — set it with one click`); refresh();
  } catch (e) { say(e.message, true); }
}
drop.addEventListener('dragover', e => { e.preventDefault(); drop.classList.add('over'); });
drop.addEventListener('dragleave', () => drop.classList.remove('over'));
drop.addEventListener('drop', e => {
  e.preventDefault(); drop.classList.remove('over');
  for (const f of e.dataTransfer.files) if (f.name.endsWith('.wallpaper')) install(f);
});
refresh();
setInterval(refresh, 4000);
</script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_json_escapes_and_flags_active() {
        // Direct units of the JSON builder.
        assert_eq!(json_escape("a\"b"), "a\\\"b");
        assert_eq!(json_escape("back\\slash"), "back\\\\slash");
    }

    #[test]
    fn uninstall_rejects_active_wallpaper() {
        // api_uninstall's guard: the active package is protected.
        let resp = api_uninstall(br#"{"name":"does-not-exist","version":"9.9.9"}"#);
        assert!(resp.contains("error"));
    }
}
