use serde::{Serialize, Serializer};

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("document not found")]
    DocumentNotFound,
    #[error("engine unavailable")]
    EngineUnavailable,
    #[error("this PDF is password-protected")]
    PasswordRequired,
    #[error("malformed or unsupported PDF: {0}")]
    Malformed(String),
    #[error("i/o error: {0}")]
    Io(String),
    #[error("stale request")]
    Stale,
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Internal(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            AppError::DocumentNotFound => "not_found",
            AppError::EngineUnavailable => "engine",
            AppError::PasswordRequired => "password",
            AppError::Malformed(_) => "malformed",
            AppError::Io(_) => "io",
            AppError::Stale => "stale",
            AppError::Unsupported(_) => "unsupported",
            AppError::Internal(_) => "internal",
        }
    }
}

/// Serialized as `{ code, message }` so the frontend can branch on `code`.
impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("AppError", 2)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.to_string())
    }
}

impl From<pdfium_render::prelude::PdfiumError> for AppError {
    fn from(e: pdfium_render::prelude::PdfiumError) -> Self {
        AppError::Internal(format!("pdfium: {e:?}"))
    }
}
