//! Errores del formato `.wallpaper`.

/// Errores al cargar, validar, instalar o empaquetar un paquete.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// El archivo no es un ZIP legible.
    #[error("no es un ZIP válido: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// El manifiesto no parsea como JSON.
    #[error("JSON inválido en {0}: {1}")]
    Json(String, #[source] serde_json::Error),
    /// Falta un archivo requerido en el paquete.
    #[error("falta {0} en el paquete")]
    Missing(String),
    /// La versión del schema no es la que habla este motor.
    #[error("schema de manifiesto no soportado: format={0} (este motor habla format={1})")]
    Format(u32, u32),
    /// El tipo está reservado (video/web) pero aún no está implementado.
    #[error("tipo de wallpaper '{0}' reservado en el schema y aún sin implementar")]
    ReservedType(String),
    /// El tipo no existe en el schema.
    #[error("tipo de wallpaper no soportado: '{0}' (implementado: shader)")]
    BadType(String),
    /// La entrada no es una ruta segura con extensión .wgsl.
    #[error("entry debe ser una ruta segura a un .wgsl, se recibió '{0}'")]
    BadEntry(String),
    /// La preview no es una ruta segura a png/jpg.
    #[error("preview debe ser una ruta segura a .png/.jpg, se recibió '{0}'")]
    BadPreview(String),
    /// Una ruta que escapa del directorio del paquete (path traversal).
    #[error("ruta insegura dentro del paquete: '{0}'")]
    UnsafePath(String),
    /// El paquete contiene un symlink (vector de escape clásico).
    #[error("symlink dentro del paquete: '{0}' (no permitido)")]
    Symlink(String),
    /// Demasiados archivos.
    #[error("demasiados archivos en el paquete: {0} (máx {1})")]
    TooManyFiles(usize, usize),
    /// Un archivo supera el límite (zip bomb).
    #[error("archivo demasiado grande: {0} ({1} bytes máx)")]
    FileTooBig(String, u64),
    /// El tamaño descomprimido total supera el límite (zip bomb).
    #[error("paquete demasiado grande descomprimido: más de {0} bytes")]
    TotalTooBig(u64),
    /// Un parámetro del manifiesto es inválido.
    #[error("parámetro inválido: {0}")]
    BadParam(String),
    /// Ningún paquete instalado tiene ese nombre.
    #[error("no hay ningún paquete instalado llamado '{0}'")]
    NotInstalled(String),
    /// El paquete pide un motor más nuevo que este.
    #[error("el paquete requiere motor >= {0} y este es {1}")]
    Engine(String, String),
    /// El título no permite derivar un identificador de instalación.
    #[error(
        "el título '{0}' no permite derivar un nombre de instalación (usa letras, números y espacios)"
    )]
    BadTitle(String),
    /// Error de E/S del sistema.
    #[error("error de E/S: {0}")]
    Io(#[from] std::io::Error),
}
