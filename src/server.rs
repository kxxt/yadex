use axum::response::{IntoResponse, Response};
use snafu::{ResultExt, Snafu};
use tracing::error;

#[axum::debug_handler]
pub async fn root() -> Result<String, YadexError> {
    let meta = tokio::fs::metadata(".").await?;
    Ok(meta.created().map(|c| format!("{c:?}"))?)
}

#[derive(Debug, Snafu)]
pub enum YadexError {
    #[snafu(
        context(false),
        display("The resource you are requesting does not exist")
    )]
    NotFound { source: std::io::Error },
    #[snafu(whatever, display("{message}"))]
    Whatever {
        source: Option<color_eyre::Report>,
        message: String,
    },
}

impl IntoResponse for YadexError {
    fn into_response(self) -> Response {
        match self {
            YadexError::NotFound { .. } => "404 Not Found".into_response(),
            YadexError::Whatever { source, message } => {
                error!("internal error: {message}, source: {source:?}");
                "Internal Server Error".into_response()
            }
        }
    }
}
