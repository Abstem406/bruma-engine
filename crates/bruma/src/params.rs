//! `bruma params`: live parameter tuning (Phase 6 — parameter UI).
//!
//! The manifest gives parameters identity (`name/label/default`);
//! `config.json` gives them persistence (per-output `params` maps) and
//! the running daemon hot-reloads it on SIGHUP. This command is the
//! missing surface: it lists the effective parameters of an installed
//! wallpaper, sets overrides (validated against the manifest, 0..1) and
//! resets them, then wakes the daemon so the change is visible
//! immediately.
//!
//! No TUI on purpose (D8 — zero new dependencies): a plain CLI over the
//! config file is scriptable, composable and enough for tuning while
//! watching the wallpaper. The manifest is loaded with the real parser
//! before any write, so a typo in a parameter name can never be
//! persisted.

use std::path::PathBuf;

use bruma_package::{Param, Store};

/// Where the daemon's config lives (same path `bruma run` reads).
fn config_path() -> PathBuf {
    crate::config::Config::default_path().unwrap_or_else(|| {
        eprintln!("error: cannot determine the config path (no HOME?)");
        std::process::exit(1);
    })
}

/// The latest installed version's manifest for `name`, with its dir.
fn manifest_for(store: &Store, name: &str) -> (bruma_package::Manifest, PathBuf) {
    let mut best: Option<(String, PathBuf)> = None;
    for (pkg, ver, dir) in store.installed() {
        if pkg == name
            && best
                .as_ref()
                .is_none_or(|(bv, _)| ver.as_str() > bv.as_str())
        {
            best = Some((ver, dir));
        }
    }
    let Some((_, dir)) = best else {
        eprintln!("error: package '{name}' is not installed (see: bruma list)");
        std::process::exit(1);
    };
    // The installed layout keeps the manifest as a loose file (the ZIP
    // form only exists during install/pack), so parse it directly with
    // the real parser (same schema, same errors).
    let json = std::fs::read_to_string(dir.join("wallpaper.json")).unwrap_or_else(|_| {
        eprintln!("error: installed package '{name}' has no wallpaper.json");
        std::process::exit(1);
    });
    match bruma_package::Manifest::parse(&json) {
        Ok(m) => (m, dir),
        Err(e) => {
            eprintln!("error: manifest of '{name}' does not parse: {e}");
            std::process::exit(1);
        }
    }
}

/// SIGHUPs every running bruma daemon (best-effort; `bruma params` also
/// works offline — the next start picks the config up).
pub fn wake_daemons() {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    for e in entries.flatten() {
        let Ok(cmd) = std::fs::read_to_string(e.path().join("cmdline")) else {
            continue;
        };
        let mut parts = cmd.split('\0').filter(|s| !s.is_empty());
        let Some(exe) = parts.next() else { continue };
        if !exe.ends_with("bruma") || !parts.any(|a| a == "run") {
            continue;
        }
        let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        unsafe { libc::kill(pid, libc::SIGHUP) };
        println!("  ↻ SIGHUP → pid {pid}");
    }
}

/// One parameter row for `list`.
struct Row {
    name: String,
    label: Option<String>,
    default: f32,
    effective: f32,
    overridden: bool,
}

/// Collects the effective parameter values of `manifest` under the
/// current config's overrides for `output` (None = default section).
fn effective_rows(
    m: &bruma_package::Manifest,
    cfg: &crate::config::Config,
    output: Option<&str>,
) -> Vec<Row> {
    let section = match output {
        Some(o) => cfg.outputs.get(o).or(cfg.default.as_ref()),
        None => cfg.default.as_ref(),
    };
    let overrides = section.map(|s| &s.params);
    m.params
        .iter()
        .map(|p| {
            let ov = overrides.and_then(|map| map.get(&p.name)).and_then(|v| {
                crate::config::params_to_pairs(&{
                    let mut one = serde_json::Map::new();
                    one.insert(p.name.clone(), v.clone());
                    one
                })
                .ok()
                .and_then(|pairs| pairs.into_iter().next().map(|(_, f)| f))
            });
            match ov {
                Some(v) => Row {
                    name: p.name.clone(),
                    label: p.label.clone(),
                    default: p.default,
                    effective: v,
                    overridden: true,
                },
                None => Row {
                    name: p.name.clone(),
                    label: p.label.clone(),
                    default: p.default,
                    effective: p.default,
                    overridden: false,
                },
            }
        })
        .collect()
}

fn print_rows(rows: &[Row]) {
    for r in rows {
        let label = r.label.as_deref().unwrap_or(&r.name);
        let mark = if r.overridden { " *" } else { "  " };
        println!(
            "{mark} {name:<14} {label:<24} effective {eff:.3}   (default {def:.3})",
            name = r.name,
            eff = r.effective,
            def = r.default,
        );
    }
    println!("\n(* = overridden in config.json; 0..1)");
}

/// Validates `value` (0..1, two decimals) against the manifest and
/// returns the JSON number for the config.
fn checked_value(p: &Param, raw: &str) -> f32 {
    let v: f32 = match raw.parse() {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: '{raw}' is not a number (parameter values are 0..1)");
            std::process::exit(1);
        }
    };
    if !(0.0..=1.0).contains(&v) {
        eprintln!("error: {v} out of range 0..1 (parameter '{}')", p.name);
        std::process::exit(1);
    }
    v
}

/// Entry point from the CLI: `bruma params <wallpaper> [subcommand..]`.
///
/// ```text
/// bruma params demo-ripple                       list effective params
/// bruma params demo-ripple --output eDP-1 ...    target one output
/// bruma params demo-ripple set intensity 0.8     persist + SIGHUP
/// bruma params demo-ripple reset                 drop all overrides
/// bruma params demo-ripple reset intensity       drop one override
/// ```
pub fn params_command(args: &[String]) {
    let mut wallpaper: Option<String> = None;
    let mut output: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--output" | "-o" => {
                let v = it.next().unwrap_or_else(|| {
                    eprintln!("error: --output needs a value");
                    std::process::exit(2);
                });
                output = Some(v.clone());
            }
            _ if wallpaper.is_none() => wallpaper = Some(a.clone()),
            _ => rest.push(a.clone()),
        }
    }
    let Some(name) = wallpaper else {
        eprintln!(
            "usage: bruma params <wallpaper> [--output NAME] [set NAME VALUE [--adopt] | reset [NAME]]"
        );
        std::process::exit(2);
    };

    let store = super::default_store();
    let (m, _) = manifest_for(&store, &name);
    let cfg = match crate::config::Config::load() {
        Ok(c) => c.unwrap_or_default(),
        Err(e) => {
            eprintln!("error: current config is invalid, fix it first: {e}");
            std::process::exit(1);
        }
    };

    match rest.first().map(|s| s.as_str()) {
        None | Some("list") => {
            println!("{} v{} — parameters:", m.title, m.version);
            let rows = effective_rows(&m, &cfg, output.as_deref());
            if rows.is_empty() {
                println!("  (the manifest declares no parameters)");
            }
            print_rows(&rows);
        }
        Some("set") => {
            let Some(param_name) = rest.get(1) else {
                eprintln!("error: set needs a parameter name (see: bruma params {name})");
                std::process::exit(2);
            };
            let Some(raw) = rest.get(2) else {
                eprintln!("error: set needs a value 0..1");
                std::process::exit(2);
            };
            let Some(p) = m.params.iter().find(|p| &p.name == param_name) else {
                eprintln!(
                    "error: '{}' is not a parameter of '{name}' (valid: {})",
                    param_name,
                    m.params
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                std::process::exit(1);
            };
            let v = checked_value(p, raw);
            let adopt = rest.iter().any(|a| a == "--adopt");

            let path = config_path();
            let mut cfg = cfg;
            let section = match &output {
                Some(o) => cfg.outputs.entry(o.clone()).or_default(),
                None => match cfg.default {
                    Some(ref mut s) => s,
                    None => {
                        cfg.default = Some(Default::default());
                        cfg.default.as_mut().expect("just set")
                    }
                },
            };
            if section
                .package
                .as_deref()
                .is_none_or(|p| p.split(':').next() != Some(name.as_str()))
            {
                if !adopt {
                    eprintln!(
                        "error: config does not run '{name}' on {} — pass --adopt to point that section at '{name}' (a daemon started with --package switches over on the next SIGHUP)",
                        output.as_deref().unwrap_or("default")
                    );
                    std::process::exit(1);
                }
                // Adopting: the section now runs THIS wallpaper, so the
                // previous package's param overrides are meaningless —
                // fresh params map with just the tuned value.
                section.package = Some(name.clone());
                section.params.clear();
                section.shader = None;
                section.image = None;
            }
            section.params.insert(
                p.name.clone(),
                serde_json::json!((v * 100.0).round() / 100.0),
            );
            let json = serde_json::to_string_pretty(&cfg).expect("serialize config");
            if let Err(e) = std::fs::write(&path, json + "\n") {
                eprintln!("error: cannot write {}: {e}", path.display());
                std::process::exit(1);
            }
            println!("✔ {name}: {} = {v:.3} → {}", p.name, path.display());
            wake_daemons();
        }
        Some("reset") => {
            let target = rest.get(1);
            let path = config_path();
            let mut cfg = cfg;
            let sections: Vec<&mut crate::config::OutputConfig> = match &output {
                Some(o) => match cfg.outputs.get_mut(o) {
                    Some(s) => vec![s],
                    None => {
                        eprintln!("error: no config entry for output '{o}'");
                        std::process::exit(1);
                    }
                },
                None => match cfg.default {
                    Some(ref mut s) => vec![s],
                    None => {
                        eprintln!("error: no default section in the config");
                        std::process::exit(1);
                    }
                },
            };
            let mut dropped = 0;
            for s in sections {
                let before = s.params.len();
                match target {
                    Some(n) => dropped += s.params.remove(n).map(|_| 1).unwrap_or(0),
                    None => {
                        s.params.clear();
                        dropped = before;
                    }
                }
            }
            if dropped == 0 {
                println!("(nothing to reset)");
                return;
            }
            let json = serde_json::to_string_pretty(&cfg).expect("serialize config");
            if let Err(e) = std::fs::write(&path, json + "\n") {
                eprintln!("error: cannot write {}: {e}", path.display());
                std::process::exit(1);
            }
            println!("✔ reset {dropped} override(s) → {}", path.display());
            wake_daemons();
        }
        Some(other) => {
            eprintln!("error: unknown params subcommand '{other}' (list | set | reset)");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_with(params: &[(&str, f32)]) -> bruma_package::Manifest {
        let params: Vec<String> = params
            .iter()
            .map(|(n, d)| format!("{{ \"name\": \"{n}\", \"default\": {d} }}"))
            .collect();
        let json = format!(
            "{{ \"format\": 1, \"type\": \"shader\", \"title\": \"t\", \
             \"entry\": \"main.wgsl\", \"preview\": \"preview.png\", \
             \"version\": \"0.1.0\", \
             \"permissions\": [], \"params\": [{}] }}",
            params.join(",")
        );
        bruma_package::Manifest::parse(&json).expect("manifest")
    }

    #[test]
    fn effective_rows_applies_overrides() {
        let m = manifest_with(&[("intensity", 0.5), ("damping", 0.2)]);
        let mut cfg = crate::config::Config::default();
        cfg.default
            .get_or_insert_with(Default::default)
            .params
            .insert("intensity".into(), serde_json::json!(0.9));

        let rows = effective_rows(&m, &cfg, None);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].overridden && (rows[0].effective - 0.9).abs() < 1e-6);
        assert!(!rows[1].overridden && (rows[1].effective - 0.2).abs() < 1e-6);
    }

    #[test]
    fn effective_rows_per_output_wins_over_default() {
        let m = manifest_with(&[("intensity", 0.5)]);
        let mut cfg = crate::config::Config::default();
        cfg.default
            .get_or_insert_with(Default::default)
            .params
            .insert("intensity".into(), serde_json::json!(0.1));
        let e = cfg.outputs.entry("eDP-1".into()).or_default();
        e.params.insert("intensity".into(), serde_json::json!(0.7));

        let rows = effective_rows(&m, &cfg, Some("eDP-1"));
        assert!((rows[0].effective - 0.7).abs() < 1e-6);
    }

    #[test]
    fn effective_rows_rejects_bad_override_values() {
        let m = manifest_with(&[("intensity", 0.5)]);
        let mut cfg = crate::config::Config::default();
        cfg.default
            .get_or_insert_with(Default::default)
            .params
            .insert("intensity".into(), serde_json::json!(9.0));
        let rows = effective_rows(&m, &cfg, None);
        // The engine's own resolver rejects this at run time; the list
        // shows the DEFAULT for that parameter instead of a bogus one.
        assert!(!rows[0].overridden);
    }
}
