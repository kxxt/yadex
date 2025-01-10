use std::{
    borrow::Cow,
    env::set_current_dir,
    fmt::{Display, Write},
    fs, io,
    os::unix::fs::{chroot, MetadataExt},
    path::{Path, PathBuf},
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
use snafu::{ResultExt, Snafu};
use tokio::{fs::DirEntry, net::TcpListener};
use tokio_stream::wrappers::ReadDirStream;
use tracing::error;

use crate::config::{ServiceConfig, TemplateConfig};

pub struct App {}

pub struct Template {
    header: String,
    footer: String,
    error: String,
}

#[derive(Debug, Snafu)]
#[snafu(display("failed to load {component} template from {path:?}: {source}"))]
pub struct TemplateLoadError {
    component: &'static str,
    path: PathBuf,
    source: std::io::Error,
}

impl Template {
    pub fn from_config(
        path_to_config: &Path,
        config: TemplateConfig,
    ) -> Result<Self, TemplateLoadError> {
        let config_dir = path_to_config.parent().unwrap();
        let header_path = config_dir.join(config.header_file);
        let header = std::fs::read_to_string(&header_path).context(TemplateLoadSnafu {
            component: "header",
            path: header_path,
        })?;
        let footer_path = config_dir.join(config.footer_file);
        let footer = std::fs::read_to_string(&footer_path).context(TemplateLoadSnafu {
            component: "footer",
            path: footer_path,
        })?;
        let error_path = config_dir.join(config.error_file);
        let error = std::fs::read_to_string(&error_path).context(TemplateLoadSnafu {
            component: "error",
            path: error_path,
        })?;
        Ok(Self {
            header,
            footer,
            error,
        })
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
                header: template.header,
                footer: template.footer.leak(),
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
    header: String,
    footer: &'static str,
}

struct DirEntryInfo<'a> {
    name: Cow<'a, str>,
    is_dir: bool,
    size: u64,
    mtime: Option<chrono::DateTime<Utc>>,
}

pub fn append_slash_for_dir(name: Cow<'_, str>, is_dir: bool) -> Cow<'_, str> {
    if is_dir {
        match name {
            Cow::Borrowed(s) => Cow::Owned(format!("{s}/")),
            Cow::Owned(mut s) => {
                s.push('/');
                Cow::Owned(s)
            }
        }
    } else {
        name
    }
}

impl Display for DirEntryInfo<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            r#"<tr>
                   <td><a href="{attribute_safe_name}">{html_safe_name}</a></td>
                   <td>{datetime}</td>
                   <td>{filesize}</td>
               </tr>"#,
            html_safe_name =
                append_slash_for_dir(html_escape::encode_safe(&self.name), self.is_dir),
            attribute_safe_name = append_slash_for_dir(
                html_escape::encode_double_quoted_attribute(&self.name),
                self.is_dir
            ),
            datetime = self
                .mtime
                .map(|t| t.to_string())
                .map(Cow::Owned)
                .unwrap_or(Cow::Borrowed("Unknown")),
            filesize = self.size
        )
    }
}

pub async fn direntry_info(val: Result<DirEntry, io::Error>) -> Option<(DirEntry, fs::Metadata)> {
    let val = val.ok()?;
    let meta = val.metadata().await.ok()?;
    Some((val, meta))
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

    let mut read_dir = ReadDirStream::new(tokio::fs::read_dir(path).await.context(NotFoundSnafu)?)
        .enumerate()
        .take(state.limit);
    let mut html = state.header;

    while let Some((idx, r)) = read_dir.next().await {
        match direntry_info(r).await {
            Some((d, meta)) => {
                let _ = write!(
                    html,
                    "{}",
                    DirEntryInfo {
                        name: d.file_name().to_string_lossy(),
                        is_dir: meta.is_dir(),
                        size: meta.size(),
                        mtime: chrono::DateTime::from_timestamp(meta.mtime(), 0)
                    }
                );
            }
            None => {
                html.push_str(
                    r#"<tr class="entry-error"><td>error occurred while getting this entry</td></tr>"#,
                );
            }
        }

        if idx == state.limit - 1 {
            // Reached limit, results might be truncated.
            html.push_str(r#"<tr class="truncated-warning"><td>Too many entries. This list might be truncated.</td></tr>"#);
        }
    }
    // entries.map()
    html.push_str(state.footer);
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
