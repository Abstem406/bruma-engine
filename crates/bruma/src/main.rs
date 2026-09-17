//! CLI del motor bruma.
//!
//! Estado: **Fase 5**. Subcomandos: `run` (color, imagen, shader o
//! paquete — uno para todas las salidas con flags, o por salida vía
//! config persistente), `validate`/`install`/`list`/`pack` (formato
//! `.wallpaper`), `config` (init/show) y `service` (instalación como
//! servicio de usuario systemd). `new` llegará con la Fase 6.

use bruma_core::Color;
use std::time::Duration;

mod config;

fn main() {
    // Logging mínimo sin dependencias; cuando el proyecto lo necesite se
    // elegirá una facade con criterio (ver D8: auditar dependencias).
    // Nivel de detalle: BRUMA_DEBUG=1 activa trazas (de momento solo info).
    log::set_logger(&BRUMA_LOGGER).expect("logger único");
    log::set_max_level(log::LevelFilter::Info);

    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("run") => run_command(&args.collect::<Vec<_>>()),
        Some("validate") => validate_command(&args.collect::<Vec<_>>()),
        Some("install") => install_command(&args.collect::<Vec<_>>()),
        Some("list") => list_command(),
        Some("pack") => pack_command(&args.collect::<Vec<_>>()),
        Some("config") => config_command(&args.collect::<Vec<_>>()),
        Some("service") => service_command(&args.collect::<Vec<_>>()),
        Some("--version") | Some("-V") | None => {
            println!(
                "bruma v{} — motor libre de wallpapers animados para Wayland",
                bruma_core::ENGINE_VERSION
            );
            if std::env::args().count() == 1 {
                eprintln!(
                    "\nUso: bruma <COMANDO>\n\nComandos:\n  run [opciones] [color]   Fondo detrás de las ventanas (sin flags, usa la config)\n  validate PAQUETE         Valida un archivo .wallpaper\n  install PAQUETE          Instala un paquete\n  list                     Lista los paquetes instalados\n  pack DIRECTORIO          Empaqueta un directorio en .wallpaper\n  config init|show         Crea/muestra la config persistente\n  service install|remove   Arranca con la sesión (servicio de usuario)"
                );
            }
        }
        Some(other) => {
            eprintln!(
                "comando desconocido: {other}\n\nComandos disponibles:\n  run | validate | install | list | pack | config | service"
            );
            std::process::exit(2);
        }
    }
}

/// Directorio de instalación: `$XDG_DATA_HOME/bruma/wallpapers`
/// (`~/.local/share/bruma/wallpapers` por defecto).
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

/// Lee un archivo .wallpaper completo a memoria (los límites anti-bomba
/// de `bruma-package` operan sobre los bytes).
fn read_package(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("error: no se pudo leer {path}: {e}");
        std::process::exit(1);
    })
}

fn validate_command(args: &[String]) {
    let Some(path) = args.first() else {
        eprintln!("uso: bruma validate PAQUETE.wallpaper");
        std::process::exit(2);
    };
    let bytes = read_package(path);
    match bruma_package::Store::validate(&bytes) {
        Ok(m) => {
            println!("✔ {path} es un paquete válido:");
            println!("{m}");
        }
        Err(e) => {
            eprintln!("✘ {path} NO es válido:\n  {e}");
            std::process::exit(1);
        }
    }
}

fn install_command(args: &[String]) {
    let Some(path) = args.first() else {
        eprintln!("uso: bruma install PAQUETE.wallpaper");
        std::process::exit(2);
    };
    let bytes = read_package(path);
    let manifest = bruma_package::Store::validate(&bytes).unwrap_or_else(|e| {
        eprintln!("error: paquete inválido: {e}");
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
            "✔ instalado '{}' {} en {}",
            manifest.title,
            manifest.version,
            dest.display()
        );
        println!(
            "  pruébalo con: bruma run --package {}",
            manifest.install_name()
        );
    } else {
        println!(
            "• '{}' {} ya estaba instalado en {}",
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
        println!("no hay paquetes instalados (usa: bruma install PAQUETE.wallpaper)");
        return;
    }
    for (name, version, path) in pkgs {
        println!("{name} {version}  — {}", path.display());
    }
}

fn pack_command(args: &[String]) {
    let Some(dir) = args.first() else {
        eprintln!("uso: bruma pack DIRECTORIO [-o SALIDA.wallpaper]");
        std::process::exit(2);
    };
    let mut out: Option<std::path::PathBuf> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                let v = args.get(i + 1).unwrap_or_else(|| {
                    eprintln!("-o requiere una ruta de salida");
                    std::process::exit(2);
                });
                out = Some(std::path::PathBuf::from(v));
                i += 2;
            }
            other => {
                eprintln!(
                    "argumento desconocido: {other}\nuso: bruma pack DIRECTORIO [-o SALIDA.wallpaper]"
                );
                std::process::exit(2);
            }
        }
    }
    match bruma_package::Store::pack(std::path::Path::new(dir), out.as_deref()) {
        Ok(p) => println!("✔ empaquetado en {}", p.display()),
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
                println!("✔ config de ejemplo escrita en {}", p.display());
                println!(
                    "  edítala (los nombres de salida salen de `niri msg outputs`) y lanza: bruma run"
                );
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
        Some("show") => {
            if let Some(p) = config::Config::default_path() {
                println!("ruta: {}", p.display());
            }
            match config::Config::load() {
                Ok(Some(c)) => println!("{c:#?}"),
                Ok(None) => println!("(no existe aún — crea una con: bruma config init)"),
                Err(e) => {
                    eprintln!("✘ config inválida: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("uso: bruma config <init|show>");
            std::process::exit(2);
        }
    }
}

/// Plantilla de la unidad de usuario: arranca con la sesión gráfica y
/// revive si muere. `run` sin flags carga la config persistente.
const SYSTEMD_UNIT: &str = "\
[Unit]
Description=bruma — motor de wallpapers animados (Wayland)
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
        eprintln!("error: sin HOME no hay unidad de usuario");
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
                eprintln!("error: no sé dónde está el binario: {e}");
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
            println!("✔ unidad escrita en {}", unit_path.display());
            if systemctl(&["daemon-reload"]) && systemctl(&["enable", "--now", "bruma.service"]) {
                println!(
                    "✔ servicio habilitado y arrancado (revive si muere; arranca con la sesión)"
                );
                println!("  logs: journalctl --user -u bruma.service -f");
            } else {
                eprintln!("⚠ systemctl --user no respondió: la unidad quedó escrita;");
                eprintln!("  revisa con: systemctl --user status bruma.service");
                std::process::exit(1);
            }
        }
        Some("remove") => {
            systemctl(&["disable", "--now", "bruma.service"]);
            match std::fs::remove_file(&unit_path) {
                Ok(()) => {
                    let _ = systemctl(&["daemon-reload"]);
                    println!("✔ servicio eliminado");
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("• no había servicio instalado");
                }
                Err(e) => {
                    eprintln!("error: {}: {e}", unit_path.display());
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("uso: bruma service <install|remove>");
            std::process::exit(2);
        }
    }
}

/// Fuente ya resuelta para UNA salida (Fase 5).
enum Fuente {
    /// Shader animado (de paquete o suelto) con overrides por posición.
    Shader {
        path: String,
        overrides: Vec<(usize, f32)>,
    },
    /// Imagen estática.
    Imagen(String),
    /// Color SHM para esta salida (fallback o contenido).
    Color(u32),
    /// Sin fuente → color global de la ventana.
    Nada,
}

/// Resuelve `nombre[:versión]` instalado → (ruta, manifiesto), con las
/// validaciones de siempre (min_engine, tipo shader).
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
        eprintln!("error: no hay instalado '{spec}' (ver: bruma list)");
        std::process::exit(1);
    };
    let json = std::fs::read_to_string(pkg_path.join("wallpaper.json")).unwrap_or_else(|e| {
        eprintln!("error: paquete sin manifiesto: {e}");
        std::process::exit(1);
    });
    let manifest = bruma_package::Manifest::parse(&json).unwrap_or_else(|e| {
        eprintln!("error: manifiesto inválido: {e}");
        std::process::exit(1);
    });
    if let Some(min) = &manifest.min_engine
        && min.as_str() > bruma_core::ENGINE_VERSION
    {
        eprintln!(
            "error: el paquete requiere motor >= {min} y este es {}",
            bruma_core::ENGINE_VERSION
        );
        std::process::exit(1);
    }
    if manifest.wallpaper_type != "shader" {
        eprintln!(
            "error: el paquete es de tipo '{}' (reservado, aún sin implementar)",
            manifest.wallpaper_type
        );
        std::process::exit(1);
    }
    (pkg_path, manifest)
}

/// Resuelve una entrada de config (o flags legacy) a [`Fuente`].
///
/// `cli_overrides` son los `--param nombre=valor` del modo legacy (con
/// config, los params viven en la propia entrada). Los desconocidos son
/// error: los typos deben doler, no ignorarse.
fn resolver_fuente(
    oc: &config::OutputConfig,
    cli_overrides: &[(String, f32)],
    pkg_cache: &mut std::collections::HashMap<
        String,
        (std::path::PathBuf, bruma_package::Manifest),
    >,
    runtime_params: &mut Vec<bruma_runtime::ParamValue>,
    manifest_fps: &mut Option<u32>,
) -> Fuente {
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
        // Overrides: config primero, CLI encima; nombre → posición.
        let mut pares = config::params_a_pares(&oc.params).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(2);
        });
        pares.extend(cli_overrides.iter().cloned());
        let mut overrides = Vec::new();
        for (name, value) in pares {
            match manifest.params.iter().position(|p| p.name == name) {
                Some(pos) => overrides.push((pos, value.clamp(0.0, 1.0))),
                None => {
                    eprintln!("error: '{spec}' no declara el parámetro '{name}'");
                    std::process::exit(2);
                }
            }
        }
        return Fuente::Shader {
            path: pkg_path.join(&manifest.entry).display().to_string(),
            overrides,
        };
    }
    if let Some(p) = &oc.shader {
        return Fuente::Shader {
            path: p.clone(),
            overrides: Vec::new(),
        };
    }
    if let Some(p) = &oc.image {
        return Fuente::Imagen(p.clone());
    }
    match &oc.color {
        Some(c) => Fuente::Color(config::parse_color_hex(c).unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(2);
        })),
        None => Fuente::Nada,
    }
}

fn run_command(args: &[String]) {
    let mut color_arg: Option<&str> = None;
    // `--gpu` fuerza init de GPU aunque la fuente elegida sea color
    // (útil para calentar shaders/driver antes de cambiar de fuente).
    let mut gpu = false;
    let mut image_path: Option<String> = None;
    let mut shader_path: Option<String> = None;
    let mut package: Option<String> = None;
    let mut fps: Option<u32> = None;
    // Fase 3: `--param=VALOR` directo. Con manifiesto, `--param
    // nombre=VALOR` resuelve por nombre (ver más abajo).
    let mut param0: Option<f32> = None;
    let mut named_params: Vec<(String, f32)> = Vec::new();
    let mut config_path: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--gpu" => gpu = true,
            "--image" => {
                i += 1;
                image_path = Some(args.get(i).map(|s| s.to_string()).unwrap_or_else(|| {
                    eprintln!("--image requiere la ruta de una imagen");
                    std::process::exit(2);
                }));
            }
            "--shader" => {
                i += 1;
                shader_path = Some(args.get(i).map(|s| s.to_string()).unwrap_or_else(|| {
                    eprintln!("--shader requiere la ruta de un archivo .wgsl");
                    std::process::exit(2);
                }));
            }
            "--package" => {
                i += 1;
                package = Some(args.get(i).map(|s| s.to_string()).unwrap_or_else(|| {
                    eprintln!(
                        "--package requiere el nombre de un paquete instalado (ver: bruma list)"
                    );
                    std::process::exit(2);
                }));
            }
            "--fps" => {
                i += 1;
                fps = Some(args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("--fps requiere un número entero (ej: --fps 60)");
                    std::process::exit(2);
                }));
            }
            // Fase 5: config explícita (excluyente con flags de fuente).
            "--config" => {
                i += 1;
                config_path = Some(args.get(i).map(|s| s.to_string()).unwrap_or_else(|| {
                    eprintln!("--config requiere la ruta de un config.json");
                    std::process::exit(2);
                }));
            }
            // Fase 3: `--param=0.5` (parámetro 0). Fase 4: además
            // `--param=intensidad=0.5` cuando hay manifiesto que dé nombres.
            p if p.starts_with("--param=") => {
                // Tras el prefijo: `VALOR` (parámetro 0) o `NOMBRE=VALOR`.
                let rest = &p["--param=".len()..];
                match rest.split_once('=') {
                    // `--param=VALOR`: sin nombre → parámetro 0 (u_params0).
                    None => {
                        let value = rest.parse::<f32>().unwrap_or_else(|_| {
                            eprintln!("--param: '{rest}' no es un número (0..1)");
                            std::process::exit(2);
                        });
                        param0 = Some(value.clamp(0.0, 1.0));
                    }
                    // `--param=NOMBRE=VALOR`: con nombre (solo válido con
                    // paquete; se valida tras cargar el manifiesto).
                    Some((name, v)) => {
                        let value = v.parse::<f32>().unwrap_or_else(|_| {
                            eprintln!("--param: '{v}' no es un número (0..1)");
                            std::process::exit(2);
                        });
                        named_params.push((name.trim().to_owned(), value));
                    }
                }
            }
            "--param" => {
                eprintln!("--param requiere el formato --param[=nombre]=VALOR (0..1)");
                std::process::exit(2);
            }
            other => color_arg = Some(other),
        }
        i += 1;
    }

    // Modo de fuentes: flags explícitos = UNA fuente para todas las
    // salidas (comportamiento previo, ideal para pruebas). Sin flags,
    // manda la config persistente (auto-carga si existe).
    let explicit_source =
        shader_path.is_some() || image_path.is_some() || package.is_some() || color_arg.is_some();
    let cfg = match &config_path {
        Some(p) => {
            if explicit_source {
                eprintln!("error: --config es excluyente con --package/--shader/--image/color");
                std::process::exit(2);
            }
            let text = std::fs::read_to_string(p).unwrap_or_else(|e| {
                eprintln!("error: no se pudo leer {p}: {e}");
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
            log::info!("config explícita: {p}");
            Some(c)
        }
        None if explicit_source => None,
        None => match config::Config::load() {
            Ok(c) => {
                if c.is_some() {
                    log::info!("config persistente cargada (run sin flags usa la config)");
                }
                c
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        },
    };

    // Color GLOBAL (fallback de últimas: sin renderer y sin color de
    // salida). Con config, el color por salida vive en su Fuente::Color.
    let color = color_arg
        .map(|s| {
            let v = s.trim().trim_start_matches("0x").trim_start_matches('#');
            u32::from_str_radix(v, 16).unwrap_or_else(|_| {
                eprintln!("color inválido: {s} (usa hex, ej: 0x3B4252)");
                std::process::exit(2);
            })
        })
        .unwrap_or(0x2E_34_40); // gris azulado oscuro por defecto

    // Resolución de fuentes por salida (la única verdad para la factory).
    let mut pkg_cache: std::collections::HashMap<
        String,
        (std::path::PathBuf, bruma_package::Manifest),
    > = Default::default();
    let mut manifest_fps: Option<u32> = None;
    let mut runtime_params: Vec<bruma_runtime::ParamValue> = Vec::new();
    let mut tiene_contenido = false;

    let cli_overrides: Vec<(String, f32)> = {
        let mut v = named_params.clone();
        if let Some(v0) = param0 {
            // `--param=VALOR` sin manifiesto no toca esto: sin nombres,
            // la CLI lo resuelve después vía with_param.
            if !v.is_empty() || package.is_some() {
                v.push(("<posición 0>".to_owned(), v0));
                v.pop();
            }
        }
        v
    };

    let mut por_salida: Vec<(String, Fuente)> = Vec::new();
    let mut default_fuente = Fuente::Nada;
    if let Some(cfg) = &cfg {
        if let Some(d) = &cfg.default {
            default_fuente = resolver_fuente(
                d,
                &[],
                &mut pkg_cache,
                &mut runtime_params,
                &mut manifest_fps,
            );
            tiene_contenido |= d.has_content();
        }
        for (name, oc) in &cfg.outputs {
            let f = resolver_fuente(
                oc,
                &[],
                &mut pkg_cache,
                &mut runtime_params,
                &mut manifest_fps,
            );
            tiene_contenido |= oc.has_content();
            por_salida.push((name.clone(), f));
        }
    } else {
        // Legacy: una fuente para todas las salidas.
        if let Some(spec) = &package {
            let oc = config::OutputConfig {
                package: Some(spec.clone()),
                ..Default::default()
            };
            default_fuente = resolver_fuente(
                &oc,
                &cli_overrides,
                &mut pkg_cache,
                &mut runtime_params,
                &mut manifest_fps,
            );
            tiene_contenido = true;
        } else if let Some(p) = &shader_path {
            default_fuente = Fuente::Shader {
                path: p.clone(),
                overrides: Vec::new(),
            };
            tiene_contenido = true;
        } else if let Some(p) = &image_path {
            default_fuente = Fuente::Imagen(p.clone());
            tiene_contenido = true;
        } else {
            default_fuente = Fuente::Color(color);
        }
    }

    // Precedencia de FPS: flag > config > manifiesto > 30.
    let fps = fps
        .or_else(|| cfg.as_ref().and_then(|c| c.fps))
        .or(manifest_fps)
        .unwrap_or(30);

    log::info!(
        "bruma run — modo {}{}",
        if tiene_contenido && por_salida.is_empty() && !matches!(default_fuente, Fuente::Color(_)) {
            "shader animado"
        } else if tiene_contenido {
            "mixto por salida"
        } else {
            "color sólido"
        },
        if tiene_contenido {
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

    // Fase 5: la factory construye un renderer POR SALIDA. La GPU se
    // descubre una sola vez (GpuShared se clona barato); cada salida
    // recibe su propio renderer sobre su propia superficie. El error de
    // factory degrada esa salida a color sólido sin tumbar el resto.
    let mut shared_gpu: Option<bruma_renderer_wgpu::GpuShared> = None;
    // Con config, una salida puede declarar shader aunque `default` sea
    // color: la factory lo sabe vía `por_salida`/`default_f` (movidos
    // dentro). Si hay cualquier shader/imagen en juego, hace falta GPU.
    let hay_gpu_en_fuentes = |f: &Fuente| matches!(f, Fuente::Shader { .. } | Fuente::Imagen(_));
    let necesita_gpu = hay_gpu_en_fuentes(&default_fuente)
        || por_salida.iter().any(|(_, f)| hay_gpu_en_fuentes(f));
    let _ = &mut gpu;
    if necesita_gpu || gpu {
        match bruma_renderer_wgpu::GpuShared::new() {
            Ok(g) => shared_gpu = Some(g),
            Err(e) => {
                // Sin GPU, el modo color sólido sigue siendo útil.
                log::warn!("sin GPU ({e}); todas las salidas en color sólido");
            }
        }
    }

    // Callback de hot-reload compartido por todas las salidas (una sola
    // notificación aunque haya N superficies recargando el mismo shader).
    let notifier = std::rc::Rc::new(bruma_platform::DesktopNotifier::new());
    let notify_state: std::rc::Rc<std::cell::RefCell<Option<(String, std::time::Instant)>>> =
        std::rc::Rc::new(std::cell::RefCell::new(None));

    // Modelo de fuentes COMPARTIDO (Fase 5): la factory lo lee en cada
    // construcción de renderer (arranque y recargas); el handler de
    // SIGHUP lo actualiza. Rc<RefCell> porque ambos viven en el hilo del
    // bucle (sin cruces de hilos, sin Mutex).
    struct ModeloFuentes {
        por_salida: Vec<(String, Fuente)>,
        default: Fuente,
    }
    let modelo_fuentes: std::rc::Rc<std::cell::RefCell<ModeloFuentes>> =
        std::rc::Rc::new(std::cell::RefCell::new(ModeloFuentes {
            por_salida,
            default: default_fuente,
        }));

    let modelo_f = modelo_fuentes.clone();
    let notifier_f = notifier.clone();
    let notify_state_f = notify_state.clone();
    window.set_renderer_factory(Box::new(move |handles| {
        let m = modelo_f.borrow();
        let fuente = handles
            .output_name
            .as_deref()
            .and_then(|n| m.por_salida.iter().find(|(name, _)| name == n))
            .map(|(_, f)| f)
            .unwrap_or(&m.default);
        let display = handles.display_ptr;
        let surface = handles.surface_ptr;
        match fuente {
            Fuente::Shader { path, overrides } => {
                let mut renderer = unsafe {
                    bruma_renderer_wgpu::AnimatedRenderer::on_shared(
                        shared_gpu.as_ref().ok_or("sin GPU")?,
                        display,
                        surface,
                        std::path::Path::new(path),
                    )
                }
                .map_err(|e| e.to_string())?;
                if !overrides.is_empty() {
                    // Overrides de ESTA salida (resueltos por nombre
                    // contra el manifiesto en la CLI).
                    renderer.set_param_overrides(overrides.clone());
                }

                // Aviso de "shader rechazado" visible sin terminal: D-Bus
                // best-effort con debounce de 2 s (compartido por salidas).
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
            Fuente::Imagen(path) => Ok(bruma_platform::FactoryRenderer::renderer(Box::new(
                unsafe {
                    bruma_renderer_wgpu::ImageRenderer::on_shared(
                        shared_gpu.as_ref().ok_or("sin GPU")?,
                        display,
                        surface,
                        std::path::Path::new(path),
                    )
                }
                .map_err(|e| e.to_string())?,
            ))),
            Fuente::Color(c) => Ok(bruma_platform::FactoryRenderer::color(Color::from_rgb_u32(
                *c,
            ))),
            Fuente::Nada => Err("sin fuente para esta salida".to_owned()),
        }
    }));

    let reports = window.present_once().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    for r in &reports {
        let modo = if r.gpu { "gpu" } else { "color" };
        log::info!(
            "fondo activo en {:?}: {}x{}px, escala {} ({modo}) — Ctrl-C para salir",
            r.name,
            r.width,
            r.height,
            r.scale
        );
    }

    // Recarga de config en caliente: SIGHUP → re-parsear y actualizar el
    // modelo de fuentes (la plataforma reconstituye los renderers vía
    // factory y repinta). Alcance: fuentes y params por salida; un cambio
    // de fps espera al próximo arranque (el runtime no se muta en caliente).
    if cfg.is_some() {
        let notifier_h = notifier.clone();
        let modelo_h = modelo_fuentes.clone();
        window.on_config_reload(Box::new(move || {
            let cfg_new = match config::Config::load() {
                // Sin config o rota: el wallpaper SIGUE con la actual.
                Ok(Some(c)) => c,
                Ok(None) => return false,
                Err(e) => {
                    log::warn!("config rechazada: {e}");
                    notifier_h.shader_rejected(&format!("config: {e}"));
                    return false;
                }
            };
            let mut pkg_cache = Default::default();
            let mut manifest_fps = None;
            let mut params = Vec::new();
            let mut m = modelo_h.borrow_mut();
            if let Some(d) = &cfg_new.default {
                m.default = resolver_fuente(d, &[], &mut pkg_cache, &mut params, &mut manifest_fps);
            }
            m.por_salida = cfg_new
                .outputs
                .iter()
                .map(|(name, oc)| {
                    (
                        name.clone(),
                        resolver_fuente(oc, &[], &mut pkg_cache, &mut params, &mut manifest_fps),
                    )
                })
                .collect();
            true
        }));
    }

    // El modo del bucle lo decide el renderer instalado: animado (shader)
    // conduce frames a la cadencia del runtime; estático (color, imagen,
    // triángulo) solo atiende eventos.
    let wants_animation = window.wants_animation();
    if wants_animation {
        let mut runtime = bruma_runtime::BasicRuntime::new(fps);
        if !runtime_params.is_empty() {
            // Defaults del manifiesto; los overrides POR SALIDA ya viven
            // en cada renderer (la animación sigue sincronizada: un solo
            // runtime, un solo reloj).
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

/// Logger mínimo a stderr, suficiente para la Fase 1.
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
