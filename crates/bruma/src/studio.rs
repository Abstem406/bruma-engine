//! `bruma studio` — the visual wallpaper creator (Phase A of the web
//! implementation decision).
//!
//! The key insight: the running daemon IS the studio's render engine.
//! The studio edits a package in place (shader source, params) and the
//! daemon hot-reloads it (shader by mtime, params by SIGHUP) — the
//! REAL wallpaper behind the windows is the live preview. A second
//! monitor makes it a two-screen rig; one screen still works (edit,
//! glance at the desktop).
//!
//! Endpoints (loopback-only, same pattern as `bruma gallery`):
//! GET  /                      → the UI (embedded)
//! GET  /api/packages          → installed packages
//! POST /api/create            → scaffold from a template (reuse of
//!                               `bruma new`'s machinery)
//! GET  /api/shader?name=N     → the package's entry shader source
//! PUT  /api/shader?name=N     → replace it (naga validates first: a
//!                               broken shader never lands; the daemon
//!                               picks the file up by mtime)
//! GET  /api/manifest?name=N   → the manifest JSON (params identity)
//! GET  /api/params?name=N     → effective values (defaults + overrides)
//! POST /api/params            → set one param (persist + SIGHUP, the
//!                               `bruma params set` path)
//!
//! Phase B (WebGPU canvas preview) will extend this UI; the API shape
//! already serves it (shader + manifest by GET).

use crate::config;
use bruma_package::Store;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

fn default_store() -> Store {
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
    Store::new(data_home.join("bruma/wallpapers"))
}

/// The latest installed version's manifest + dir for `name` (same
/// lookup as `bruma params`).
fn manifest_for(name: &str) -> Option<(bruma_package::Manifest, std::path::PathBuf)> {
    let store = default_store();
    let mut best: Option<(String, std::path::PathBuf)> = None;
    for (pkg, ver, dir) in store.installed() {
        if pkg == name
            && best
                .as_ref()
                .is_none_or(|(bv, _)| ver.as_str() > bv.as_str())
        {
            best = Some((ver, dir));
        }
    }
    let (_, dir) = best?;
    let json = std::fs::read_to_string(dir.join("wallpaper.json")).ok()?;
    bruma_package::Manifest::parse(&json).ok().map(|m| (m, dir))
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn api_packages() -> String {
    let store = default_store();
    let items: Vec<String> = store
        .installed()
        .into_iter()
        .map(|(name, version, _)| {
            let title = manifest_for(&name)
                .map(|(m, _)| m.title)
                .unwrap_or_else(|| name.clone());
            format!(
                r#"{{"name":"{}","version":"{}","title":"{}"}}"#,
                json_escape(&name),
                json_escape(&version),
                json_escape(&title)
            )
        })
        .collect();
    format!(r#"{{"packages":[{}]}}"#, items.join(","))
}

/// A cheap distinguishable token for temp dir names (names are
/// install-slugged already, but keep the pid in front anyway).
fn temp_token(name: &str) -> u64 {
    name.bytes()
        .fold(0xcbf29ce4u64, |h, b| (h ^ b as u64) * 0x100000001b3)
}

fn api_create(body: &[u8]) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "bad JSON".to_owned())?;
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("name required")?;
    let template = v
        .get("template")
        .and_then(|t| t.as_str())
        .unwrap_or("waves"); // Same machinery as `bruma new`: scaffold + real-parser self-check,
    // then straight into the store (no intermediate zip file left).
    // `new::create` scaffolds INTO a subdir named after the package, so
    // the pack root is dir/name.
    let dir = std::env::temp_dir().join(format!(
        "bruma-studio-{}-{:x}",
        std::process::id(),
        temp_token(name)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    crate::new::create(name, template, Some(&dir))?;
    let scaffold = dir.join(name);
    let zip = Store::pack(&scaffold, None).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&zip).map_err(|e| e.to_string())?;
    let manifest = Store::validate(&bytes).map_err(|e| e.to_string())?;
    let install_name = manifest.install_name();
    let title = manifest.title.clone();
    default_store()
        .install(&bytes, &install_name, &manifest.version)
        .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_file(&zip);
    println!(
        "[studio] {install_name} {} — created from template '{template}'",
        manifest.version
    );
    Ok(format!(
        r#"{{"ok":true,"name":"{}","title":"{}"}}"#,
        json_escape(&install_name),
        json_escape(&title)
    ))
}

fn api_shader_get(name: &str) -> Result<Vec<u8>, (String, &'static str)> {
    let (manifest, dir) =
        manifest_for(name).ok_or_else(|| (format!("'{name}' is not installed"), "404"))?;
    std::fs::read(dir.join(&manifest.entry))
        .map_err(|e| (format!("cannot read shader: {e}"), "500"))
}

/// Validates with naga BEFORE writing: a broken shader never lands (the
/// daemon's hot-reload would reject it anyway, but the creator gets the
/// error at edit time with line numbers). The source is assembled as
/// the engine assembles it — prelude (texture fits) + creator code —
/// so the injected helpers the shader calls are in scope.
fn naga_error(creator_source: &str) -> Option<String> {
    let source = bruma_renderer_wgpu::with_prelude(
        creator_source,
        &[bruma_renderer_wgpu::TextureFit::Cover; bruma_renderer_wgpu::TEXTURE_SLOTS],
    );
    let module = match naga::front::wgsl::parse_str(&source) {
        Ok(m) => m,
        Err(e) => return Some(e.emit_to_string(&source)),
    };
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    );
    validator.validate(&module).err().map(|e| e.to_string())
}

fn api_shader_put(name: &str, source: &[u8]) -> Result<String, (String, &'static str)> {
    let (manifest, dir) =
        manifest_for(name).ok_or_else(|| (format!("'{name}' is not installed"), "404"))?;
    let src =
        std::str::from_utf8(source).map_err(|_| ("shader must be UTF-8".to_owned(), "400"))?;
    if let Some(err) = naga_error(src) {
        return Err((err, "400"));
    }
    let path = dir.join(&manifest.entry);
    std::fs::write(&path, src).map_err(|e| (format!("cannot write: {e}"), "500"))?;
    println!("[studio] {name} — shader updated ({})", path.display());
    Ok(r#"{"ok":true}"#.into())
}

fn api_activate(body: &[u8]) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "bad JSON".to_owned())?;
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or("name required")?;
    if !manifest_for(name).is_some() {
        return Err(format!("'{name}' is not installed"));
    }
    crate::mime::apply_as_default(name);
    Ok(format!(r#"{{"ok":true,"active":"{name}"}}"#))
}

fn api_uninstall(body: &[u8]) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "bad JSON".to_owned())?;
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or("name required")?;
    let version = v
        .get("version")
        .and_then(|n| n.as_str())
        .ok_or("version required")?;
    if manifest_for(name).is_none() {
        return Err(format!("'{name}' is not installed"));
    }
    // The studio has no view of which package is active; the same rule
    // as the gallery: removing the running one leaves the daemon with a
    // dead path. Refuse; the creator activates another first.
    let active = config::Config::load()
        .ok()
        .flatten()
        .and_then(|c| c.default)
        .and_then(|d| d.package)
        .map(|p| p.split(':').next().unwrap_or(&p).to_owned());
    if active.as_deref() == Some(name) {
        return Err(format!(
            "'{name}' is the active wallpaper — activate another first"
        ));
    }
    match default_store().uninstall(name, version) {
        Ok(true) => Ok(format!(r#"{{"ok":true,"uninstalled":"{name}"}}"#)),
        Ok(false) => Err(format!("{name} {version} not found")),
        Err(e) => Err(e.to_string()),
    }
}

fn api_preview_get(name: &str) -> Result<Vec<u8>, (String, &'static str)> {
    let (_, dir) =
        manifest_for(name).ok_or_else(|| (format!("'{name}' is not installed"), "404"))?;
    std::fs::read(dir.join("preview.png")).map_err(|e| (format!("no preview: {e}"), "404"))
}

fn api_preview_put(name: &str, bytes: &[u8]) -> Result<String, (String, &'static str)> {
    let (_, dir) =
        manifest_for(name).ok_or_else(|| (format!("'{name}' is not installed"), "404"))?;
    if image::guess_format(bytes).is_err() {
        return Err(("preview must be a png/jpg image".into(), "400"));
    }
    std::fs::write(dir.join("preview.png"), bytes)
        .map_err(|e| (format!("cannot write: {e}"), "500"))?;
    println!("[studio] {name} — preview updated");
    Ok(r#"{"ok":true}"#.into())
}

fn api_textures_get(name: &str) -> Result<String, (String, &'static str)> {
    let (manifest, dir) =
        manifest_for(name).ok_or_else(|| (format!("'{name}' is not installed"), "404"))?;
    let items: Vec<String> = manifest
        .textures
        .iter()
        .map(|t| {
            let dims = image::image_dimensions(dir.join(&t.path))
                .map(|(w, h)| format!(r#""w":{w},"h":{h},"#))
                .unwrap_or_default();
            format!(
                r#"{{"path":"{}","fit":"{}",{}"url":"/api/texture?name={}&i={}"}}"#,
                json_escape(&t.path),
                match t.fit {
                    bruma_package::TextureFit::Cover => "cover",
                    bruma_package::TextureFit::Contain => "contain",
                },
                dims,
                json_escape(name),
                t.path.trim_start_matches("assets/")
            )
        })
        .collect();
    Ok(format!(r#"{{"textures":[{}]}}"#, items.join(",")))
}

/// Replaces (or adds) the texture at `slot` with the uploaded image.
/// The manifest entry is created/updated with the requested fit and
/// the file lands at `assets/photo<ext>` (ext sniffed from the bytes).
fn api_texture_put(name: &str, fit: &str, bytes: &[u8]) -> Result<String, (String, &'static str)> {
    let fit = match fit {
        "cover" => bruma_package::TextureFit::Cover,
        "contain" => bruma_package::TextureFit::Contain,
        _ => return Err(("fit must be cover|contain".into(), "400")),
    };
    let (mut manifest, dir) =
        manifest_for(name).ok_or_else(|| (format!("'{name}' is not installed"), "404"))?;
    let format = image::guess_format(bytes)
        .map_err(|_| ("upload must be a png/jpg image".to_owned(), "400"))?;
    let ext = match format {
        image::ImageFormat::Jpeg => "jpg",
        _ => "png",
    };
    let rel = format!("assets/photo.{ext}");
    let assets = dir.join("assets");
    std::fs::create_dir_all(&assets).map_err(|e| (format!("cannot create assets/: {e}"), "500"))?;
    std::fs::write(dir.join(&rel), bytes).map_err(|e| (format!("cannot write: {e}"), "500"))?;
    // Manifest update: slot 0 replaced (the engine's texture slots are
    // fixed; the studio manages the first as "the photo").
    manifest.textures.clear();
    manifest.textures.push(bruma_package::TextureSpec {
        path: rel.clone(),
        fit,
    });
    write_manifest(&dir, &manifest)?;
    // Permissions: a texture almost always rides with feedback+mouse
    // (water) — add them so the re-installed engine arms the features.
    for perm in ["feedback", "mouse"] {
        if !manifest.permissions.iter().any(|p| p == perm) {
            manifest.permissions.push(perm.to_owned());
        }
    }
    write_manifest(&dir, &manifest)?;
    println!("[studio] {name} — texture {rel} ({fit:?})");
    Ok(format!(r#"{{"ok":true,"path":"{rel}"}}"#))
}

/// Writes the manifest back to the installed package (pretty, with the
/// same field names the parser accepts).
fn write_manifest(
    dir: &std::path::Path,
    manifest: &bruma_package::Manifest,
) -> Result<(), (String, &'static str)> {
    let json = serde_json::to_string_pretty(manifest).map_err(|e| (e.to_string(), "500"))? + "\n";
    std::fs::write(dir.join("wallpaper.json"), json)
        .map_err(|e| (format!("cannot write manifest: {e}"), "500"))
}

/// COMPOSE (the layers v1): a user photo + an effect template over it.
/// Reuses the engine's own composable pair: `water-cursor` (water over
/// the photo) or `water-photo`/`parallax`/etc. — the template's texture
/// slot IS the layer under the effect. The chosen template's code
/// becomes the package's shader and the photo lands in its slot.
fn api_compose(body: &[u8]) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "bad JSON".to_owned())?;
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or("name required")?;
    let effect = v
        .get("effect")
        .and_then(|e| e.as_str())
        .unwrap_or("water-cursor");
    let fit = v.get("fit").and_then(|f| f.as_str()).unwrap_or("cover");
    let photo = v
        .get("photo_b64")
        .and_then(|p| p.as_str())
        .ok_or("photo_b64 required (the image, base64)")?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(photo)
        .map_err(|_| "photo_b64 is not valid base64".to_owned())?;
    // 1. The effect template's source becomes the package shader.
    let source =
        crate::new::template_source(effect).ok_or_else(|| format!("unknown effect '{effect}'"))?;
    // Does the template actually sample a texture? (Only the photo
    // families declare `textures`; parallax/fog/waves/trail are pure
    // math.) Composing one of those over a photo is legal but the photo
    // would be INVISIBLE — take it from the manifest and say so.
    let uses_texture = source.contains("tex0");
    api_shader_put(name, source.as_bytes()).map_err(|(e, _)| e)?;
    // 2. The template's identity replaces the manifest's params and
    // permissions, preserving the package title/name. The photo goes to
    // the texture slot ONLY for the photo families.
    let (mut manifest, dir) =
        manifest_for(name).ok_or_else(|| format!("'{name}' is not installed"))?;
    let fresh_json = crate::new::manifest_json(&manifest.title, effect)
        .ok_or_else(|| format!("unknown effect '{effect}'"))?;
    let mut fresh = bruma_package::Manifest::parse(&fresh_json)
        .map_err(|e| format!("internal error: template manifest invalid: {e}"))?;
    fresh.version = manifest.version.clone();
    if uses_texture {
        // The photo goes to the texture slot (api_texture_put writes
        // the manifest with the texture); re-read before applying the
        // template identity so that edit is not lost.
        api_texture_put(name, fit, &bytes).map_err(|(e, _)| e)?;
        let (manifest, dir) =
            manifest_for(name).ok_or_else(|| format!("'{name}' is not installed"))?;
        let mut manifest = manifest;
        manifest.params = fresh.params;
        manifest.permissions = fresh.permissions;
        write_manifest(&dir, &manifest).map_err(|(e, _)| e)?;
    } else {
        manifest.params = fresh.params;
        manifest.permissions = fresh.permissions;
        manifest.textures.clear();
        write_manifest(&dir, &manifest).map_err(|(e, _)| e)?;
        println!("[studio] {effect} ignores the photo (pure-math effect) — texture removed");
    }
    println!("[studio] {name} — composed: {effect}");
    Ok(format!(
        r#"{{"ok":true,"name":"{}","effect":"{effect}","uses_photo":{uses_texture}}}"#,
        json_escape(name)
    ))
}

/// Effective values = manifest defaults with the config's overrides on
/// top (default section — the studio edits the default).
fn api_params_get(name: &str) -> String {
    let Some((manifest, _)) = manifest_for(name) else {
        return format!(r#"{{"error":"'{name}' is not installed"}}"#);
    };
    let overrides: std::collections::HashMap<String, f64> = config::Config::load()
        .ok()
        .flatten()
        .and_then(|c| c.default)
        .map(|d| {
            d.params
                .iter()
                .filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f)))
                .collect()
        })
        .unwrap_or_default();
    let rows: Vec<String> = manifest
        .params
        .iter()
        .map(|p| {
            let effective = overrides.get(&p.name).copied().unwrap_or(p.default as f64);
            format!(
                r#"{{"name":"{}","label":{},"default":{},"value":{}}}"#,
                json_escape(&p.name),
                p.label
                    .as_ref()
                    .map(|l| format!(r#""{}""#, json_escape(l)))
                    .unwrap_or_else(|| "null".into()),
                p.default,
                (effective * 100.0).round() / 100.0
            )
        })
        .collect();
    format!(r#"{{"params":[{}]}}"#, rows.join(","))
}

fn api_manifest_get(name: &str) -> Result<String, (String, &'static str)> {
    let (manifest, _) =
        manifest_for(name).ok_or_else(|| (format!("'{name}' is not installed"), "404"))?;
    serde_json::to_string(&manifest).map_err(|e| (e.to_string(), "500"))
}

fn api_params_set(body: &[u8]) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_slice(body).map_err(|_| "bad JSON".to_owned())?;
    let name = v
        .get("name")
        .and_then(|n| n.as_str())
        .ok_or("name required")?;
    let param = v
        .get("param")
        .and_then(|p| p.as_str())
        .ok_or("param required")?;
    let value = v
        .get("value")
        .and_then(|x| x.as_f64())
        .ok_or("value required")?;
    // The identity check is the real parser (`bruma params set`'s path):
    // unknown names and out-of-range values are rejected BEFORE any
    // write.
    let (manifest, _) = manifest_for(name).ok_or_else(|| format!("'{name}' is not installed"))?;
    if !manifest.params.iter().any(|p| p.name == param) {
        let known: Vec<&str> = manifest.params.iter().map(|p| p.name.as_str()).collect();
        return Err(format!(
            "'{name}' does not declare '{param}' (known: {})",
            known.join(", ")
        ));
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(format!("{param} must be 0..1"));
    }
    // Persist into the config's default section, then wake the
    // daemons. Editing IS viewing in the studio: if the config runs a
    // different package, the section ADOPTS this one (the previous
    // package's params are meaningless — same semantics as `bruma
    // open`), so the first slider move brings the wallpaper up.
    let mut cfg = config::Config::load().ok().flatten().unwrap_or_default();
    let section = cfg.default.get_or_insert_with(Default::default);
    if section
        .package
        .as_deref()
        .map(|p| p.split(':').next().unwrap_or(p))
        != Some(name)
    {
        section.package = Some(name.to_owned());
        section.params.clear();
        section.shader = None;
        section.image = None;
    }
    section.params.insert(
        param.to_owned(),
        serde_json::json!((value * 100.0).round() / 100.0),
    );
    let path = config::Config::default_path().ok_or("no config path")?;
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, json + "\n").map_err(|e| e.to_string())?;
    crate::params::wake_daemons();
    Ok(r#"{"ok":true}"#.into())
}

/// GET/PUT/POST router; `body` carries the request body (already read).
fn handle(stream: &mut TcpStream, method: &str, path: &str, query: &str, body: &[u8]) {
    let q = |key: &str| -> Option<String> {
        query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == key).then(|| url_decode(v))
        })
    };
    let ok = |stream: &mut TcpStream, result: Result<String, (String, &'static str)>| match result {
        Ok(j) => respond_json(stream, "200 OK", j),
        Err((e, status)) => {
            let status = match status {
                "400" => "400 Bad Request",
                "404" => "404 Not Found",
                _ => "500 Internal Server Error",
            };
            respond_json(
                stream,
                status,
                format!(r#"{{"error":"{}"}}"#, json_escape(&e)),
            );
        }
    };
    match (method, path) {
        ("GET", "/") => {
            respond(
                stream,
                "200 OK",
                "text/html; charset=utf-8",
                UI_HTML.as_bytes(),
            );
        }
        ("GET", "/api/packages") => respond_json(stream, "200 OK", api_packages()),
        ("POST", "/api/create") => match api_create(body) {
            Ok(j) => respond_json(stream, "200 OK", j),
            Err(e) => respond_json(
                stream,
                "400 Bad Request",
                format!(r#"{{"error":"{}"}}"#, json_escape(&e)),
            ),
        },
        ("GET", "/api/shader") => match q("name") {
            Some(n) => ok(
                stream,
                api_shader_get(&n).map(|src| String::from_utf8_lossy(&src).into_owned()),
            ),
            None => respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"name required"}"#.into(),
            ),
        },
        ("PUT", "/api/shader") => match q("name") {
            Some(n) => ok(stream, api_shader_put(&n, body)),
            None => respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"name required"}"#.into(),
            ),
        },
        ("GET", "/api/manifest") => match q("name") {
            Some(n) => ok(stream, api_manifest_get(&n)),
            None => respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"name required"}"#.into(),
            ),
        },
        ("GET", "/api/params") => match q("name") {
            Some(n) => respond_json(stream, "200 OK", api_params_get(&n)),
            None => respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"name required"}"#.into(),
            ),
        },
        ("POST", "/api/params") => match api_params_set(body) {
            Ok(j) => respond_json(stream, "200 OK", j),
            Err(e) => respond_json(
                stream,
                "400 Bad Request",
                format!(r#"{{"error":"{}"}}"#, json_escape(&e)),
            ),
        },
        ("POST", "/api/activate") => match api_activate(body) {
            Ok(j) => respond_json(stream, "200 OK", j),
            Err(e) => respond_json(
                stream,
                "400 Bad Request",
                format!(r#"{{"error":"{}"}}"#, json_escape(&e)),
            ),
        },
        ("POST", "/api/uninstall") => match api_uninstall(body) {
            Ok(j) => respond_json(stream, "200 OK", j),
            Err(e) => respond_json(
                stream,
                "400 Bad Request",
                format!(r#"{{"error":"{}"}}"#, json_escape(&e)),
            ),
        },
        ("GET", "/api/preview") => match q("name") {
            Some(n) => match api_preview_get(&n) {
                Ok(png) => respond(stream, "200 OK", "image/png", &png),
                Err((e, status)) => ok(stream, Err::<String, (String, &'static str)>((e, status))),
            },
            None => respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"name required"}"#.into(),
            ),
        },
        ("PUT", "/api/preview") => match q("name") {
            Some(n) => match api_preview_put(&n, body) {
                Ok(j) => respond_json(stream, "200 OK", j),
                Err((e, status)) => ok(stream, Err::<String, (String, &'static str)>((e, status))),
            },
            None => respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"name required"}"#.into(),
            ),
        },
        ("GET", "/api/textures") => match q("name") {
            Some(n) => match api_textures_get(&n) {
                Ok(items) => respond_json(stream, "200 OK", items),
                Err((e, status)) => ok(stream, Err::<String, (String, &'static str)>((e, status))),
            },
            None => respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"name required"}"#.into(),
            ),
        },
        ("PUT", "/api/textures") => match q("name") {
            Some(n) => match q("fit") {
                Some(fit) => match api_texture_put(&n, &fit, body) {
                    Ok(j) => respond_json(stream, "200 OK", j),
                    Err((e, status)) => {
                        ok(stream, Err::<String, (String, &'static str)>((e, status)))
                    }
                },
                None => respond_json(
                    stream,
                    "400 Bad Request",
                    r#"{"error":"fit required (cover|contain)"}"#.into(),
                ),
            },
            None => respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"name required"}"#.into(),
            ),
        },
        ("GET", "/api/texture") => match (q("name"), q("i")) {
            (Some(n), Some(i)) => api_texture_file(&n, &i, stream),
            _ => respond_json(
                stream,
                "400 Bad Request",
                r#"{"error":"name and i required"}"#.into(),
            ),
        },
        ("POST", "/api/compose") => match api_compose(body) {
            Ok(j) => respond_json(stream, "200 OK", j),
            Err(e) => respond_json(
                stream,
                "400 Bad Request",
                format!(r#"{{"error":"{}"}}"#, json_escape(&e)),
            ),
        },
        _ => respond_json(stream, "404 Not Found", r#"{"error":"no route"}"#.into()),
    }
}

/// Serves one texture file of an installed package (the UI shows it
/// as an <img>): /api/texture?name=N&i=photo.jpg (relative to assets/).
fn api_texture_file(name: &str, rel: &str, stream: &mut TcpStream) {
    let Some((_, dir)) = manifest_for(name) else {
        respond_json(stream, "404 Not Found", r#"{"error":"not found"}"#.into());
        return;
    };
    // No traversal: only the file name under assets/.
    let file = std::path::Path::new(rel)
        .file_name()
        .map(|f| dir.join("assets").join(f));
    match file.and_then(|p| std::fs::read(p).ok()) {
        Some(bytes) => {
            let ctype = match image::guess_format(&bytes) {
                Ok(image::ImageFormat::Jpeg) => "image/jpeg",
                _ => "image/png",
            };
            respond(stream, "200 OK", ctype, &bytes);
        }
        None => respond_json(stream, "404 Not Found", r#"{"error":"no texture"}"#.into()),
    }
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 3 <= bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => out.push(b),
                    // Malformed escape: copy the three bytes verbatim.
                    Err(_) => out.extend_from_slice(&bytes[i..i + 3]),
                }
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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

/// Reads head + full body (shader PUTs can exceed the 8 KiB a small
/// request occupies; the loop reads to Content-Length).
fn read_request(stream: &mut TcpStream) -> Option<(String, String, String, Vec<u8>)> {
    let mut buf = Vec::with_capacity(8192);
    let head_end;
    loop {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            head_end = pos + 4;
            break;
        }
        if buf.len() > 1 << 20 {
            return None; // head sanity cap
        }
    }
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let first = lines.next()?.to_string();
    let mut parts = first.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let content_length = lines
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse::<usize>().ok())?
        })
        .next()
        .unwrap_or(0);
    if content_length > 8 << 20 {
        return None; // a shader is text; 8 MiB is plenty
    }
    let mut body = buf[head_end..].to_vec();
    while body.len() < content_length {
        let mut chunk = [0u8; 16384];
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_owned(), q.to_owned()),
        None => (target, String::new()),
    };
    Some((method, path, query, body))
}

fn serve_loop(listener: TcpListener) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let Some((method, path, query, body)) = read_request(&mut stream) else {
            continue;
        };
        handle(&mut stream, &method, &path, &query, &body);
    }
}

/// `bruma studio [--port N]` — starts the server and opens the browser.
/// Blocks until killed (Ctrl-C).
pub fn studio_command(args: &[String]) {
    let mut port: u16 = 7601;
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
                eprintln!("unknown argument: {other}\nusage: bruma studio [--port N]");
                std::process::exit(2);
            }
        }
    }
    let listener = TcpListener::bind(("127.0.0.1", port)).unwrap_or_else(|e| {
        eprintln!("error: cannot bind 127.0.0.1:{port}: {e}");
        std::process::exit(1);
    });
    let url = format!("http://127.0.0.1:{port}/");
    println!("[studio] serving the creator at {url}");
    println!("[studio] the running wallpaper is the live preview (edit → hot-reload)");
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

include!("studio_ui.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_decode_handles_escapes() {
        assert_eq!(url_decode("a%20b+c"), "a b c");
        assert_eq!(url_decode("plain"), "plain");
        assert_eq!(url_decode("%7Bx%7D"), "{x}");
        assert_eq!(url_decode("%zz"), "%zz");
    }
    #[test]
    fn naga_error_reports_broken_shader() {
        // A complete fragment entry point (as the engine compiles it).
        assert!(
            naga_error(
                "@fragment fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }"
            )
            .is_none()
        );
        // The prelude is in scope: a creator shader may call its helper.
        assert!(naga_error(
            "@fragment fn fs_main() -> @location(0) vec4<f32> { let _ = bruma_texture_fit(vec2<f32>(0.5), placeholder_tex_error, vec2<f32>(1.0), 0.0); return vec4<f32>(0.0); }"
        )
        .is_some());
        let err = naga_error("fn broken() -> { let x: undeclared_type = 1; }");
        assert!(err.is_some());
    }
}
