//! Errors of the `.wallpaper` format.

/// Errors when loading, validating, installing or packing a package.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// The file is not a readable ZIP.
    #[error("not a valid ZIP: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// The manifest does not parse as JSON.
    #[error("invalid JSON in {0}: {1}")]
    Json(String, #[source] serde_json::Error),
    /// A required file is missing from the package.
    #[error("missing {0} in the package")]
    Missing(String),
    /// The schema version is not this engine's.
    #[error("unsupported manifest schema: format={0} (this engine speaks format={1})")]
    Format(u32, u32),
    /// The type is reserved (video/web) but not implemented yet.
    #[error("wallpaper type '{0}' is reserved in the schema but not implemented yet")]
    ReservedType(String),
    /// The type does not exist in the schema.
    #[error("unsupported wallpaper type: '{0}' (implemented: shader)")]
    BadType(String),
    /// The entry is not a safe path with a .wgsl extension.
    #[error("entry must be a safe path to a .wgsl, got '{0}'")]
    BadEntry(String),
    /// The preview is not a safe path to png/jpg.
    #[error("preview must be a safe path to .png/.jpg, got '{0}'")]
    BadPreview(String),
    /// Path escaping the package directory (path traversal).
    #[error("unsafe path inside the package: '{0}'")]
    UnsafePath(String),
    /// The package contains a symlink (classic escape vector).
    #[error("symlink inside the package: '{0}' (not allowed)")]
    Symlink(String),
    /// Too many files.
    #[error("too many files in the package: {0} (maximum {1})")]
    TooManyFiles(usize, usize),
    /// A single file exceeds the limit (zip bomb).
    #[error("file too large: {0} ({1} bytes maximum)")]
    FileTooBig(String, u64),
    /// The total decompressed size exceeds the limit (zip bomb).
    #[error("package too large decompressed: more than {0} bytes")]
    TotalTooBig(u64),
    /// A manifest parameter is invalid.
    #[error("invalid parameter: {0}")]
    BadParam(String),
    /// There is no installed package with that name.
    #[error("no installed package named '{0}'")]
    NotInstalled(String),
    /// The package asks for a newer engine than this one.
    #[error("the package requires engine >= {0} and this is {1}")]
    Engine(String, String),
    /// The title does not allow deriving an installation identifier.
    #[error(
        "the title '{0}' does not allow deriving an install name (use letters, numbers and spaces)"
    )]
    BadTitle(String),
    /// System I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
