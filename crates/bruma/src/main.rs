//! CLI del motor bruma.
//!
//! Estado: **Fase 3**. Subcomando `run` con cuatro modos: color sólido,
//! triángulo (prueba de pipeline), imagen y shader animado (con hot-reload
//! de WGSL y límite de FPS). Los demás subcomandos llegarán con sus
//! fases: `validate`/`install` (F4), `new`/`pack` (F6).

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
        Some("--version") | Some("-V") | None => {
            println!(
                "bruma v{} — motor libre de wallpapers animados para Wayland",
                bruma_core::ENGINE_VERSION
            );
            if std::env::args().count() == 1 {
                eprintln!(
                    "\nUso: bruma <COMANDO>\n\nComandos:\n  run [opciones] [color]  Fondo detrás de las ventanas\n\nOpciones de run:\n  --shader ARCHIVO.wgsl  Shader animado con hot-reload (Fase 3)\n  --image IMAGEN         Imagen estática a pantalla completa\n  --gpu                  Triángulo de prueba del pipeline wgpu\n  --fps N                Límite de FPS del shader animado (por defecto 30)"
                );
            }
        }
        Some(other) => {
            eprintln!(
                "comando desconocido: {other}\n\nComandos disponibles:\n  run [opciones] [color]  Fondo detrás de las ventanas"
            );
            std::process::exit(2);
        }
    }
}
fn run_command(args: &[String]) {
    let mut color_arg: Option<&str> = None;
    let mut gpu = false;
    let mut image_path: Option<String> = None;
    let mut shader_path: Option<String> = None;
    let mut fps: u32 = 30;
    let mut param0: Option<f32> = None;
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
            "--fps" => {
                i += 1;
                fps = args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("--fps requiere un número entero (ej: --fps 60)");
                    std::process::exit(2);
                });
            }
            // Fase 3: solo el primer parámetro, sin namespacing; el
            // análisis de `nombre=valor` completo llega con el manifiesto.
            p if p.starts_with("--param") => {
                let value = p
                    .split_once('=')
                    .and_then(|(_, v)| v.parse::<f32>().ok())
                    .unwrap_or_else(|| {
                        eprintln!("--param requiere el formato --param=VALOR (0..1)");
                        std::process::exit(2);
                    });
                param0 = Some(value.clamp(0.0, 1.0));
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

    if let Some(path) = &shader_path {
        // Fase 3: shader animado con hot-reload. El renderer se crea ANTES
        // del primer configure para que pinte él el primer frame; el bucle
        // animado lo conduce el runtime (ver abajo).
        let mut renderer = unsafe {
            bruma_renderer_wgpu::AnimatedRenderer::new_wayland(
                window.display_ptr(),
                window.surface_ptr(),
                std::path::Path::new(path),
            )
        }
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });

        // El aviso de "shader rechazado" debe verse aunque bruma corra
        // como servicio sin terminal: notificación de escritorio vía
        // D-Bus (best-effort; sin bus de sesión es un no-op). El dedup
        // de eventos ya lo hace el renderer; aquí solo amortiguamos
        // notificaciones idénticas a menos de 2 s (típico: varios
        // guardados seguidos del mismo autosave).
        let notifier = std::rc::Rc::new(bruma_platform::DesktopNotifier::new());
        let mut last_notify: Option<(String, std::time::Instant)> = None;
        const MIN_NOTIFY_GAP: Duration = Duration::from_secs(2);
        renderer.set_reload_callback(Box::new(move |event| {
            use bruma_renderer_wgpu::ReloadEvent;
            match event {
                ReloadEvent::Rejected { error } => {
                    // naga puede dar errores largos multi-línea: nos
                    // quedamos con la primera línea y recortamos a 140
                    // bytes para que la burbuja sea legible.
                    let first_line = error.lines().next().unwrap_or(error);
                    let mut brief: String = first_line.chars().take(140).collect();
                    if brief.len() < first_line.len() {
                        brief.push('…');
                    }
                    let now = std::time::Instant::now();
                    let fresh = last_notify.as_ref().is_none_or(|(m, t)| {
                        now.duration_since(*t) >= MIN_NOTIFY_GAP || *m != brief
                    });
                    if fresh {
                        last_notify = Some((brief.clone(), now));
                        notifier.shader_rejected(&brief);
                    }
                }
                ReloadEvent::Recovered => {
                    last_notify = None;
                    notifier.shader_recovered();
                }
                ReloadEvent::Applied => {}
            }
        }));
        window.set_frame_renderer(Box::new(renderer));
    } else if let Some(path) = &image_path {
        // Fase 2, paso 2: imagen a pantalla completa (hito de la fase).
        let renderer = unsafe {
            bruma_renderer_wgpu::ImageRenderer::new_wayland(
                window.display_ptr(),
                window.surface_ptr(),
                std::path::Path::new(path),
            )
        }
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });
        window.set_frame_renderer(Box::new(renderer));
    } else if gpu {
        // Fase 2: renderer wgpu (triángulo de bienvenida). Hay que crearlo
        // ANTES del primer configure, para que pinte él el primer frame.
        let renderer = unsafe {
            bruma_renderer_wgpu::WgpuRenderer::new_wayland(
                window.display_ptr(),
                window.surface_ptr(),
            )
        }
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });
        window.set_frame_renderer(Box::new(renderer));
    }

    let (w, h) = window.present_once().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
    log::info!("fondo activo: {w}x{h}px — Ctrl-C para salir");

    // El modo del bucle lo decide el renderer instalado: animado (shader)
    // conduce frames a la cadencia del runtime; estático (color, imagen,
    // triángulo) solo atiende eventos.
    let wants_animation = window.wants_animation();
    if wants_animation {
        let mut runtime = bruma_runtime::BasicRuntime::new(fps);
        if let Some(v) = param0 {
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
