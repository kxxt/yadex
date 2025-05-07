use std::{
    env::set_current_dir,
    fs, io,
    os::unix::fs::{chroot, MetadataExt},
    path::{Path, PathBuf},
    sync::Arc,
};

use axum::{
    extract::State,
    http::Uri,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};
use chrono::Utc;
use futures_util::StreamExt as SExt;
use handlebars::RenderError;
use serde::Serialize;
use snafu::{ResultExt, Snafu};
use tokio::{fs::DirEntry, net::TcpListener};
use tokio_stream::wrappers::ReadDirStream;
use tracing::error;

use crate::config::{ServiceConfig, TemplateConfig};

pub struct App {}

pub struct Template {
    registry: handlebars::Handlebars<'static>,
}

#[derive(Debug, Snafu)]
pub enum TemplateLoadError {
    #[snafu(display("failed to load {component} template from {path:?}: {source}"))]
    Io {
        path: PathBuf,
        source: std::io::Error,
        component: &'static str,
    },
    Register {
        component: &'static str,
        source: handlebars::TemplateError,
    },
}

impl Template {
    pub fn from_config(
        path_to_config: &Path,
        config: TemplateConfig,
    ) -> Result<Self, TemplateLoadError> {
        let mut registry = handlebars::Handlebars::new();
        let config_dir = path_to_config.parent().unwrap();
        let index_path = config_dir.join(config.index_file);
        let index = std::fs::read_to_string(&index_path).context(IoSnafu {
            component: "index",
            path: index_path,
        })?;
        registry
            .register_template_string("index", index)
            .context(RegisterSnafu { component: "index" })?;
        let error_path = config_dir.join(config.error_file);
        let error = std::fs::read_to_string(&error_path).context(IoSnafu {
            component: "error",
            path: error_path,
        })?;
        registry
            .register_template_string("error", error)
            .context(RegisterSnafu { component: "error" })?;
        Ok(Self { registry })
    }

    pub fn render<T>(&self, name: &str, data: &T) -> Result<String, RenderError>
    where
        T: Serialize,
    {
        self.registry.render(name, data)
    }
}

impl App {
    pub async fn serve(
        config: ServiceConfig,
        listener: TcpListener,
        template: Template,
    ) -> Result<(), YadexError> {
        let router = Router::new()
            .fallback(get(directory_listing))
            .with_state(AppState {
                limit: if config.limit == 0 {
                    usize::MAX
                } else {
                    config.limit as usize
                },
                template: Arc::new(template),
            });
        let root: &'static Path = Box::leak(Box::<Path>::from(config.root));
        chroot(root).whatever_context("failed to chroot")?;
        set_current_dir("/").whatever_context("failed to cd into new root")?;
        axum::serve(listener, router)
            .await
            .with_whatever_context(|_| "serve failed")
    }
}

#[derive(Clone)]
pub struct AppState {
    limit: usize,
    template: Arc<Template>,
}

#[derive(Debug, Clone, Serialize)]
struct DirEntryInfo {
    name: String,
    is_dir: bool,
    size: u64,
    href: String,
    datetime: Option<chrono::DateTime<Utc>>,
}

pub async fn direntry_info(val: Result<DirEntry, io::Error>) -> Option<(DirEntry, fs::Metadata)> {
    let val = val.ok()?;
    let meta = val.metadata().await.ok()?;
    Some((val, meta))
}

#[derive(Debug, Clone, Serialize)]
struct IndexData<'a> {
    entry: &'a [DirEntryInfo],
    maybe_truncated: bool,
}

#[axum::debug_handler]
pub async fn directory_listing(
    State(state): State<AppState>,
    uri: Uri,
) -> Result<Response, YadexError> {
    let path = uri.path();

    if !path.ends_with('/') {
        return Ok(Redirect::permanent(&format!("{path}/")).into_response());
    }

    let entries = ReadDirStream::new(tokio::fs::read_dir(path).await.context(NotFoundSnafu)?)
        .take(state.limit)
        .filter_map(async |entry| match direntry_info(entry).await {
            Some((d, meta)) => {
                let name = d.file_name();
                let name = name.to_string_lossy();
                Some(DirEntryInfo {
                    is_dir: meta.is_dir(),
                    size: meta.size(),
                    href: format!(
                        "{path}{file}{slash}",
                        file = html_escape::encode_double_quoted_attribute(&urlencoding::encode(
                            &name
                        )),
                        slash = if meta.is_dir() { "/" } else { "" }
                    ),
                    name: name.into_owned(),
                    datetime: chrono::DateTime::from_timestamp(meta.mtime(), 0),
                })
            }
            None => None,
        })
        .collect::<Vec<_>>()
        .await;
    let html = state
        .template
        .render(
            "index",
            &IndexData {
                entry: &entries,
                maybe_truncated: entries.len() == state.limit,
            },
        )
        .context(RenderSnafu { template: "index" })?;
    Ok(Html(html).into_response())
}

#[derive(Debug, Snafu)]
pub enum YadexError {
    #[snafu(display("The resource you are requesting does not exist"))]
    NotFound { source: std::io::Error },
    #[snafu(whatever, display("{message}"))]
    Whatever {
        #[snafu(source(from(color_eyre::Report, Some)))]
        source: Option<color_eyre::Report>,
        message: String,
    },
    #[snafu(display("The template {template} failed to render"))]
    Render {
        source: RenderError,
        template: &'static str,
    },
}

impl IntoResponse for YadexError {
    fn into_response(self) -> Response {
        match &self {
            YadexError::NotFound { .. } => "404 Not Found".into_response(),
            YadexError::Whatever { source, message } => {
                error!("internal error: {message}, source: {source:?}");
                "Internal Server Error".into_response()
            }
            YadexError::Render { source, .. } => {
                error!("internal error: {self}, source: {source:?}");
                "Internal Server Error".into_response()
            }
        }
    }
}
