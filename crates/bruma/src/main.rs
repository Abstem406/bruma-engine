//! bruma engine's CLI.
//!
//! Status: **Phase 5**. Subcommands: `run` (color, image, shader or
//! package — one for all outputs with flags, or per output via the
//! persistent config), `validate`/`install`/`list`/`pack` (the
//! `.wallpaper` format), `config` (init/show) and `service` (systemd user
//! service installation). `new` arrives with Phase 6.

use bruma_core::Color;
use std::time::Duration;

mod config;
mod gallery;
mod layers;
mod mime;
mod new;
mod params;
mod studio;

fn main() {
    // Minimal logging with no dependencies; when the project needs more, a
    // facade will be chosen with care (see D8: audit dependencies).
    // Verbosity: BRUMA_DEBUG=1 enables traces (info only for now).
    log::set_logger(&BRUMA_LOGGER).expect("single logger");
    log::set_max_level(log::LevelFilter::Info);

    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("run") => run_command(&args.collect::<Vec<_>>()),
        Some("validate") => validate_command(&args.collect::<Vec<_>>()),
        Some("install") => install_command(&args.collect::<Vec<_>>()),
        Some("list") => list_command(),
        Some("pack") => pack_command(&args.collect::<Vec<_>>()),
        Some("new") => new_command(&args.collect::<Vec<_>>()),
        Some("config") => config_command(&args.collect::<Vec<_>>()),
        Some("params") => params::params_command(&args.collect::<Vec<_>>()),
        Some("open") => mime::open_command(&args.collect::<Vec<_>>()),
        Some("mime") => mime::mime_command(&args.collect::<Vec<_>>()),
        Some("gallery") => gallery::gallery_command(&args.collect::<Vec<_>>()),
        Some("studio") => studio::studio_command(&args.collect::<Vec<_>>()),
        Some("service") => service_command(&args.collect::<Vec<_>>()),
        Some("--version") | Some("-V") | None => {
            println!(
                "bruma v{} — free animated wallpaper engine for Wayland",
                bruma_core::ENGINE_VERSION
            );
            if std::env::args().count() == 1 {
                eprintln!(
                    "\nUsage: bruma <COMMAND>\n\nCommands:\n  run [options] [color]    Background behind the windows (no flags: uses the config)\n  validate PACKAGE         Validates a .wallpaper file\n  install PACKAGE          Installs a package\n  open PACKAGE.wallpaper   Installs it AND makes it the wallpaper\n  mime install|remove      Registers .wallpaper for double click / drag-and-drop\n  gallery [--port N]       Visual collection manager (browser, drag & drop)
  studio [--port N]        Visual creator: edit shader/params with live preview\n  list                     Lists installed packages\n  pack DIRECTORY           Packs a directory into .wallpaper\n  new NAME [--template T]  Scaffolds a wallpaper package (templates: {} )  \n  config init|show         Creates/shows the persistent config\n  params WALLPAPER [op]    Live parameter tuning (list | set NAME VALUE | reset)\n  service install|remove   Starts with the session (user service)",
                    new::template_list()
                );
            }
        }
        Some(other) => {
            eprintln!(
                "unknown command: {other}\n\nAvailable commands:\n  run | validate | install | open | mime | gallery | studio | list | pack | new | config | params | service"
            );
            std::process::exit(2);
        }
    }
}

/// Install directory: `$XDG_DATA_HOME/bruma/wallpapers`
/// (`~/.local/share/bruma/wallpapers` by default).
fn default_store() -> bruma_package::Store {
    let data_home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_owned());
            std::path::PathBuf::from(home).join(".local/share")
        });
    bruma_package::Store::new(data_home.join("bruma/wallpapers"))
}

/// Reads a whole .wallpaper file into memory (bruma-package's anti-bomb
/// limits operate on the bytes).
fn read_package(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("error: could not read {path}: {e}");
        std::process::exit(1);
    })
}

fn validate_command(args: &[String]) {
    let Some(path) = args.first() else {
        eprintln!("usage: bruma validate PACKAGE.wallpaper");
        std::process::exit(2);
    };
    let bytes = read_package(path);
    match bruma_package::Store::validate(&bytes) {
        Ok(m) => {
            println!("✔ {path} is a valid package:");
            println!("{m}");
        }
        Err(e) => {
            eprintln!("✘ {path} is NOT valid:\n  {e}");
            std::process::exit(1);
        }
    }
}

fn install_command(args: &[String]) {
    let Some(path) = args.first() else {
        eprintln!("usage: bruma install PACKAGE.wallpaper");
        std::process::exit(2);
    };
    let bytes = read_package(path);
    let manifest = bruma_package::Store::validate(&bytes).unwrap_or_else(|e| {
        eprintln!("error: invalid package: {e}");
        std::process::exit(1);
    });
    let store = default_store();
    let (dest, installed) = store
        .install(&bytes, &manifest.install_name(), &manifest.version)
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });
    if installed {
        println!(
            "✔ installed '{}' {} at {}",
            manifest.title,
            manifest.version,
            dest.display()
        );
        println!(
            "  try it with: bruma run --package {}",
            manifest.install_name()
        );
    } else {
        println!(
            "• '{}' {} was already installed at {}",
            manifest.title,
            manifest.version,
            dest.display()
        );
    }
}

fn list_command() {
    let store = default_store();
    let pkgs = store.installed();
    if pkgs.is_empty() {
        println!("no packages installed (use: bruma install PACKAGE.wallpaper)");
        return;
    }
    for (name, version, path) in pkgs {
        println!("{name} {version}  — {}", path.display());
    }
}

fn pack_command(args: &[String]) {
    let Some(dir) = args.first() else {
        eprintln!("usage: bruma pack DIRECTORY [-o OUTPUT.wallpaper]");
        std::process::exit(2);
    };
    let mut out: Option<std::path::PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                let v = args.get(i + 1).unwrap_or_else(|| {
                    eprintln!("-o requires an output path");
                    std::process::exit(2);
                });
                out = Some(std::path::PathBuf::from(v));
                i += 2;
            }
            other => {
                eprintln!(
                    "unknown argument: {other}\nusage: bruma pack DIRECTORY [-o OUTPUT.wallpaper]"
                );
                std::process::exit(2);
            }
        }
    }
    match bruma_package::Store::pack(std::path::Path::new(dir), out.as_deref()) {
        Ok(p) => println!("✔ packed into {}", p.display()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn new_command(args: &[String]) {
    let Some(name) = args.first() else {
        eprintln!(
            "usage: bruma new NAME [--template waves|fog|water] [--dir PATH]\n  NAME is also the package title; rename it later in wallpaper.json"
        );
        std::process::exit(2);
    };
    let mut template = "waves";
    let mut dir: Option<std::path::PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--template" | "-t" => {
                i += 1;
                template = args.get(i).map(|s| s.as_str()).unwrap_or_else(|| {
                    eprintln!("--template requires a name ({})", new::template_list());
                    std::process::exit(2);
                });
            }
            "--dir" => {
                i += 1;
                dir = Some(std::path::PathBuf::from(
                    args.get(i)
                        .unwrap_or_else(|| {
                            eprintln!("--dir requires a path");
                            std::process::exit(2);
                        })
                        .clone(),
                ));
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    match new::create(name, template, dir.as_deref()) {
        Ok(files) => {
            println!(
                "✔ wallpaper '{name}' scaffolded (template: {template}) in {}",
                files[0]
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .display()
            );
            println!("  next steps:");
            println!(
                "    1. edit {} with your editor (hot-reload shows it live)",
                files[1].display()
            );
            println!(
                "    2. bruma pack {}/            → {}.wallpaper",
                name, name
            );
            println!(
                "    3. bruma validate {}.wallpaper && bruma install {}.wallpaper",
                name, name
            );
            println!("    4. bruma run --package {}", name);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}

fn config_command(args: &[String]) {
    match args.first().map(|s| s.as_str()) {
        Some("init") => match config::Config::write_example() {
            Ok(p) => {
                println!("✔ example config written to {}", p.display());
                println!(
                    "  edit it (output names come from `niri msg outputs`) and start: bruma run"
                );
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        Some("show") => {
            if let Some(p) = config::Config::default_path() {
                println!("path: {}", p.display());
            }
            match config::Config::load() {
                Ok(Some(c)) => println!("{c:#?}"),
                Ok(None) => println!("(does not exist yet — create one with: bruma config init)"),
                Err(e) => {
                    eprintln!("✘ invalid config: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: bruma config <init|show>");
            std::process::exit(2);
        }
    }
}

/// User unit template: starts with the graphical session and revives if it
/// dies. `run` with no flags loads the persistent config.
const SYSTEMD_UNIT: &str = "\
[Unit]
Description=bruma — animated wallpaper engine (Wayland)
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart={exe} run
Restart=always
RestartSec=2

[Install]
WantedBy=graphical-session.target
";

fn service_command(args: &[String]) {
    let Some(home) = std::env::var("HOME").ok().filter(|h| !h.is_empty()) else {
        eprintln!("error: without HOME there is no user unit");
        std::process::exit(1);
    };
    let unit_dir = std::path::PathBuf::from(home).join(".config/systemd/user");
    let unit_path = unit_dir.join("bruma.service");
    let systemctl = |args: &[&str]| -> bool {
        std::process::Command::new("systemctl")
            .arg("--user")
            .args(args)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    match args.first().map(|s| s.as_str()) {
        Some("install") => {
            let exe = std::env::current_exe().unwrap_or_else(|e| {
                eprintln!("error: cannot locate the binary: {e}");
                std::process::exit(1);
            });
            std::fs::create_dir_all(&unit_dir).unwrap_or_else(|e| {
                eprintln!("error: {}: {e}", unit_dir.display());
                std::process::exit(1);
            });
            std::fs::write(
                &unit_path,
                SYSTEMD_UNIT.replace("{exe}", &exe.display().to_string()),
            )
            .unwrap_or_else(|e| {
                eprintln!("error: {}: {e}", unit_path.display());
                std::process::exit(1);
            });
            println!("✔ unit written to {}", unit_path.display());
            if systemctl(&["daemon-reload"]) && systemctl(&["enable", "--now", "bruma.service"]) {
                println!(
                    "✔ service enabled and started (revives if it dies; starts with the session)"
                );
                println!("  logs: journalctl --user -u bruma.service -f");
            } else {
                eprintln!("⚠ systemctl --user did not respond: the unit was written;");
                eprintln!("  check with: systemctl --user status bruma.service");
                std::process::exit(1);
            }
        }
        Some("remove") => {
            systemctl(&["disable", "--now", "bruma.service"]);
            match std::fs::remove_file(&unit_path) {
                Ok(()) => {
                    let _ = systemctl(&["daemon-reload"]);
                    println!("✔ service removed");
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("• no service was installed");
                }
                Err(e) => {
                    eprintln!("error: {}: {e}", unit_path.display());
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: bruma service <install|remove>");
            std::process::exit(2);
        }
    }
}

/// An already-resolved source for ONE output (Phase 5).
enum Source {
    /// Animated shader (from a package or a loose file) with overrides by
    /// position. `textures` are the package's texture files in manifest
    /// order (absolute paths); empty for loose shaders. `feedback` arms
    /// the previous-frame ping-pong (manifest `feedback` permission).
    Shader {
        path: String,
        overrides: Vec<(usize, f32)>,
        /// Package texture files in manifest order (absolute paths) with
        /// their declared aspect fit; empty for loose shaders.
        textures: Vec<(String, bruma_renderer_wgpu::TextureFit)>,
        feedback: bool,
    },
    /// Static image.
    Image(String),
    /// SHM color for this output (fallback or content).
    Color(u32),
    /// No source → the window's global color.
    None,
}

/// Resolves an installed `name[:version]` → (path, manifest), with the
/// usual validations (min_engine, shader type).
fn resolve_package(spec: &str) -> (std::path::PathBuf, bruma_package::Manifest) {
    let store = default_store();
    let (pkg, want_ver) = match spec.split_once(':') {
        Some((p, v)) => (p, Some(v.to_owned())),
        None => (spec, None),
    };
    let mut found = None;
    for (n, v, path) in store.installed() {
        if n == pkg && want_ver.as_deref().is_none_or(|w| v == *w) {
            found = Some((v, path));
        }
    }
    let Some((_ver, pkg_path)) = found else {
        eprintln!("error: nothing installed named '{spec}' (see: bruma list)");
        std::process::exit(1);
    };
    let json = std::fs::read_to_string(pkg_path.join("wallpaper.json")).unwrap_or_else(|e| {
        eprintln!("error: package without a manifest: {e}");
        std::process::exit(1);
    });
    let manifest = bruma_package::Manifest::parse(&json).unwrap_or_else(|e| {
        eprintln!("error: invalid manifest: {e}");
        std::process::exit(1);
    });
    if let Some(min) = &manifest.min_engine
        && min.as_str() > bruma_core::ENGINE_VERSION
    {
        eprintln!(
            "error: the package requires engine >= {min} and this is {}",
            bruma_core::ENGINE_VERSION
        );
        std::process::exit(1);
    }
    if manifest.wallpaper_type != "shader" {
        eprintln!(
            "error: the package is of type '{}' (reserved, not implemented yet)",
            manifest.wallpaper_type
        );
        std::process::exit(1);
    }
    (pkg_path, manifest)
}

/// Resolves a config entry (or legacy flags) into a [`Source`].
///
/// `cli_overrides` are the legacy-mode `--param name=value` flags (with a
/// config, params live in the entry itself). Unknown ones are an error:
/// typos must hurt, not be ignored.
fn resolve_source(
    oc: &config::OutputConfig,
    cli_overrides: &[(String, f32)],
    pkg_cache: &mut std::collections::HashMap<
        String,
        (std::path::PathBuf, bruma_package::Manifest),
    >,
    runtime_params: &mut Vec<bruma_runtime::ParamValue>,
    manifest_fps: &mut Option<u32>,
) -> Source {
    if let Some(spec) = &oc.package {
        let entry = pkg_cache
            .entry(spec.clone())
            .or_insert_with(|| resolve_package(spec));
        let (pkg_path, manifest) = (&entry.0.clone(), &entry.1.clone());
        if manifest_fps.is_none() {
            *manifest_fps = manifest.fps;
        }
        if runtime_params.is_empty() {
            *runtime_params = manifest
                .params
                .iter()
                .map(|p| bruma_runtime::ParamValue {
                    name: p.name.clone(),
                    value: p.default,
                })
                .collect();
        }
        // Overrides: config first, CLI on top; name → position.
        let mut pairs = config::params_to_pairs(&oc.params).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(2);
        });
        pairs.extend(cli_overrides.iter().cloned());
        let mut explicit: Vec<(usize, f32)> = Vec::new();
        for (name, value) in pairs {
            match manifest.params.iter().position(|p| p.name == name) {
                Some(pos) => explicit.push((pos, value.clamp(0.0, 1.0))),
                None => {
                    eprintln!("error: '{spec}' does not declare parameter '{name}'");
                    std::process::exit(2);
                }
            }
        }
        // FULL coverage: every manifest position gets a value (explicit
        // override or the manifest default). Renderers then hold the
        // complete param state, so a hot package switch (SIGHUP after a
        // compose) can never inherit the previous package's uniforms —
        // the runtime's own seeding happens at startup only.
        let mut by_pos = explicit
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        let overrides: Vec<(usize, f32)> = manifest
            .params
            .iter()
            .enumerate()
            .map(|(i, p)| (i, by_pos.remove(&i).unwrap_or(p.default).clamp(0.0, 1.0)))
            .collect();
        return Source::Shader {
            path: pkg_path.join(&manifest.entry).display().to_string(),
            overrides,
            textures: manifest
                .textures
                .iter()
                .map(|spec| {
                    (
                        pkg_path.join(&spec.path).display().to_string(),
                        match spec.fit {
                            bruma_package::TextureFit::Cover => {
                                bruma_renderer_wgpu::TextureFit::Cover
                            }
                            bruma_package::TextureFit::Contain => {
                                bruma_renderer_wgpu::TextureFit::Contain
                            }
                        },
                    )
                })
                .collect(),
            feedback: manifest.permissions.iter().any(|p| p == "feedback"),
        };
    }
    if let Some(p) = &oc.shader {
        return Source::Shader {
            path: p.clone(),
            overrides: Vec::new(),
            textures: Vec::new(),
            feedback: false,
        };
    }
    if let Some(p) = &oc.image {
        return Source::Image(p.clone());
    }
    match &oc.color {
        Some(c) => Source::Color(config::parse_color_hex(c).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(2);
        })),
        None => Source::None,
    }
}

fn run_command(args: &[String]) {
    let mut color_arg: Option<&str> = None;
    // `--gpu` forces GPU init even if the chosen source is a color (handy
    // to warm up shaders/driver before switching sources).
    let mut gpu = false;
    let mut image_path: Option<String> = None;
    let mut shader_path: Option<String> = None;
    let mut package: Option<String> = None;
    let mut fps: Option<u32> = None;
    // Phase 3: direct `--param=VALUE`. With a manifest, `--param
    // name=VALUE` resolves by name (see below).
    let mut param0: Option<f32> = None;
    let mut named_params: Vec<(String, f32)> = Vec::new();
    let mut config_path: Option<String> = None;
    // Disable the fullscreen pause (D12): keep rendering under
    // fullscreen windows. A fullscreen window on ANOTHER workspace also
    // counts as covering (compositors don't report workspaces over
    // wlr-foreign-toplevel), so users who move to an empty workspace
    // and still see a frozen wallpaper need this.
    let mut no_fullscreen_pause = false;
    // D12's battery escape hatch: keep rendering while discharging.
    let mut no_battery_pause = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--gpu" => gpu = true,
            "--image" => {
                i += 1;
                image_path = Some(args.get(i).map(|s| s.to_string()).unwrap_or_else(|| {
                    eprintln!("--image requires an image path");
                    std::process::exit(2);
                }));
            }
            "--shader" => {
                i += 1;
                shader_path = Some(args.get(i).map(|s| s.to_string()).unwrap_or_else(|| {
                    eprintln!("--shader requires the path to a .wgsl file");
                    std::process::exit(2);
                }));
            }
            "--package" => {
                i += 1;
                package = Some(args.get(i).map(|s| s.to_string()).unwrap_or_else(|| {
                    eprintln!(
                        "--package requires the name of an installed package (see: bruma list)"
                    );
                    std::process::exit(2);
                }));
            }
            "--fps" => {
                i += 1;
                fps = Some(args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("--fps requires an integer (e.g. --fps 60)");
                    std::process::exit(2);
                }));
            }
            // Phase 5: explicit config (exclusive with source flags).
            "--config" => {
                i += 1;
                config_path = Some(args.get(i).map(|s| s.to_string()).unwrap_or_else(|| {
                    eprintln!("--config requires the path to a config.json");
                    std::process::exit(2);
                }));
            }
            "--no-fullscreen-pause" => no_fullscreen_pause = true,
            "--no-battery-pause" => no_battery_pause = true,
            // Phase 3: `--param=0.5` (parameter 0). Phase 4: also
            // `--param=intensidad=0.5` when a manifest provides names.
            p if p.starts_with("--param=") => {
                // After the prefix: `VALUE` (parameter 0) or `NAME=VALUE`.
                let rest = &p["--param=".len()..];
                match rest.split_once('=') {
                    // `--param=VALUE`: no name → parameter 0 (u_params0).
                    None => {
                        let value = rest.parse::<f32>().unwrap_or_else(|_| {
                            eprintln!("--param: '{rest}' is not a number (0..1)");
                            std::process::exit(2);
                        });
                        param0 = Some(value.clamp(0.0, 1.0));
                    }
                    // `--param=NAME=VALUE`: with a name (only valid with a
                    // package; validated after loading the manifest).
                    Some((name, v)) => {
                        let value = v.parse::<f32>().unwrap_or_else(|_| {
                            eprintln!("--param: '{v}' is not a number (0..1)");
                            std::process::exit(2);
                        });
                        named_params.push((name.trim().to_owned(), value));
                    }
                }
            }
            "--param" => {
                eprintln!("--param requires the form --param[=name]=VALUE (0..1)");
                std::process::exit(2);
            }
            other => color_arg = Some(other),
        }
        i += 1;
    }

    // Source mode: explicit flags = ONE source for all outputs (previous
    // behavior, ideal for testing). Without flags, the persistent config
    // rules (auto-loaded if it exists).
    let explicit_source =
        shader_path.is_some() || image_path.is_some() || package.is_some() || color_arg.is_some();
    let cfg = match &config_path {
        Some(p) => {
            if explicit_source {
                eprintln!("error: --config is exclusive with --package/--shader/--image/color");
                std::process::exit(2);
            }
            let text = std::fs::read_to_string(p).unwrap_or_else(|e| {
                eprintln!("error: could not read {p}: {e}");
                std::process::exit(1);
            });
            let c: config::Config = serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("error: {p}: {e}");
                std::process::exit(1);
            });
            c.validate().unwrap_or_else(|e| {
                eprintln!("error: {p}: {e}");
                std::process::exit(1);
            });
            log::info!("explicit config: {p}");
            Some(c)
        }
        None if explicit_source => None,
        None => match config::Config::load() {
            Ok(c) => {
                if c.is_some() {
                    log::info!("persistent config loaded (bare run uses the config)");
                }
                c
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
    };

    // GLOBAL color (last-resort fallback: no renderer and no output
    // color). With a config, the per-output color lives in its
    // Source::Color.
    let color = color_arg
        .map(|s| {
            let v = s.trim().trim_start_matches("0x").trim_start_matches('#');
            u32::from_str_radix(v, 16).unwrap_or_else(|_| {
                eprintln!("invalid color: {s} (use hex, e.g. 0x3B4252)");
                std::process::exit(2);
            })
        })
        .unwrap_or(0x2E_34_40); // default dark bluish gray

    // Per-output source resolution (the single source of truth for the
    // factory).
    let mut pkg_cache: std::collections::HashMap<
        String,
        (std::path::PathBuf, bruma_package::Manifest),
    > = Default::default();
    let mut manifest_fps: Option<u32> = None;
    let mut runtime_params: Vec<bruma_runtime::ParamValue> = Vec::new();
    let mut has_content = false;

    let cli_overrides: Vec<(String, f32)> = named_params.clone();

    let mut per_output: Vec<(String, Source)> = Vec::new();
    let mut default_source = Source::None;
    if let Some(cfg) = &cfg {
        if let Some(d) = &cfg.default {
            default_source = resolve_source(
                d,
                &[],
                &mut pkg_cache,
                &mut runtime_params,
                &mut manifest_fps,
            );
            has_content |= d.has_content();
        }
        for (name, oc) in &cfg.outputs {
            let f = resolve_source(
                oc,
                &[],
                &mut pkg_cache,
                &mut runtime_params,
                &mut manifest_fps,
            );
            has_content |= oc.has_content();
            per_output.push((name.clone(), f));
        }
    } else {
        // Legacy: one source for all outputs.
        if let Some(spec) = &package {
            let oc = config::OutputConfig {
                package: Some(spec.clone()),
                ..Default::default()
            };
            default_source = resolve_source(
                &oc,
                &cli_overrides,
                &mut pkg_cache,
                &mut runtime_params,
                &mut manifest_fps,
            );
            has_content = true;
        } else if let Some(p) = &shader_path {
            default_source = Source::Shader {
                path: p.clone(),
                overrides: Vec::new(),
                textures: Vec::new(),
                feedback: false,
            };
            has_content = true;
        } else if let Some(p) = &image_path {
            default_source = Source::Image(p.clone());
            has_content = true;
        } else {
            default_source = Source::Color(color);
        }
    }

    // FPS precedence: flag > config > manifest > 30.
    let fps = fps
        .or_else(|| cfg.as_ref().and_then(|c| c.fps))
        .or(manifest_fps)
        .unwrap_or(30);

    log::info!(
        "bruma run — mode {}{}",
        if has_content && per_output.is_empty() && !matches!(default_source, Source::Color(_)) {
            "animated shader"
        } else if has_content {
            "per-output mix"
        } else {
            "solid color"
        },
        if has_content {
            format!(" ({fps} fps)")
        } else {
            String::new()
        }
    );
    let mut window = bruma_platform::BackgroundWindow::new(Color::from_rgb_u32(color))
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });
    // D12's escape hatch: disable the fullscreen pause (flag, or config
    // "fullscreen_pause": false — the flag wins).
    window.set_fullscreen_pause(
        !no_fullscreen_pause && cfg.as_ref().is_none_or(|c| c.fullscreen_pause),
    );
    // Same for the battery pause (render while discharging): the API
    // takes "pause enabled", the flag says "disable it".
    window.set_battery_pause(!no_battery_pause);

    // Phase 5: the factory builds one renderer PER OUTPUT. The GPU is
    // discovered once (GpuShared clones cheaply); each output gets its own
    // renderer on its own surface. A factory error degrades that output to
    // solid color without taking the rest down.
    let mut shared_gpu: Option<bruma_renderer_wgpu::GpuShared> = None;
    // With a config, an output may declare a shader even if `default` is a
    // color: the factory knows via `per_output`/`default_source` (moved
    // inside). Any shader/image in play requires the GPU.
    let source_needs_gpu = |f: &Source| matches!(f, Source::Shader { .. } | Source::Image(_));
    let needs_gpu =
        source_needs_gpu(&default_source) || per_output.iter().any(|(_, f)| source_needs_gpu(f));
    if needs_gpu || gpu {
        match bruma_renderer_wgpu::GpuShared::new() {
            Ok(g) => shared_gpu = Some(g),
            Err(e) => {
                // Without a GPU, solid-color mode is still useful.
                log::warn!("no GPU ({e}); all outputs on solid color");
            }
        }
    }

    // Hot-reload callback shared by all outputs (a single notification
    // even with N surfaces reloading the same shader).
    let notifier = std::rc::Rc::new(bruma_platform::DesktopNotifier::new());
    let notify_state: std::rc::Rc<std::cell::RefCell<Option<(String, std::time::Instant)>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));

    // SHARED source model (Phase 5): the factory reads it on every
    // renderer build (startup and reloads); the SIGHUP handler updates it.
    // Rc<RefCell> because both live on the loop's thread (no cross-thread,
    // no Mutex).
    struct SourceModel {
        per_output: Vec<(String, Source)>,
        default: Source,
    }
    let source_model: std::rc::Rc<std::cell::RefCell<SourceModel>> =
        std::rc::Rc::new(std::cell::RefCell::new(SourceModel {
            per_output,
            default: default_source,
        }));

    let model_f = source_model.clone();
    let notifier_f = notifier.clone();
    let notify_state_f = notify_state.clone();
    window.set_renderer_factory(Box::new(move |handles| {
        let m = model_f.borrow();
        // A per-output section WITHOUT content is no pin at all: it falls
        // through to the default (an empty section must not shadow it).
        let source = handles
            .output_name
            .as_deref()
            .and_then(|n| m.per_output.iter().find(|(name, _)| name == n))
            .map(|(_, f)| f)
            .filter(|f| !matches!(f, Source::None))
            .unwrap_or(&m.default);
        let display = handles.display_ptr;
        let surface = handles.surface_ptr;
        match source {
            Source::Shader {
                path,
                overrides,
                textures,
                feedback,
            } => {
                let mut renderer = unsafe {
                    bruma_renderer_wgpu::AnimatedRenderer::on_shared(
                        shared_gpu.as_ref().ok_or("no GPU")?,
                        display,
                        surface,
                        std::path::Path::new(path),
                        *feedback,
                        // A `display(uv, frame, u)` entry means the
                        // shader controls how the offscreen state reaches
                        // the screen (e.g. water over a photo): the blit
                        // runs fs_display.
                        *feedback
                            && std::fs::read_to_string(path)
                                .is_ok_and(|src| src.contains("fn display(")),
                    )
                }
                .map_err(|e| e.to_string())?;
                // Package textures (manifest `textures` order) go to the
                // fixed slots before the first frame; empty for loose
                // shaders.
                renderer.set_textures(
                    &textures.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(),
                    &textures.iter().map(|(_, f)| *f).collect::<Vec<_>>(),
                );
                if !overrides.is_empty() {
                    // THIS output's overrides (resolved by name against
                    // the manifest in the CLI).
                    renderer.set_param_overrides(overrides.clone());
                }

                // "Shader rejected" notice visible without a terminal:
                // best-effort D-Bus with a 2 s debounce (shared by
                // outputs).
                let notifier = notifier_f.clone();
                let last = notify_state_f.clone();
                renderer.set_reload_callback(Box::new(move |event| {
                    use bruma_renderer_wgpu::ReloadEvent;
                    match event {
                        ReloadEvent::Rejected { error } => {
                            let first_line = error.lines().next().unwrap_or(error);
                            let mut brief: String = first_line.chars().take(140).collect();
                            if brief.len() < first_line.len() {
                                brief.push('…');
                            }
                            let mut last = last.borrow_mut();
                            let now = std::time::Instant::now();
                            let fresh = last.as_ref().is_none_or(|(m, t)| {
                                now.duration_since(*t) >= Duration::from_secs(2) || *m != brief
                            });
                            if fresh {
                                *last = Some((brief.clone(), now));
                                notifier.shader_rejected(&brief);
                            }
                        }
                        ReloadEvent::Recovered => {
                            *last.borrow_mut() = None;
                            notifier.shader_recovered();
                        }
                        ReloadEvent::Applied => {}
                    }
                }));
                Ok(bruma_platform::FactoryRenderer::renderer(Box::new(
                    renderer,
                )))
            }
            Source::Image(path) => Ok(bruma_platform::FactoryRenderer::renderer(Box::new(
                unsafe {
                    bruma_renderer_wgpu::ImageRenderer::on_shared(
                        shared_gpu.as_ref().ok_or("no GPU")?,
                        display,
                        surface,
                        std::path::Path::new(path),
                    )
                }
                .map_err(|e| e.to_string())?,
            ))),
            Source::Color(c) => Ok(bruma_platform::FactoryRenderer::color(Color::from_rgb_u32(
                *c,
            ))),
            Source::None => Err("no source for this output".to_owned()),
        }
    }));

    let reports = window.present_once().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    for r in &reports {
        let mode = if r.gpu { "gpu" } else { "color" };
        log::info!(
            "wallpaper active on {:?}: {}x{}px, scale {} ({mode}) — Ctrl-C to exit",
            r.name,
            r.width,
            r.height,
            r.scale
        );
    }

    // Hot config reload: SIGHUP → re-parse and update the source model
    // (the platform rebuilds the renderers via the factory and repaints).
    // Scope: sources and params per output; an fps change waits for the
    // next start (the runtime is not mutated hot).
    if cfg.is_some() {
        let notifier_h = notifier.clone();
        let model_h = source_model.clone();
        window.on_config_reload(Box::new(move || {
            let cfg_new = match config::Config::load() {
                // No config or broken one: the wallpaper KEEPS the current.
                Ok(Some(c)) => c,
                Ok(None) => return false,
                Err(e) => {
                    log::warn!("config rejected: {e}");
                    notifier_h.shader_rejected(&format!("config: {e}"));
                    return false;
                }
            };
            let mut pkg_cache = Default::default();
            let mut manifest_fps = None;
            let mut params = Vec::new();
            let mut m = model_h.borrow_mut();
            if let Some(d) = &cfg_new.default {
                m.default = resolve_source(d, &[], &mut pkg_cache, &mut params, &mut manifest_fps);
            }
            m.per_output = cfg_new
                .outputs
                .iter()
                .map(|(name, oc)| {
                    (
                        name.clone(),
                        resolve_source(oc, &[], &mut pkg_cache, &mut params, &mut manifest_fps),
                    )
                })
                .collect();
            true
        }));
    } else if explicit_source && package.is_some() {
        // CLI run (--package): the full config reloader is not
        // installed, but `bruma params` still needs live tuning. A
        // narrow one: merge the config sections that run THIS package
        // (default + per-output matches, output params winning) into a
        // fresh Source — the factory rebuilds the renderers with the
        // new overrides. The source NEVER switches package here; if no
        // section runs the package, the current one stays.
        let pkg_base = package
            .as_deref()
            .expect("checked")
            .split(':')
            .next()
            .expect("nonempty")
            .to_owned();
        let cli_named = named_params.clone();
        let model_h = source_model.clone();
        window.on_config_reload(Box::new(move || {
            let Ok(Some(cfg_new)) = config::Config::load() else {
                return false;
            };
            let mut merged = match &cfg_new.default {
                Some(d)
                    if d.package
                        .as_deref()
                        .is_some_and(|p| p.split(':').next() == Some(pkg_base.as_str())) =>
                {
                    d.clone()
                }
                _ => config::OutputConfig {
                    package: Some(pkg_base.clone()),
                    ..Default::default()
                },
            };
            for s in cfg_new.outputs.values() {
                if s.package
                    .as_deref()
                    .is_some_and(|p| p.split(':').next() == Some(pkg_base.as_str()))
                {
                    merged
                        .params
                        .extend(s.params.iter().map(|(k, v)| (k.clone(), v.clone())));
                }
            }
            let mut pkg_cache = Default::default();
            let mut manifest_fps = None;
            let mut params = Vec::new();
            let resolved = resolve_source(
                &merged,
                &cli_named,
                &mut pkg_cache,
                &mut params,
                &mut manifest_fps,
            );
            let mut m = model_h.borrow_mut();
            m.default = resolved;
            m.per_output.clear();
            true
        }));
    }

    // The loop's mode is decided by the installed renderer: animated
    // (shader) drives frames at the runtime's pace; static (color, image,
    // triangle) only attends events.
    let wants_animation = window.wants_animation();
    if wants_animation {
        let mut runtime = bruma_runtime::BasicRuntime::new(fps);
        if !runtime_params.is_empty() {
            // Manifest defaults; the PER-OUTPUT overrides already live in
            // each renderer (the animation stays synchronized: one
            // runtime, one clock).
            runtime.set_params(runtime_params);
        } else if let Some(v) = param0 {
            runtime = runtime.with_param("param0", v);
        }
        window.run_with_runtime(&mut runtime).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });
    } else {
        window.run().unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });
    }
}

/// Minimal stderr logger, enough for Phase 1.
struct BrumaLogger;

static BRUMA_LOGGER: BrumaLogger = BrumaLogger;

impl log::Log for BrumaLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::LevelFilter::Info
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("[bruma] {}: {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}
