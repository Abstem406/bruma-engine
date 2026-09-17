//! # bruma-runtime
//!
//! Contrato de runtime: la API que un wallpaper puede consumir (tiempo,
//! delta, resolución, mouse, parámetros, audio opcional).
//!
//! Estado: **Fase 3**. Este crate es **puro**: define el contrato
//! compartido por el runtime nativo (wgpu) y el futuro runtime web
//! (WASM, Fase 7), sin dependencias (D6).
//!
//! La implementación nativa vive en `bruma-renderer-wgpu`; la demo de
//! la fase es un shader animado editado en vivo sin reiniciar.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

/// Estado de un frame: lo que el runtime le entrega al renderer en
/// cada paso de animación. Es el mismo concepto de un uniform block
/// estándar de shadertoy / LiveWallpaper: todo lo que un shader
/// necesita saber del mundo.
///
/// Definido aquí (crate puro) para que la futura galería web lo
/// reutilice tal cual: son solo números.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameState {
    /// Segundos desde que arrancó el wallpaper.
    pub time: f32,
    /// Segundos desde el frame anterior (para físicas estables).
    pub delta: f32,
    /// Ancho del área de dibujo, en píxeles de buffer.
    pub width: u32,
    /// Alto del área de dibujo, en píxeles de buffer.
    pub height: u32,
    /// Posición X del cursor (píxeles; `-1.0` = desconocida).
    pub mouse_x: f32,
    /// Posición Y del cursor (píxeles; `-1.0` = desconocida).
    pub mouse_y: f32,
    /// Valores planos de los primeros parámetros declarados (en WGSL:
    /// `u_params0..3`). Los nombres los lleva el manifiesto (Fase 4) y
    /// [`WallpaperRuntime::params`]; a la GPU solo llegan números.
    pub params: [f32; 4],
}

impl Default for FrameState {
    fn default() -> Self {
        FrameState {
            time: 0.0,
            delta: 0.0,
            width: 0,
            height: 0,
            mouse_x: -1.0,
            mouse_y: -1.0,
            params: [0.0; 4],
        }
    }
}

/// Parámetros declarados por un wallpaper y ajustables por el usuario.
///
/// Definidos en la Fase 3; la UI generada desde el manifiesto llega
/// con la Fase 6 (herramientas para creadores). `value` es un slider
/// continuo 0..=1: suficiente para animar shaders sin un sistema de
/// tipos grande.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamValue {
    pub name: String,
    pub value: f32,
}

/// Contrato del runtime de un wallpaper animado.
///
/// El motor llama a [`Self::begin_frame`] antes de pintar cada frame y
/// a [`Self::end_frame`] tras presentarlo. El runtime acumula el
/// tiempo y decide el ritmo (límite de FPS); el renderer solo pinta
/// el frame que el runtime pide.
///
/// Implementaciones: nativo en `bruma-renderer-wgpu`, WASM en Fase 7.
pub trait WallpaperRuntime {
    /// Avanza el estado al siguiente frame.
    ///
    /// `now` es el instante monótono actual; el runtime calcula
    /// `delta` y `time` y aplica el límite de FPS. Devuelve una
    /// decisión Skip cuando el ritmo dice que aún no toca pintar y el
    /// llamador puede dormir hasta el próximo deadline.
    fn begin_frame(&mut self, now: std::time::Instant) -> FrameDecision;

    /// Frames por segundo objetivo (0 = sin límite, corre al ritmo de
    /// vblank/swapchain).
    fn target_fps(&self) -> u32;

    /// Estado del frame actual (válido tras `begin_frame`).
    fn state(&self) -> FrameState;

    /// Parámetros ajustables actuales (por nombre).
    fn params(&self) -> &[ParamValue];

    /// Actualiza un parámetro por nombre; `false` si no existe.
    fn set_param(&mut self, name: &str, value: f32) -> bool;

    /// Trabajo extra tras presentar (estadísticas, pausas...).
    /// Por defecto no hace nada.
    fn end_frame(&mut self) {}
}

/// Resultado de [`WallpaperRuntime::begin_frame`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameDecision {
    /// Toca pintar: el estado ya está actualizado.
    Draw,
    /// Aún no: dormir como máximo hasta `deadline`.
    Skip {
        /// Instante absoluto en que vence el próximo frame.
        deadline: std::time::Instant,
    },
}

/// Ayudante para tests y usuarios: duración mínima entre frames para
/// un FPS objetivo (0 fps => sin límite => `Duration::ZERO`).
pub fn frame_interval(target_fps: u32) -> Duration {
    if target_fps == 0 {
        Duration::ZERO
    } else {
        Duration::from_nanos(1_000_000_000 / u64::from(target_fps))
    }
}

/// Runtime de propósito general: reloj acumulado, límite de FPS,
/// posición del mouse y parámetros ajustables. Cubre la gran mayoría
/// de los wallpapers; los casos especiales (escena con física propia,
/// pausa por visibilidad...) implementan [`WallpaperRuntime`] directo.
///
/// Siempre declara al menos un parámetro `"param0"`, el mismo que
/// expone el shader de demo (`u_params0`): así la CLI puede animarlo
/// sin conocer el wallpaper concreto.
#[derive(Debug, Clone)]
pub struct BasicRuntime {
    fps: u32,
    last: Option<Instant>,
    state: FrameState,
    params: Vec<ParamValue>,
    paused: bool,
}

impl BasicRuntime {
    /// Runtime nuevo con el límite de FPS dado (0 = sin límite).
    pub fn new(fps: u32) -> Self {
        BasicRuntime {
            fps,
            last: None,
            state: FrameState::default(),
            params: vec![ParamValue {
                name: "param0".to_owned(),
                value: 0.0,
            }],
            paused: false,
        }
    }

    /// Marca el runtime como pausado desde el inicio.
    pub fn paused(mut self) -> Self {
        self.paused = true;
        self
    }

    /// Pausa o reanuda la animación. En pausa, el tiempo NO avanza y
    /// el runtime pide despertar solo una vez por segundo (el socket
    /// de Wayland despierta el bucle igual con cualquier evento).
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Fija un valor inicial para un parámetro (p. ej. desde la CLI).
    /// Ignora en silencio los nombres desconocidos: `set_param` sí
    /// reporta; este constructor no tiene a quién reportarle.
    pub fn with_param(mut self, name: &str, value: f32) -> Self {
        let _ = self.set_param(name, value);
        self
    }

    /// Reemplaza los parámetros declarados por los dados (máx 4: los
    /// que caben en el uniform block). Lo usa la CLI al cargar un
    /// paquete, cuyos nombres vienen del manifiesto.
    pub fn set_params(&mut self, mut params: Vec<ParamValue>) {
        params.truncate(4);
        self.params = params;
    }

    /// Fija el valor del parámetro en la posición `index` (0..3), que
    /// es como llega a la GPU (`u_params0..3`).
    pub fn set_param_at(&mut self, index: usize, value: f32) {
        if let Some(p) = self.params.get_mut(index) {
            p.value = value.clamp(0.0, 1.0);
        }
    }

    /// Actualiza la posición del cursor (lo llama la plataforma). Pasa
    /// por `state.mouse_x/y`, que el renderer copia a los uniforms.
    pub fn set_mouse(&mut self, x: f32, y: f32) {
        self.state.mouse_x = x;
        self.state.mouse_y = y;
    }

    /// Actualiza la resolución del área de dibujo (lo llama la
    /// plataforma tras cada configure).
    pub fn set_resolution(&mut self, width: u32, height: u32) {
        self.state.width = width;
        self.state.height = height;
    }
}

impl WallpaperRuntime for BasicRuntime {
    fn begin_frame(&mut self, now: Instant) -> FrameDecision {
        // En pausa el tiempo está congelado: no tocamos `last` ni `time`.
        if self.paused {
            return FrameDecision::Skip {
                deadline: now + Duration::from_secs(1),
            };
        }
        let interval = frame_interval(self.fps);
        if let Some(last) = self.last
            && now.duration_since(last) < interval
        {
            return FrameDecision::Skip {
                deadline: last + interval,
            };
        }
        let delta = self
            .last
            .map_or(0.0, |l| now.duration_since(l).as_secs_f32());
        self.state.time += delta;
        self.state.delta = delta;
        self.last = Some(now);
        FrameDecision::Draw
    }

    fn target_fps(&self) -> u32 {
        self.fps
    }

    fn state(&self) -> FrameState {
        FrameState {
            params: std::array::from_fn(|i| self.params.get(i).map_or(0.0, |p| p.value)),
            ..self.state
        }
    }

    fn params(&self) -> &[ParamValue] {
        &self.params
    }

    fn set_param(&mut self, name: &str, value: f32) -> bool {
        match self.params.iter_mut().find(|p| p.name == name) {
            Some(p) => {
                p.value = value.clamp(0.0, 1.0);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runtime mínimo para validar el contrato con tests: reloj con
    /// límite de FPS y acumulación de tiempo.
    struct TestRuntime {
        last: Option<Instant>,
        time: f32,
        fps: u32,
    }

    impl WallpaperRuntime for TestRuntime {
        fn begin_frame(&mut self, now: Instant) -> FrameDecision {
            let interval = frame_interval(self.fps);
            if let Some(last) = self.last
                && now.duration_since(last) < interval
            {
                return FrameDecision::Skip {
                    deadline: last + interval,
                };
            }
            self.delta_applied(now);
            FrameDecision::Draw
        }

        fn target_fps(&self) -> u32 {
            self.fps
        }

        fn state(&self) -> FrameState {
            FrameState {
                time: self.time,
                ..FrameState::default()
            }
        }

        fn params(&self) -> &[ParamValue] {
            &[]
        }

        fn set_param(&mut self, _name: &str, _value: f32) -> bool {
            false
        }
    }

    impl TestRuntime {
        fn delta_applied(&mut self, now: Instant) {
            let delta = self
                .last
                .map_or(0.0, |l| now.duration_since(l).as_secs_f32());
            self.time += delta;
            self.last = Some(now);
        }
    }

    #[test]
    fn frame_interval_values() {
        assert_eq!(frame_interval(60), Duration::from_nanos(16_666_666));
        assert_eq!(frame_interval(30), Duration::from_nanos(33_333_333));
        assert_eq!(frame_interval(0), Duration::ZERO);
    }

    #[test]
    fn default_state_has_unknown_mouse() {
        let s = FrameState::default();
        assert_eq!((s.mouse_x, s.mouse_y), (-1.0, -1.0));
        assert_eq!(s.time, 0.0);
    }

    #[test]
    fn runtime_limits_fps_and_accumulates_time() {
        let mut rt = TestRuntime {
            last: None,
            time: 0.0,
            fps: 10,
        };
        let t0 = Instant::now();

        // Primer frame: siempre dibuja.
        assert_eq!(rt.begin_frame(t0), FrameDecision::Draw);

        // 30 ms después a 10 fps (intervalo 100 ms): skip con deadline.
        let t1 = t0 + Duration::from_millis(30);
        match rt.begin_frame(t1) {
            FrameDecision::Skip { deadline } => {
                assert_eq!(deadline - t0, Duration::from_millis(100));
            }
            other => panic!("expected Skip, got {other:?}"),
        }

        // 150 ms después: dibuja y acumula tiempo desde el último frame.
        let t2 = t0 + Duration::from_millis(150);
        assert_eq!(rt.begin_frame(t2), FrameDecision::Draw);
        assert!((rt.state().time - 0.15).abs() < 1e-6);
    }

    #[test]
    fn basic_runtime_pause_freezes_time() {
        let mut rt = BasicRuntime::new(30).paused();
        let t0 = Instant::now();

        // En pausa: skip con deadline lejano (~1 s) y tiempo intacto.
        match rt.begin_frame(t0) {
            FrameDecision::Skip { deadline } => {
                assert_eq!(deadline - t0, Duration::from_secs(1));
            }
            other => panic!("expected Skip, got {other:?}"),
        }

        let t1 = t0 + Duration::from_millis(2500);
        match rt.begin_frame(t1) {
            FrameDecision::Skip { .. } => {}
            other => panic!("expected Skip, got {other:?}"),
        }

        // Reanudar: el primer frame es inmediato y el tiempo parte de
        // 0 (el reloj no acumuló en pausa).
        rt.set_paused(false);
        assert_eq!(rt.begin_frame(t1), FrameDecision::Draw);
        assert_eq!(rt.state().time, 0.0);

        let t2 = t1 + Duration::from_millis(100);
        assert_eq!(rt.begin_frame(t2), FrameDecision::Draw);
        assert!((rt.state().time - 0.1).abs() < 1e-6);
    }

    #[test]
    fn basic_runtime_params_set_by_name() {
        let mut rt = BasicRuntime::new(60);
        assert!(rt.set_param("param0", 0.7));
        assert_eq!(rt.params()[0].value, 0.7);
        // Clampeado al rango declarado 0..=1.
        assert!(rt.set_param("param0", 5.0));
        assert_eq!(rt.params()[0].value, 1.0);
        // Nombre desconocido: false.
        assert!(!rt.set_param("does_not_exist", 0.5));
        // Y el constructor fluent también lo aplica.
        assert_eq!(
            BasicRuntime::new(60).with_param("param0", 0.25).params()[0].value,
            0.25
        );
    }

    #[test]
    fn basic_runtime_resolution_and_mouse() {
        let mut rt = BasicRuntime::new(60);
        rt.set_resolution(1920, 1200);
        rt.set_mouse(10.0, 20.0);
        let s = rt.state();
        assert_eq!((s.width, s.height), (1920, 1200));
        assert_eq!((s.mouse_x, s.mouse_y), (10.0, 20.0));
    }
}
