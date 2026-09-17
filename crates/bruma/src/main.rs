//! CLI del motor bruma.
//!
//! Estado: **Fase 4**. Subcomandos: `run` (color, triángulo, imagen,
//! shader o paquete instalado), `validate`/`install`/`list`/`pack`
//! (formato `.wallpaper`). `new` llegará con la Fase 6.

use bruma_core::Color;
use std::time::Duration;

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
        Some("--version") | Some("-V") | None => {
            println!(
                "bruma v{} — motor libre de wallpapers animados para Wayland",
                bruma_core::ENGINE_VERSION
            );
            if std::env::args().count() == 1 {
                eprintln!(
                    "\nUso: bruma <COMANDO>\n\nComandos:\n  run [opciones] [color]   Fondo detrás de las ventanas\n  validate PAQUETE         Valida un archivo .wallpaper\n  install PAQUETE          Instala un paquete\n  list                     Lista los paquetes instalados\n  pack DIRECTORIO          Empaqueta un directorio en .wallpaper"
                );
            }
        }
        Some(other) => {
            eprintln!(
                "comando desconocido: {other}\n\nComandos disponibles:\n  run | validate | install | list | pack"
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

fn run_command(args: &[String]) {
    let mut color_arg: Option<&str> = None;
    let mut gpu = false;
    let mut image_path: Option<String> = None;
    let mut shader_path: Option<String> = None;
    let mut package: Option<String> = None;
    let mut fps: Option<u32> = None;
    // Fase 3: `--param=VALOR` directo. Con manifiesto, `--param
    // nombre=VALOR` resuelve por nombre (ver más abajo).
    let mut param0: Option<f32> = None;
    let mut named_params: Vec<(String, f32)> = Vec::new();
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

    let color = color_arg
        .map(|s| {
            let v = s.trim().trim_start_matches("0x").trim_start_matches('#');
            u32::from_str_radix(v, 16).unwrap_or_else(|_| {
                eprintln!("color inválido: {s} (usa hex, ej: 0x3B4252)");
                std::process::exit(2);
            })
        })
        .unwrap_or(0x2E_34_40); // gris azulado oscuro por defecto

    // Resolución del manifiesto si hay paquete: valida min_engine y fps
    // por defecto; los parámetros con nombre se convierten a posiciones.
    let mut manifest_fps: Option<u32> = None;
    let mut manifest_params: Vec<bruma_runtime::ParamValue> = Vec::new();
    if let Some(name) = &package {
        let store = default_store();
        let mut found = None;
        // El nombre puede llevar versión ("paquete:1.2.0"); sin versión,
        // la más alta instalada.
        let (pkg, want_ver) = match name.split_once(':') {
            Some((p, v)) => (p, Some(v.to_owned())),
            None => (name.as_str(), None),
        };
        for (n, v, path) in store.installed() {
            if n == *pkg && want_ver.as_deref().is_none_or(|w| v == *w) {
                found = Some((v, path));
            }
        }
        let Some((ver, pkg_path)) = found else {
            eprintln!("error: no hay instalado '{name}' (ver: bruma list)");
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
        // Parámetros: los del manifiesto en orden; los --param nombre=VALOR
        // los sobreescriben por nombre. `--param=VALOR` sobreescribe el
        // primero (u_params0).
        manifest_params = manifest
            .params
            .iter()
            .map(|p| bruma_runtime::ParamValue {
                name: p.name.clone(),
                value: p.default,
            })
            .collect();
        for (name, value) in named_params.drain(..) {
            match manifest_params.iter_mut().find(|p| p.name == name) {
                Some(p) => p.value = value.clamp(0.0, 1.0),
                None => {
                    eprintln!("error: el paquete no declara el parámetro '{name}'");
                    std::process::exit(2);
                }
            }
        }
        if let Some(v) = param0
            && let Some(first) = manifest_params.first_mut()
        {
            first.value = v;
        }
        manifest_fps = manifest.fps;
        shader_path = Some(pkg_path.join(&manifest.entry).display().to_string());
        log::info!(
            "paquete '{}' {} — {}",
            manifest.title,
            ver,
            pkg_path.display()
        );
    }

    let fps = fps.unwrap_or(manifest_fps.unwrap_or(30));

    log::info!(
        "bruma run — modo {}{}",
        if shader_path.is_some() {
            "shader animado"
        } else if image_path.is_some() {
            "imagen"
        } else if gpu {
            "wgpu"
        } else {
            "color sólido"
        },
        if shader_path.is_some() {
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
    let mut gpu_err = |e: String| -> String {
        log::warn!("GPU compartida: {e}");
        e
    };
    let _ = &mut gpu_err;
    if shader_path.is_some() || image_path.is_some() || gpu {
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

    let shader_for_factory = shader_path.clone();
    let image_for_factory = image_path.clone();
    let notifier_f = notifier.clone();
    let notify_state_f = notify_state.clone();
    window.set_renderer_factory(Box::new(move |handles| {
        let display = handles.display_ptr;
        let surface = handles.surface_ptr;
        let renderer: Box<dyn bruma_renderer_wgpu::FrameRendererAlias> =
            if let Some(path) = &shader_for_factory {
                let mut renderer = unsafe {
                    bruma_renderer_wgpu::AnimatedRenderer::on_shared(
                        shared_gpu.as_ref().ok_or("sin GPU")?,
                        display,
                        surface,
                        std::path::Path::new(path),
                    )
                }
                .map_err(|e| e.to_string())?;

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
                Box::new(renderer)
            } else if let Some(path) = &image_for_factory {
                Box::new(
                    unsafe {
                        bruma_renderer_wgpu::ImageRenderer::on_shared(
                            shared_gpu.as_ref().ok_or("sin GPU")?,
                            display,
                            surface,
                            std::path::Path::new(path),
                        )
                    }
                    .map_err(|e| e.to_string())?,
                )
            } else {
                Box::new(
                    unsafe {
                        bruma_renderer_wgpu::WgpuRenderer::on_shared(
                            shared_gpu.as_ref().ok_or("sin GPU")?,
                            display,
                            surface,
                        )
                    }
                    .map_err(|e| e.to_string())?,
                )
            };
        Ok(renderer)
    }));

    let reports = window.present_once().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    for r in &reports {
        let modo = if r.gpu { "gpu" } else { "color" };
        log::info!(
            "fondo activo en {:?}: {}x{}px ({modo}) — Ctrl-C para salir",
            r.name,
            r.width,
            r.height
        );
    }

    // El modo del bucle lo decide el renderer instalado: animado (shader)
    // conduce frames a la cadencia del runtime; estático (color, imagen,
    // triángulo) solo atiende eventos.
    let wants_animation = window.wants_animation();
    if wants_animation {
        let mut runtime = bruma_runtime::BasicRuntime::new(fps);
        if !manifest_params.is_empty() {
            runtime.set_params(manifest_params);
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
