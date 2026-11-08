//! Errors of the world read path.

/// A world file could not be read, was malformed, or a stored chunk does not
/// hold the fields this model requires.
///
/// Everything here means "the bytes in front of us are not what the reader
/// understands": reading never falls back to a guess and never fabricates a
/// default value.
#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    /// The region file could not be opened or read.
    #[error("region {path} cannot be read: {source}")]
    Io {
        /// Path that was read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The region layout or a NBT payload inside it is malformed.
    #[error("region {path} is malformed: {detail}")]
    Corrupt {
        /// Path of the failing file.
        path: String,
        /// What exactly did not parse.
        detail: String,
    },
    /// The sector announces a compression type the Anvil layout does not define.
    #[error("region {path}: unknown sector compression type {kind}")]
    UnknownCompression {
        /// Path of the failing file.
        path: String,
        /// The observed compression byte.
        kind: u8,
    },
    /// A world document (`level.dat`) could not be opened or read.
    #[error("world file {path} cannot be read: {source}")]
    WorldFile {
        /// Path of the world file that failed to read.
        path: String,
        /// Underlying I/O error (including a broken gzip container).
        #[source]
        source: std::io::Error,
    },
    /// A world document (`level.dat`) holds something the reader does not
    /// recognise (wrong container, missing or malformed field).
    #[error("world document {path} is malformed: {detail}")]
    WorldDocument {
        /// Path of the failing world document.
        path: String,
        /// What exactly did not parse.
        detail: String,
    },
    /// A stored chunk does not carry a field the model needs.
    #[error("chunk ({x}, {z}) has no usable field {field}")]
    MissingField {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Name of the missing NBT field.
        field: &'static str,
    },
    /// A field exists but carries a tag type or shape the reader rejects.
    #[error("chunk ({x}, {z}) field {field} has the wrong type")]
    WrongType {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Name of the failing NBT field.
        field: &'static str,
    },
}
