//! Desktop integration (Phase 6, the last milestone piece).
//!
//! `bruma mime install` registers the `.wallpaper` MIME type and a
//! desktop entry, so a package downloaded from anywhere opens with a
//! double click / drag-and-drop. The handler is `bruma open`, which
//! installs the package AND applies it (default config section + SIGHUP
//! to running daemons): download → double click → wallpaper running.
//! `bruma open FILE` alone does the same from the terminal.
//!
//! Scope: XDG desktops (freedesktop.org shared-mime-info + desktop
//! entries). No new dependencies (D8): the XML/ini content is written
//! by hand and the database updates are best-effort external commands.

use crate::config;

/// The MIME type bruma owns.
pub const MIME_TYPE: &str = "application/x-bruma-wallpaper";

/// The shared-mime-info XML for `application/x-bruma-wallpaper`.
fn mime_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="{MIME_TYPE}">
    <comment>bruma animated wallpaper</comment>
    <sub-class-of type="application/zip"/>
    <glob pattern="*.wallpaper"/>
  </mime-type>
</mime-info>
"#
    )
}

/// The desktop entry that opens `.wallpaper` files with bruma.
fn desktop_entry() -> String {
    format!(
        r#"[Desktop Entry]
Type=Application
Name=bruma
GenericName=Animated wallpaper engine
Comment=Install and run a bruma wallpaper package
Exec=bruma open %f
Icon=bruma
Terminal=false
NoDisplay=false
Categories=Utility;
MimeType={MIME_TYPE};
"#
    )
}

/// XDG data home (matches the store's layout: `$XDG_DATA_HOME` or
/// `~/.local/share`).
fn data_home() -> Option<std::path::PathBuf> {
    std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|h| !h.is_empty())
                .map(|h| std::path::PathBuf::from(h).join(".local/share"))
        })
}

/// Runs a best-effort database update; absence of the tool is normal
/// (not every desktop ships them) and only earns a note.
fn update_database(cmd: &str, args: &[&str]) {
    match std::process::Command::new(cmd).args(args).status() {
        Ok(s) if s.success() => println!("  ✔ {cmd}"),
        Ok(_) => println!("  ⚠ {cmd} failed (the association may need a re-login)"),
        Err(_) => println!("  • {cmd} not found (skipped; usually harmless)"),
    }
}

/// `bruma mime install|remove` — registers (or unregisters) the
/// `.wallpaper` MIME type and its handler on this user session.
pub fn mime_command(args: &[String]) {
    let Some(home) = data_home() else {
        eprintln!("error: without XDG_DATA_HOME or HOME there is no desktop integration");
        std::process::exit(1);
    };
    let mime_xml_dir = home.join("mime/packages");
    let apps_dir = home.join("applications");
    let xml_path = mime_xml_dir.join("x-bruma-wallpaper.xml");
    let desktop_path = apps_dir.join("bruma-open.desktop");

    match args.first().map(|s| s.as_str()) {
        Some("install") => {
            for dir in [&mime_xml_dir, &apps_dir] {
                std::fs::create_dir_all(dir).unwrap_or_else(|e| {
                    eprintln!("error: {}: {e}", dir.display());
                    std::process::exit(1);
                });
            }
            std::fs::write(&xml_path, mime_xml()).unwrap_or_else(|e| {
                eprintln!("error: {}: {e}", xml_path.display());
                std::process::exit(1);
            });
            std::fs::write(&desktop_path, desktop_entry()).unwrap_or_else(|e| {
                eprintln!("error: {}: {e}", desktop_path.display());
                std::process::exit(1);
            });
            println!("✔ {MIME_TYPE} registered for *.wallpaper");
            println!(
                "  files: {} + {}",
                xml_path.display(),
                desktop_path.display()
            );
            // The `Exec` line must resolve: without a bruma binary on
            // PATH the double click would do nothing.
            if which_bruma().is_none() {
                eprintln!("⚠ 'bruma' is not on PATH: add it or fix the desktop entry");
            }
            update_database(
                "update-mime-database",
                &[home.join("mime").to_str().unwrap_or_default()],
            );
            update_database(
                "update-desktop-database",
                &[apps_dir.to_str().unwrap_or_default()],
            );
            println!("  try: double click any .wallpaper file (or drag it to the desktop)");
        }
        Some("remove") => {
            let mut removed = false;
            for path in [&xml_path, &desktop_path] {
                match std::fs::remove_file(path) {
                    Ok(()) => {
                        removed = true;
                        println!("  ✘ {}", path.display());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => {
                        eprintln!("error: {}: {e}", path.display());
                        std::process::exit(1);
                    }
                }
            }
            if removed {
                update_database(
                    "update-mime-database",
                    &[home.join("mime").to_str().unwrap_or_default()],
                );
                println!("✔ MIME association removed");
            } else {
                println!("• no MIME association was installed");
            }
        }
        _ => {
            eprintln!("usage: bruma mime install|remove");
            std::process::exit(2);
        }
    }
}

/// Locates `bruma` on PATH (to warn when the desktop entry would
/// resolve to nothing).
fn which_bruma() -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("bruma"))
            .find(|p| p.is_file())
    })
}

/// Installs a `.wallpaper` file with the same validations as `bruma
/// install` and returns its manifest.
fn install_package_file(path: &str) -> bruma_package::Manifest {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let manifest = bruma_package::Store::validate(&bytes).unwrap_or_else(|e| {
        eprintln!("error: invalid package: {e}");
        std::process::exit(1);
    });
    let store = crate::default_store();
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
    } else {
        println!(
            "• '{}' {} was already installed at {}",
            manifest.title,
            manifest.version,
            dest.display()
        );
    }
    manifest
}

/// Applies the just-installed wallpaper: the default config section
/// points at the package (per-output sections keep theirs — pin those
/// with `bruma params --adopt`), then running daemons are SIGHUPed.
pub fn apply_as_default(name: &str) {
    let mut cfg = match config::Config::load() {
        Ok(Some(c)) => c,
        Ok(None) => config::Config::default(),
        Err(e) => {
            eprintln!("error: config.json does not parse ({e}); not touching it");
            eprintln!("  fix it and then: bruma params {name} set intensity 1 --adopt");
            return;
        }
    };
    // Activation is a whole-desktop statement: per-output pins from the
    // previous setup would silently keep outputs on stale content (the
    // user sees "activate did nothing"), so release them all. Empty
    // sections are removed too: a present-but-empty section would still
    // shadow the default at output resolution.
    let released: Vec<String> = cfg.outputs.keys().cloned().collect();
    for name in &released {
        cfg.outputs.remove(name);
    }
    match &mut cfg.default {
        Some(d) => {
            d.package = Some(name.to_owned());
            // The previous package's params are meaningless for this one
            // (worse: unknown names make the strict config REJECT the
            // whole file at startup). The new wallpaper starts from its
            // manifest defaults; tune with `bruma params`.
            d.params.clear();
        }
        None => {
            cfg.default = Some(config::OutputConfig {
                package: Some(name.to_owned()),
                ..Default::default()
            })
        }
    }
    let path = config::Config::default_path().unwrap_or_else(|| {
        eprintln!("error: neither HOME nor XDG_CONFIG_HOME set");
        std::process::exit(1);
    });
    let json = match serde_json::to_string_pretty(&cfg) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: cannot serialize config: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(&path, json + "\n") {
        eprintln!("error: cannot write {}: {e}", path.display());
        return;
    }
    println!("✔ '{name}' is now the default wallpaper");
    if !released.is_empty() {
        println!("  released stale pins on: {}", released.join(", "));
    }
    crate::params::wake_daemons();
}

/// `bruma open FILE.wallpaper` — the desktop entry's target and the
/// terminal shortcut: install + apply in one step.
pub fn open_command(args: &[String]) {
    let Some(path) = args.first() else {
        eprintln!("usage: bruma open PACKAGE.wallpaper");
        std::process::exit(2);
    };
    let manifest = install_package_file(path);
    apply_as_default(&manifest.install_name());
    println!("  tune it with: bruma params {}", manifest.install_name());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_xml_declares_type_and_glob() {
        let xml = mime_xml();
        assert!(xml.contains(r#"type="application/x-bruma-wallpaper""#));
        assert!(xml.contains(r#"pattern="*.wallpaper""#));
        // A package IS a zip: file managers use the subclass for magic
        // detection fallback.
        assert!(xml.contains(r#"sub-class-of type="application/zip""#));
    }

    #[test]
    fn desktop_entry_targets_open() {
        let entry = desktop_entry();
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("Exec=bruma open %f"));
        assert!(entry.contains("MimeType=application/x-bruma-wallpaper;"));
        // Not hidden: it must show up in "Open With" dialogs.
        assert!(entry.contains("NoDisplay=false"));
    }
}
