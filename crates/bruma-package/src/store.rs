//! Packing (`pack`), loading, validation and installation of `.wallpaper`
//! packages.
//!
//! Security (the three classic ZIP attacks, all covered):
//!
//! 1. **Path traversal** (`../../.bashrc`): every entry is validated with
//!    [`zip::read::ZipFile::enclosed_name`]; only paths that stay inside
//!    the destination directory get extracted (plus the manifest's early
//!    rejection for `entry`/`preview`).
//! 2. **Symlinks** (escape via links): any symlink entry rejects the whole
//!    package (wallpapers don't need them).
//! 3. **Zip bombs** (disproportionate decompression): hard limits per
//!    file (using the declared size BEFORE reading) and for the total
//!    decompressed size, plus a cap on the number of entries.
//!
//! Installation copies the package into a versioned directory
//! (`<install_dir>/<name>/<semver-version>/`) so installing a new version
//! never destroys the previous one.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use zip::read::ZipFile;
use zip::{CompressionMethod, ZipArchive};

use crate::error::PackError;
use crate::manifest::Manifest;

/// Security limits. A wallpaper is text + a preview: this is plenty.
/// (FUTURE: manifest v2 may declare large assets.)
pub const MAX_FILES: usize = 256;
pub const MAX_FILE_SIZE: u64 = 32 * 1024 * 1024;
pub const MAX_TOTAL_SIZE: u64 = 96 * 1024 * 1024;

/// Cap for the compressed package held in memory (double protection: even
/// if the ZIP lies about sizes, the reader never sees more than this).
const MAX_ZIP_INPUT: u64 = 256 * 1024 * 1024;

/// Directory where packages are installed. Passed explicitly so the crate
/// decides nothing about the system (D6 boundary); the CLI resolves
/// `XDG_DATA_HOME/bruma/wallpapers` and creates it if missing.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Store over `root`. Does not create it: [`Store::install`] does so
    /// lazily.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Store { root: root.into() }
    }

    /// Store root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Lists installed packages (one entry per version). No full-tree
    /// walk: `root/<package>/<version>/`.
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

    /// Install path of a package with the given name and version.
    pub fn installed_path(&self, name: &str, version: &str) -> PathBuf {
        self.root.join(name).join(version)
    }

    /// Packs a directory (containing `wallpaper.json`) into a `.wallpaper`
    /// file. Output is `<title-slug>.wallpaper` in `out_dir` when `out`
    /// is `None`.
    pub fn pack(dir: &Path, out: Option<&Path>) -> Result<PathBuf, PackError> {
        let manifest_path = dir.join("wallpaper.json");
        let json = fs::read_to_string(&manifest_path).map_err(|_| {
            PackError::Missing("wallpaper.json (in the directory to pack)".to_owned())
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
        // Wallpapers are text: sensible maximum compression and normal
        // file permissions (never executable).
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
        // Extra assets declared by the creator: any `assets/` present in
        // the directory is included (validated like everything else on
        // validate/install).
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

    /// Reads and validates the manifest of a ZIP in memory. Also walks
    /// ALL entries applying the security rules (symlinks, sizes, paths):
    /// `validate` is as strict as `install`.
    pub fn load_manifest(bytes: &[u8]) -> Result<Manifest, PackError> {
        let mut archive = open_archive(bytes)?;
        let json = read_entry_string(&mut archive, "wallpaper.json")?;
        let manifest = Manifest::parse(&json)?;
        check_entry_and_preview(&mut archive, &manifest)?;
        scan_archive(&mut archive)?;
        Ok(manifest)
    }

    /// Validates a whole package (the `bruma validate` step).
    ///
    /// Returns the validated manifest and the entry's path as it lives
    /// inside the ZIP (for diagnostics).
    pub fn validate(bytes: &[u8]) -> Result<Manifest, PackError> {
        Self::load_manifest(bytes)
    }

    /// Installs the package into the store.
    ///
    /// Extraction goes to a staging directory renamed at the end: if the
    /// package is invalid or extraction fails halfway, no partial garbage
    /// is left in the store. Never overwrites: if the version is already
    /// installed, returns the existing path (`installed: false`), as good
    /// package managers do.
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
        // Full second pass (defense in depth) BEFORE writing anything:
        // symlinks, sizes and paths.
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
                // Benign race: another install may have won meanwhile.
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

    /// Removes an installed package (name + version). Fails only if the
    /// directory cannot be deleted; a missing package is a `None`.
    pub fn uninstall(&self, name: &str, version: &str) -> Result<bool, PackError> {
        let dir = self.installed_path(name, version);
        if !dir.join("wallpaper.json").is_file() {
            return Ok(false);
        }
        fs::remove_dir_all(&dir)?;
        Ok(true)
    }
}

/// Opens a ZIP from memory, with limits. `Cursor` is `Read + Seek` and the
/// `ZipArchive` owns it.
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

/// Reads a text entry from the ZIP with checks: no symlinks and a size
/// limit BEFORE materializing it.
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

/// Walks ALL entries applying the security rules: symlinks forbidden,
/// per-file size cap, contained paths and decompressed total under the
/// limit (anti zip bomb).
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

/// Extracts all entries to `dest` (already validated by [`scan_archive`];
/// the checks here are belt and braces).
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

/// Checks that the manifest's `entry`, `preview` and declared `textures`
/// exist inside the package and that none of those entries is a symlink.
fn check_entry_and_preview(
    archive: &mut ZipArchive<std::io::Cursor<&[u8]>>,
    manifest: &Manifest,
) -> Result<(), PackError> {
    let mut names: Vec<&str> = vec![&manifest.entry, &manifest.preview];
    names.extend(manifest.textures.iter().map(|s| s.path.as_str()));
    for name in names {
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
        "title": "Test",
        "entry": "main.wgsl",
        "preview": "preview.png",
        "permissions": [],
        "min_engine": "0.1.0"
    }"#;

    /// Builds a `.wallpaper` in memory.
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
    fn well_formed_package_manifest() {
        let bytes = zip_bytes(&bytes_of(&good_entries()));
        let m = Store::load_manifest(&bytes).unwrap();
        assert_eq!(m.title, "Test");
        assert_eq!(m.format, SCHEMA_VERSION);
    }

    #[test]
    fn missing_entry() {
        let entries: Vec<(&str, Vec<u8>)> = good_entries()
            .into_iter()
            .filter(|(n, _)| *n != "main.wgsl")
            .collect();
        let bytes = zip_bytes(&bytes_of(&entries));
        let err = Store::load_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("main.wgsl"), "{err}");
    }

    #[test]
    fn symlink_rejected() {
        let mut bytes = zip_bytes(&bytes_of(&good_entries()));
        // Add a symlink by rewriting the zip with one inside (crate's
        // add_symlink).
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
                    "/home/user/.bashrc",
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                w.finish().unwrap();
            }
            bytes = out.into_inner();
        }
        let err = Store::load_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("symlink"), "{err}");
        // And when installing too.
        let (store, dir) = temp_store("symlink");
        let err = store
            .install(&bytes, "test", "0.1.0")
            .unwrap_err()
            .to_string();
        assert!(err.contains("symlink"), "{err}");
        assert!(!dir.join("test").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Unique temp dir per test (tests run in parallel).
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
    fn path_traversal_rejected_on_install() {
        let mut entries = good_entries();
        entries.push(("../../escape.sh", b"echo pwned".to_vec()));
        let bytes = zip_bytes(&bytes_of(&entries));
        let (store, dir) = temp_store("traversal");
        let err = store
            .install(&bytes, "test", "0.1.0")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unsafe"), "{err}");
        // Nothing was written (not even staging): validation comes first.
        assert!(!dir.join("test").exists(), "garbage left in {:?}", dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn uninstall_removes_and_reports_missing() {
        let bytes = zip_bytes(&bytes_of(&good_entries()));
        let (store, dir) = temp_store("uninstall");
        store.install(&bytes, "test", "0.1.0").unwrap();
        assert!(store.uninstall("test", "0.1.0").unwrap());
        assert!(store.installed().is_empty());
        // Gone already: reported as false, not an error.
        assert!(!store.uninstall("test", "0.1.0").unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_list_and_no_overwrite() {
        let bytes = zip_bytes(&bytes_of(&good_entries()));
        let (store, dir) = temp_store("install");
        let (path, installed) = store.install(&bytes, "test", "0.1.0").unwrap();
        assert!(installed);
        assert!(path.join("wallpaper.json").is_file());
        assert!(path.join("main.wgsl").is_file());

        let (_, installed_again) = store.install(&bytes, "test", "0.1.0").unwrap();
        assert!(!installed_again, "must not overwrite");

        let list = store.installed();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "test");
        assert_eq!(list[0].1, "0.1.0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zip_bomb_rejected() {
        // Entry declaring a huge size: the limit applies BEFORE
        // materializing anything (central directory scan).
        let mut entries = good_entries();
        entries.push(("assets/huge.bin", vec![0u8; 40 * 1024 * 1024]));
        let bytes = zip_bytes(&bytes_of(&entries));
        let err = Store::load_manifest(&bytes).unwrap_err().to_string();
        assert!(err.contains("too large"), "{err}");
        let (store, dir) = temp_store("bomb");
        let err = store
            .install(&bytes, "test", "0.1.0")
            .unwrap_err()
            .to_string();
        assert!(err.contains("too large"), "{err}");
        assert!(!dir.join("test").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pack_creates_valid_zip() {
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
        assert_eq!(m.title, "Test");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
