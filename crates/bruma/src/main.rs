//! CLI del motor bruma.
//!
//! Estado: **Fase 0**. Este binario es solo un marcador que confirma que
//! el workspace compila y enlaza el núcleo. Los subcomandos llegarán con
//! sus fases: `run` (F2), `validate`/`install` (F4), `new`/`pack` (F6).

fn main() {
    println!(
        "bruma v{} — motor libre de wallpapers animados para Wayland\n\
         Fase 0: estructura creada. Aún no hay subcomandos.",
        bruma_core::ENGINE_VERSION
    );
}
