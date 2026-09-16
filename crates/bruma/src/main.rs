//! CLI del motor bruma.
//!
//! Estado: **Fase 1**. Subcomando `run` disponible: muestra un fondo de
//! color sólido detrás de todas las ventanas (wlr-layer-shell). Los demás
//! subcomandos llegarán con sus fases: `validate`/`install` (F4),
//! `new`/`pack` (F6).

use bruma_core::Color;

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
                    "\nUso: bruma <COMANDO>\n\nComandos:\n  run [--gpu] [color]  Fondo detrás de las ventanas (sólido o triángulo wgpu)"
                );
            }
        }
        Some(other) => {
            eprintln!(
                "comando desconocido: {other}\n\nComandos disponibles:\n  run [--gpu] [color]  Fondo detrás de las ventanas"
            );
            std::process::exit(2);
        }
    }
}
fn run_command(args: &[String]) {
    let mut color_arg: Option<&str> = None;
    let mut gpu = false;
    for arg in args {
        match arg.as_str() {
            "--gpu" => gpu = true,
            other => color_arg = Some(other),
        }
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
        "bruma run — modo {}",
        if gpu { "wgpu" } else { "color sólido" }
    );
    let mut window = bruma_platform::BackgroundWindow::new(Color::from_rgb_u32(color))
        .unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        });

    if gpu {
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

    window.run().unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });
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
