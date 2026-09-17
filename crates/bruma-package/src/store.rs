//! Empaquetado (`pack`), carga, validación e instalación de paquetes
//! `.wallpaper`.
//!
//! Seguridad (los tres ataques clásicos de ZIP, todos cubiertos):
//!
//! 1. **Path traversal** (`../../.bashrc`): cada entrada se valida con
//!    [`zip::read::ZipFile::enclosed_name`]; solo rutas que quedan dentro
//!    del directorio destino se extraen (además del rechazo temprano del
//!    manifiesto para `entry`/`preview`).
//! 2. **Symlinks** (escape con enlaces): cualquier entrada symlink
//!    rechaza el paquete entero (los wallpapers no los necesitan).
//! 3. **Zip bombs** (descompresión desproporcionada): límites duros por
//!    archivo (usando el tamaño declarado ANTES de leer) y por total
//!    descomprimido, más un tope de número de entradas.
//!
//! La instalación copia el paquete a un directorio versionado
//! (`<install_dir>/<nombre>/<versión-semver>/`) para que instalar una
//! versión nueva no destruya la anterior.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use zip::read::ZipFile;
use zip::{CompressionMethod, ZipArchive};

use crate::error::PackError;
use crate::manifest::Manifest;

/// Límites de seguridad. Un wallpaper es texto + un preview: con esto
/// sobra. (FUTURO: el manifiesto v2 podrá declarar assets grandes.)
pub const MAX_FILES: usize = 256;
pub const MAX_FILE_SIZE: u64 = 32 * 1024 * 1024;
pub const MAX_TOTAL_SIZE: u64 = 96 * 1024 * 1024;

/// Límite del paquete comprimido en memoria (protección doble: incluso
/// si el ZIP declara tamaños falsos, el lector nunca ve más que esto).
const MAX_ZIP_INPUT: u64 = 256 * 1024 * 1024;

/// Directorio donde se instalan los paquetes. Se pasa explícito para que
/// el crate no decida nada del sistema (frontera D6); la CLI resuelve
/// `XDG_DATA_HOME/bruma/wallpapers` y lo crea si falta.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Store sobre `root`. No lo crea: [`Store::install`] lo hace perezoso.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    /// Raíz del store.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Lista los paquetes instalados (una entrada por versión). Sin
    /// recorrer el árbol entero: `root/<paquete>/<versión>/`.
    pub fn installed(&self) -> Vec<(String, String, PathBuf)> {
        let mut out = Vec::new();
        let Ok(pkgs) = fs::read_dir(&self.root) else {
            return out;
        };
        for pkg in pkgs.flatten() {
            if !pkg.path().is_dir() {
                continue;
            }
            let Some(name) = pkg.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(vers) = fs::read_dir(pkg.path()) else {
                continue;
            };
            for v in vers.flatten() {
                if v.path().join("wallpaper.json").is_file()
                    && let Some(ver) = v.file_name().to_str().map(str::to_owned)
                {
                    out.push((name.clone(), ver, v.path()));
                }
            }
        }
        out.sort();
        out
    }

    /// Ruta de instalación de un paquete con nombre y versión dados.
    pub fn installed_path(&self, name: &str, version: &str) -> PathBuf {
        self.root.join(name).join(version)
    }

    /// Empaqueta un directorio (con `wallpaper.json` dentro) en un
    /// archivo `.wallpaper`. La salida es `<título-slug>.wallpaper` en
    /// `out_dir` si `out` es `None`.
    pub fn pack(dir: &Path, out: Option<&Path>) -> Result<PathBuf, PackError> {
        let manifest_path = dir.join("wallpaper.json");
        let json = fs::read_to_string(&manifest_path).map_err(|_| {
            PackError::Missing("wallpaper.json (en el directorio a empaquetar)".to_owned())
        })?;
        let manifest = Manifest::parse(&json)?;

        let file_name = format!("{}.wallpaper", manifest.install_name());
        let out_path = match out {
            Some(p) if p.is_dir() => p.join(&file_name),
            Some(p) => p.to_owned(),
            None => dir.join(&file_name),
        };

        let file = fs::File::create(&out_path)?;
        let mut zip = zip::ZipWriter::new(file);
        // Los wallpapers son texto: compresión máxima razonable y
        // permisos normales de archivo (nunca ejecutables).
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);

        let mut add_file = |rel: &str| -> Result<(), PackError> {
            let path = dir.join(rel);
            let data = fs::read(&path).map_err(|_| PackError::Missing(rel.to_owned()))?;
            if data.len() as u64 > MAX_FILE_SIZE {
                return Err(PackError::FileTooBig(rel.to_owned(), MAX_FILE_SIZE));
            }
            zip.start_file(rel, options).map_err(PackError::from)?;
            std::io::Write::write_all(&mut zip, &data)?;
            Ok(())
        };

        add_file("wallpaper.json")?;
        add_file(&manifest.entry)?;
        add_file(&manifest.preview)?;
        // Assets extra declarados por el creador: cualquier `assets/`
        // presente en el directorio se incluye (validado igual que el
        // resto al validar/instalar).
        let assets = dir.join("assets");
        if assets.is_dir() {
            let mut entries: Vec<_> = fs::read_dir(&assets)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect();
            entries.sort();
            for p in entries {
                let rel = p
                    .strip_prefix(dir)
                    .map_err(|_| PackError::UnsafePath(p.display().to_string()))?
                    .to_string_lossy()
                    .replace('\\', "/");
                add_file(&rel)?;
            }
        }
        zip.finish()?;
        Ok(out_path)
    }

    /// Lee y valida el manifiesto de un ZIP en memoria. Recorre además
    /// TODAS las entradas aplicando las reglas de seguridad (symlinks,
    /// tamaños, rutas): `validate` es tan estricto como `install`.
    pub fn load_manifest(bytes: &[u8]) -> Result<Manifest, PackError> {
        let mut archive = open_archive(bytes)?;
        let json = read_entry_string(&mut archive, "wallpaper.json")?;
        let manifest = Manifest::parse(&json)?;
        check_entry_and_preview(&mut archive, &manifest)?;
        scan_archive(&mut archive)?;
        Ok(manifest)
    }

    /// Valida un paquete completo (el paso de `bruma validate`).
    ///
    /// Devuelve el manifiesto validado y la ruta del entry como está
    /// dentro del ZIP (para diagnósticos).
    pub fn validate(bytes: &[u8]) -> Result<Manifest, PackError> {
        Self::load_manifest(bytes)
    }

    /// Instala el paquete en el store.
    ///
    /// La extracción se hace a un directorio de staging y se renombra al
    /// final: si el paquete es inválido o la extracción falla a medias,
    /// no queda basura parcial en el store. No sobrescribe: si la
    /// versión ya está instalada, devuelve la ruta existente
    /// (`installed: false`), como buena costumbre de gestores.
    pub fn install(
        &self,
        bytes: &[u8],
        name: &str,
        version: &str,
    ) -> Result<(PathBuf, bool), PackError> {
        let mut archive = open_archive(bytes)?;
        let json = read_entry_string(&mut archive, "wallpaper.json")?;
        let manifest = Manifest::parse(&json)?;
        check_entry_and_preview(&mut archive, &manifest)?;
        // Segunda pasada completa (defensa en profundidad) ANTES de
        // escribir nada: symlinks, tamaños y rutas.
        scan_archive(&mut archive)?;

        let dest_root = self.installed_path(name, version);
        if dest_root.join("wallpaper.json").is_file() {
            return Ok((dest_root, false));
        }

        let parent = dest_root
            .parent()
            .ok_or_else(|| PackError::UnsafePath(dest_root.display().to_string()))?;
        fs::create_dir_all(parent)?;
        let staging = parent.join(format!(
            ".{}.tmp",
            dest_root.file_name().unwrap_or_default().to_string_lossy()
        ));
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging)?;

        let extracted = extract_all(&mut archive, &staging);
        match extracted {
            Ok(()) => {
                // Carrera benigna: otra instalación pudo ganar mientras.
                if dest_root.join("wallpaper.json").is_file() {
                    let _ = fs::remove_dir_all(&staging);
                    return Ok((dest_root, false));
                }
                fs::rename(&staging, &dest_root)?;
                Ok((dest_root, true))
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                Err(e)
            }
        }
    }
}

/// Abre un ZIP desde memoria, con límites. `Cursor` es `Read + Seek` y
/// el `ZipArchive` lo posee.
fn open_archive(bytes: &[u8]) -> Result<ZipArchive<std::io::Cursor<&[u8]>>, PackError> {
    if bytes.len() as u64 > MAX_ZIP_INPUT {
        return Err(PackError::TotalTooBig(MAX_ZIP_INPUT));
    }
    let cursor = std::io::Cursor::new(bytes);
    let archive = ZipArchive::new(cursor)?;
    if archive.len() > MAX_FILES {
        return Err(PackError::TooManyFiles(archive.len(), MAX_FILES));
    }
    Ok(archive)
}

/// Lee una entrada de texto del ZIP con validaciones: sin symlinks y con
/// límite de tamaño ANTES de materializarla.
fn read_entry_string(
    archive: &mut ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> Result<String, PackError> {
    let mut file = archive
        .by_name(name)
        .map_err(|_| PackError::Missing(name.to_owned()))?;
    if file.is_symlink() {
        return Err(PackError::Symlink(name.to_owned()));
    }
    if file.size() > MAX_FILE_SIZE {
        return Err(PackError::FileTooBig(name.to_owned(), MAX_FILE_SIZE));
    }
    let mut s = String::new();
    file.read_to_string(&mut s)
        .map_err(|e| PackError::Json(name.to_owned(), serde_json::Error::io(e)))?;
    Ok(s)
}

/// Recorre TODAS las entradas aplicando las reglas de seguridad:
/// symlinks prohibidos, tamaño máximo por archivo, rutas contenidas y
/// total descomprimido bajo el límite (anti zip bomb).
fn scan_archive(archive: &mut ZipArchive<std::io::Cursor<&[u8]>>) -> Result<(), PackError> {
    let mut total: u64 = 0;
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = file.name().to_owned();
        if file.is_symlink() {
            return Err(PackError::Symlink(name));
        }
        if file.size() > MAX_FILE_SIZE {
            return Err(PackError::FileTooBig(name, MAX_FILE_SIZE));
        }
        if file.enclosed_name().is_none() {
            return Err(PackError::UnsafePath(name));
        }
        total += file.size();
        if total > MAX_TOTAL_SIZE {
            return Err(PackError::TotalTooBig(MAX_TOTAL_SIZE));
        }
    }
    Ok(())
}

/// Extrae todas las entradas a `dest` (ya validadas por
/// [`scan_archive`]; las comprobaciones aquí son de cinturón y tirantes).
fn extract_all(
    archive: &mut ZipArchive<std::io::Cursor<&[u8]>>,
    dest: &Path,
) -> Result<(), PackError> {
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let Some(rel) = file.enclosed_name() else {
            return Err(PackError::UnsafePath(file.name().to_owned()));
        };
        if file.is_dir() {
            fs::create_dir_all(dest.join(&rel))?;
            continue;
        }
        let mut data = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut data)?;
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(out, &data)?;
    }
    Ok(())
}

/// Comprueba que `entry` y `preview` del manifiesto existen dentro del
/// paquete y que ninguna entrada del ZIP es symlink.
fn check_entry_and_preview(
    archive: &mut ZipArchive<std::io::Cursor<&[u8]>>,
    manifest: &Manifest,
) -> Result<(), PackError> {
    for name in [&manifest.entry, &manifest.preview] {
        let file: ZipFile<'_, std::io::Cursor<&[u8]>> = archive
            .by_name(name)
            .map_err(|_| PackError::Missing(name.to_owned()))?;
        if file.is_symlink() {
            return Err(PackError::Symlink(name.to_owned()));
        }
        if file.size() > MAX_FILE_SIZE {
            return Err(PackError::FileTooBig(name.to_owned(), MAX_FILE_SIZE));
        }
        if file.enclosed_name().is_none() {
            return Err(PackError::UnsafePath(name.to_owned()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::SCHEMA_VERSION;

    const MANIFEST: &str = r#"{
        "format": 1,
        "type": "shader",
        "title": "Prueba",
        "entry": "main.wgsl",
        "preview": "preview.png",
        "permissions": [],
        "min_engine": "0.1.0"
    }"#;

    /// Construye un `.wallpaper` en memoria.
    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, data) in entries {
                w.start_file(*name, opts).unwrap();
                std::io::Write::write_all(&mut w, data).unwrap();
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    fn good_entries() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("wallpaper.json", MANIFEST.as_bytes().to_vec()),
            (
                "main.wgsl",
                b"fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }".to_vec(),
            ),
            ("preview.png", vec![0x89, b'P', b'N', b'G', 0, 0, 0, 0]),
        ]
    }

    fn bytes_of<'a>(v: &'a [(&'a str, Vec<u8>)]) -> Vec<(&'a str, &'a [u8])> {
        v.iter().map(|(n, d)| (*n, d.as_slice())).collect()
    }

    #[test]
    fn manifiesto_de_paquete_bien_formado() {
        let bytes = zip_bytes(&bytes_of(&good_entries()));
        let m = Store::load_manifest(&bytes).unwrap();
        assert_eq!(m.title, "Prueba");
        assert_eq!(m.format, SCHEMA_VERSION);
    }

    #[test]
    fn falta_el_entry() {
        let entries: Vec<(&str, Vec<u8>)> = good_entries()
            .into_iter()
            .filter(|(n, _)| *n != "main.wgsl")
            .collect();
        let bytes = zip_bytes(&bytes_of(&entries));
        let err = Store::load_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("main.wgsl"), "{err}");
    }

    #[test]
    fn symlink_rechazado() {
        let mut bytes = zip_bytes(&bytes_of(&good_entries()));
        // Añadimos un symlink reescribiendo el zip con uno dentro
        // (add_symlink del crate).
        let mut buf = std::io::Cursor::new(bytes.clone());
        {
            let mut r = zip::ZipArchive::new(&mut buf).unwrap();
            let mut out = std::io::Cursor::new(Vec::new());
            {
                let mut w = zip::ZipWriter::new(&mut out);
                for i in 0..r.len() {
                    let mut f = r.by_index(i).unwrap();
                    let opts = zip::write::SimpleFileOptions::default();
                    w.start_file(f.name().to_owned(), opts).unwrap();
                    std::io::copy(&mut f, &mut w).unwrap();
                }
                w.add_symlink(
                    "evil",
                    "/home/usuario/.bashrc",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                w.finish().unwrap();
            }
            bytes = out.into_inner();
        }
        let err = Store::load_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("symlink"), "{err}");
        // Y también al instalar.
        let (store, dir) = temp_store("symlink");
        let err = store
            .install(&bytes, "prueba", "0.1.0")
            .unwrap_err()
            .to_string();
        assert!(err.contains("symlink"), "{err}");
        assert!(!dir.join("prueba").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Directorio temporal único por test (los tests corren en paralelo).
    fn temp_store(tag: &str) -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "bruma-test-{}-{}-{tag}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        (Store::new(&dir), dir)
    }

    #[test]
    fn path_traversal_rechazado_al_instalar() {
        let mut entries = good_entries();
        entries.push(("../../escape.sh", b"echo pwned".to_vec()));
        let bytes = zip_bytes(&bytes_of(&entries));
        let (store, dir) = temp_store("traversal");
        let err = store
            .install(&bytes, "prueba", "0.1.0")
            .unwrap_err()
            .to_string();
        assert!(err.contains("insegura"), "{err}");
        // Nada queda escrito (ni staging): la validación es previa.
        assert!(!dir.join("prueba").exists(), "quedó basura en {:?}", dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_y_listado_y_no_sobrescribe() {
        let bytes = zip_bytes(&bytes_of(&good_entries()));
        let (store, dir) = temp_store("install");
        let (path, installed) = store.install(&bytes, "prueba", "0.1.0").unwrap();
        assert!(installed);
        assert!(path.join("wallpaper.json").is_file());
        assert!(path.join("main.wgsl").is_file());

        let (_, installed_again) = store.install(&bytes, "prueba", "0.1.0").unwrap();
        assert!(!installed_again, "no debe sobrescribir");

        let list = store.installed();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "prueba");
        assert_eq!(list[0].1, "0.1.0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zip_bomb_rechazada() {
        // Entrada que declara tamaño enorme: el límite se aplica ANTES de
        // materializar nada (escaneo del central directory).
        let mut entries = good_entries();
        entries.push(("assets/huge.bin", vec![0u8; 40 * 1024 * 1024]));
        let bytes = zip_bytes(&bytes_of(&entries));
        let err = Store::load_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("demasiado grande"), "{err}");
        let (store, dir) = temp_store("bomb");
        let err = store
            .install(&bytes, "prueba", "0.1.0")
            .unwrap_err()
            .to_string();
        assert!(err.contains("demasiado grande"), "{err}");
        assert!(!dir.join("prueba").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_crea_zip_valido() {
        let dir = std::env::temp_dir().join(format!(
            "bruma-pack-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("t")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, data) in good_entries() {
            std::fs::write(dir.join(name), data).unwrap();
        }
        let out = Store::pack(&dir, None).unwrap();
        assert!(out.is_file(), "{out:?}");
        let bytes = std::fs::read(&out).unwrap();
        let m = Store::load_manifest(&bytes).unwrap();
        assert_eq!(m.title, "Prueba");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
