# bruma 🌫️

> **Motor libre de wallpapers animados para Wayland** — shaders WGSL,
> eficiente, con formato abierto.

`bruma` es un motor libre y gratuito para tener fondos de pantalla animados
en Linux/Wayland: shaders WGSL a 60 fps con coste mínimo de CPU, un formato
de paquete abierto (`.wallpaper`) y herramientas para que cualquiera cree y
comparta sus fondos.

## Estado del proyecto

Fases 0–5 completadas: shaders WGSL animados con hot-reload, imagen y
color sólido, formato de paquete `.wallpaper` (instalación segura con
`validate`/`install`), y escritorio completo — multi-monitor con fondo
distinto por pantalla, pausa automática en fullscreen/bloqueo/batería,
escala HiDPI por salida y recarga de config en caliente. En construcción:
herramientas para creadores (Fase 6) — ver [PLAN.md](PLAN.md).

```bash
bruma run                        # sin flags: usa la config persistente
bruma run 0x3B4252               # color hex a elección (todas las pantallas)
bruma run --gpu                  # triángulo WGSL (prueba de pipeline)
bruma run --image foto.png       # imagen a pantalla completa
bruma run --shader hola.wgsl     # shader animado con hot-reload
bruma run --shader hola.wgsl --fps 60 --param=0.5  # ritmo y parámetros
bruma run --package mi-onda --param=intensidad=0.9  # paquete instalado
```

## Escritorio completo (Fase 5)

Una superficie por salida (multi-monitor con GPU compartida), pausa
automática en fullscreen (solo esa pantalla), sesión bloqueada y
batería (global, vía logind/UPower). Config persistente por pantalla:

```bash
bruma config init                # crea ~/.config/bruma/config.json
bruma config show                # qué va a correr y dónde
bruma service install            # arranca con la sesión (systemd --user)
bruma service remove             # lo quita
```

Ejemplo de `config.json` — cada pantalla lo suyo, params distintos del
mismo paquete (la animación va sincronizada: un solo reloj):

```json
{
  "default": { "package": "onda-bruma-demo", "params": { "intensidad": 0.9 } },
  "outputs": {
    "eDP-1":    { "package": "onda-bruma-demo", "params": { "intensidad": 0.15 } },
    "HDMI-A-1": { "color": "#1d2021" }
  },
  "fps": 30
}
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
| 4 | formato `.wallpaper` | ✅ `pack` → `validate` → `install` → `run --package` |
| 5 | multi-monitor, pausas | ✅ wallpaper por pantalla, HiDPI, hotplug |
| 6 | plantillas para creadores | un extraño crea y comparte |
| 7 | galería web (WASM/WebGPU) | previews vivos en el navegador |

## Compilar

```bash
cargo build          # compilar todo
cargo test           # ejecutar los tests
cargo run -p bruma   # probar la CLI
```

## Paquetes `.wallpaper`

Un wallpaper se distribuye como un zip con `wallpaper.json`, una preview y
sus shaders:

```bash
bruma pack mi-fondo/ -o mi-fondo.wallpaper   # empaquetar un directorio
bruma validate mi-fondo.wallpaper            # validar (¿es seguro y correcto?)
bruma install mi-fondo.wallpaper             # instalar en ~/.local/share/bruma
bruma list                                   # ver lo instalado
bruma run --package mi-fondo                 # usarlo como fondo
bruma run --package mi-fondo --param=velocidad=0.8
```

El manifiesto declara parámetros con nombre que la CLI valida y resuelve:

```json
{
  "format": 1,
  "type": "shader",
  "title": "Mi fondo",
  "version": "0.1.0",
  "entry": "shader.wgsl",
  "preview": "preview.png",
  "params": [{ "name": "velocidad", "label": "Velocidad", "default": 0.5 }],
  "fps": 30
}
```

La validación rechaza paquetes maliciosos: rutas con path traversal,
symlinks dentro del zip y bombas de descompresión.

## Licencia

Doble licencia MIT / Apache-2.0, como el ecosistema Rust.
