# bruma 🌫️

> **Motor libre de wallpapers animados para Wayland** — shaders WGSL,
> eficiente, con formato abierto.

`bruma` es un motor libre y gratuito para tener fondos de pantalla animados
en Linux/Wayland: shaders WGSL a 60 fps con coste mínimo de CPU, un formato
de paquete abierto (`.wallpaper`) y herramientas para que cualquiera cree y
comparta sus fondos.

## Estado del proyecto

Fase 3 completada: `bruma run --shader hola.wgsl` ejecuta un shader
animado (wgpu/Vulkan, límite de FPS, CPU ~4% a 30 fps en 2560x1440) y el
archivo se recarga **en caliente** al editarlo, sin reiniciar. También:
imagen a pantalla completa (Fase 2), color sólido (Fase 1) y triángulo de
prueba. En construcción — ver [PLAN.md](PLAN.md).

```bash
bruma run                        # fondo de color sólido
bruma run 0x3B4252               # color hex a elección
bruma run --gpu                  # triángulo WGSL (prueba de pipeline)
bruma run --image foto.png       # imagen a pantalla completa
bruma run --shader hola.wgsl     # shader animado con hot-reload
bruma run --shader hola.wgsl --fps 60 --param=0.5  # ritmo y parámetros
```

## Arquitectura

```
crates/
├── bruma-core/            # tipos base, sin dependencias externas
├── bruma-package/         # formato .wallpaper (zip) — Fase 4
├── bruma-runtime/         # contrato de runtime (tiempo, mouse...) — Fase 3
├── bruma-renderer/        # contrato de renderizado — Fase 2
├── bruma-platform/        # Wayland wlr-layer-shell — Fase 1
└── bruma/                 # CLI: run, install, new, validate
```

Regla de oro: `core`, `package` y `runtime` **nunca** dependerán de
wgpu, Wayland ni Steam (ver [DECISIONS.md](DECISIONS.md)).

## Roadmap

| Fase | Qué | Demo |
|---|---|---|
| 0 | workspace, CI, licencias, docs | ✅ esta estructura |
| 1 | ventana de fondo Wayland (niri primero) | ✅ color sólido detrás de todo |
| 2 | wgpu + WGSL | ✅ imagen a pantalla completa |
| 3 | contrato de runtime + hot-reload | ✅ shader animado editado en vivo |
| 4 | formato `.wallpaper` | `bruma validate` / `install` |
| 5 | multi-monitor, pausas | wallpaper por pantalla |
| 6 | plantillas para creadores | un extraño crea y comparte |
| 7 | galería web (WASM/WebGPU) | previews vivos en el navegador |

## Compilar

```bash
cargo build          # compilar todo
cargo test           # ejecutar los tests
cargo run -p bruma   # probar la CLI
```

## Licencia

Doble licencia MIT / Apache-2.0, como el ecosistema Rust.
