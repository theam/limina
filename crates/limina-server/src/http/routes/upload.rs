//! `POST /v1/projects/{slug}/sources/upload`: a `multipart/form-data` file stored in the
//! project's workspace and registered as an `UPLOAD` source.
//!
//! The file streams straight to disk ([`crate::app::ProjectOperations::upload_source`] refuses
//! it past `LIMINA_UPLOAD_LIMIT_BYTES` with a 422 and keeps nothing); the request body as a whole
//! is capped at that limit plus [`FORM_OVERHEAD_BYTES`] (413 beyond). Browsers send the `name`
//! field before the file. A client that sends the file first has it spooled to a temporary file
//! until the name arrives.

use std::io;
use std::path::PathBuf;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use axum::body::Bytes;
use axum::extract::{FromRequest, Multipart, Request, State};
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::http::request::Parts;
use futures::{Stream, StreamExt};
use limina_core::{LiminaError, Principal, internal, new_uuid};
use limina_domain::collaboration::Role;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWriteExt, ReadBuf};

use super::{Params, created};
use crate::app::Upload;
use crate::http::App;
use crate::http::call::{Call, Checked, read_body};
use crate::http::error::{ApiError, ApiResult};
use crate::http::requests::UploadSourceForm;
use crate::http::schemas::{ActorHeader, PublicErrors, SourceResponse};
use crate::http::validate::{Loc, Text, check_text};

/// Room for the multipart framing and the `name` field on top of the upload limit.
pub const FORM_OVERHEAD_BYTES: u64 = 1024 * 1024;
/// The largest text field (Starlette's `max_part_size`).
const TEXT_FIELD_LIMIT: usize = 1024 * 1024;

/// Uploads a file as a source (editors).
#[utoipa::path(post, path = "/v1/projects/{slug}/sources/upload",
    operation_id = "upload_source_v1_projects__slug__sources_upload_post", summary = "Upload Source",
    params(("slug" = String, Path), ActorHeader),
    request_body(content = UploadSourceForm, content_type = "multipart/form-data"),
    responses((status = 201, description = "Successful Response", body = SourceResponse), PublicErrors),
    security(("HTTPBearer" = [])))]
pub async fn upload_source(
    State(app): State<App>,
    Params(slug): Params<String>,
    request: Request,
) -> ApiResult {
    let (parts, body) = request.into_parts();
    let mut call = Call::new(parts.clone());
    // An over-long `X-Limina-Actor` is answered before the form is read.
    let principal = call.caller().await?;
    call.check()?;
    let principal = principal.checked()?;
    let upload = Uploading { app: &app, slug: &slug, principal: &principal };
    match form_kind(&parts) {
        FormKind::Multipart => {
            let request = Request::from_parts(parts, body);
            match Multipart::from_request(request, &()).await {
                Ok(multipart) => upload.multipart(multipart, call).await,
                Err(_) => upload.fields(call, None, None),
            }
        }
        FormKind::UrlEncoded => {
            let limit = app.settings.json_body_limit_bytes;
            let bytes = read_body(body, limit).await?;
            let pairs: Vec<(String, String)> =
                url::form_urlencoded::parse(&bytes).into_owned().collect();
            let last = |key: &str| {
                pairs.iter().rev().find(|(name, _)| name == key).map(|(_, value)| value.clone())
            };
            upload.fields(call, last("name"), last("file"))
        }
        FormKind::Other => upload.fields(call, None, None),
    }
}

enum FormKind {
    Multipart,
    UrlEncoded,
    Other,
}

fn form_kind(parts: &Parts) -> FormKind {
    let content_type = parts
        .headers
        .get(CONTENT_TYPE)
        .map(|value| String::from_utf8_lossy(value.as_bytes()).to_ascii_lowercase())
        .unwrap_or_default();
    let essence = content_type.split(';').next().unwrap_or_default().trim().to_owned();
    match essence.as_str() {
        "multipart/form-data" => FormKind::Multipart,
        "application/x-www-form-urlencoded" => FormKind::UrlEncoded,
        _ => FormKind::Other,
    }
}

struct Uploading<'a> {
    app: &'a App,
    slug: &'a str,
    principal: &'a Principal,
}

impl Uploading<'_> {
    /// The validation errors of a form without a usable file part: `name` first, then `file`
    /// (missing, or sent as a plain text field).
    fn fields(&self, mut call: Call, name: Option<String>, file_text: Option<String>) -> ApiResult {
        valid_name(&mut call, name.as_deref());
        let loc = Loc::body().field("file");
        match file_text {
            Some(text) => call.errors().push_ctx(
                "value_error",
                &loc,
                "Value error, Expected UploadFile, received: <class 'str'>",
                json!(text),
                json!({"error": {}}),
            ),
            None => call.errors().missing(&loc, Value::Null),
        }
        call.check()?;
        Err(ApiError::Limina(internal("an upload form without a file passed validation")))
    }

    async fn multipart(&self, mut multipart: Multipart, mut call: Call) -> ApiResult {
        let mut name: Option<String> = None;
        let mut file_text: Option<String> = None;
        let mut spooled: Option<Spooled> = None;
        loop {
            let field = match multipart.next_field().await {
                Ok(Some(field)) => field,
                Ok(None) => break,
                Err(error) => return Err(multipart_error(error.status(), self.limit())),
            };
            match (field.name(), field.file_name().is_some()) {
                (Some("name"), false) => name = Some(text_field(field).await?),
                (Some("file"), false) => file_text = Some(text_field(field).await?),
                (Some("file"), true) if spooled.is_none() => {
                    let filename = field.file_name().map(str::to_owned);
                    let content_type = field.content_type().map(str::to_owned);
                    let Some(name) = name.clone() else {
                        // The name has not arrived yet: keep the file until it does.
                        self.authorize().await?;
                        spooled = Some(spool(field, filename, content_type, self.limit()).await?);
                        continue;
                    };
                    if !valid_name(&mut call, Some(&name)) {
                        call.check()?;
                    }
                    self.authorize().await?;
                    let meta = Upload {
                        name: &name,
                        filename: filename.as_deref(),
                        content_type: content_type.as_deref(),
                    };
                    let mut reader = FieldReader::new(field);
                    let stored = self.store(meta, &mut reader).await;
                    return match (stored, reader.failure) {
                        (Ok(source), _) => created(source),
                        (Err(_), Some(status)) => Err(multipart_error(status, self.limit())),
                        (Err(error), None) => Err(error),
                    };
                }
                _ => {}
            }
        }
        let Some(spooled) = spooled else {
            return self.fields(call, name, file_text);
        };
        let outcome = self.store_spooled(&mut call, name.as_deref(), &spooled).await;
        let _ = tokio::fs::remove_file(&spooled.path).await;
        outcome
    }

    async fn store_spooled(
        &self,
        call: &mut Call,
        name: Option<&str>,
        spooled: &Spooled,
    ) -> ApiResult {
        if !valid_name(call, name) {
            call.check()?;
        }
        let name = name.unwrap_or_default();
        let meta = Upload {
            name,
            filename: spooled.filename.as_deref(),
            content_type: spooled.content_type.as_deref(),
        };
        let file = tokio::fs::File::open(&spooled.path).await.map_err(internal)?;
        created(self.store(meta, file).await?)
    }

    async fn store<R>(
        &self,
        meta: Upload<'_>,
        reader: R,
    ) -> ApiResult<limina_domain::knowledge::Source>
    where
        R: AsyncRead + Unpin + Send,
    {
        let operations = self.app.runtime.operations();
        Ok(operations.upload_source(self.slug, meta, reader, self.principal).await?)
    }

    /// Python checked the editor role before storing anything.
    async fn authorize(&self) -> ApiResult<()> {
        let collaboration = self.app.runtime.collaboration();
        collaboration.require_role(self.slug, self.principal, Role::Editor).await?;
        Ok(())
    }

    fn limit(&self) -> u64 {
        self.app.settings.upload_limit_bytes
    }
}

/// Validates the `name` field; an empty value counts as missing, as FastAPI treats empty form
/// values.
fn valid_name(call: &mut Call, name: Option<&str>) -> bool {
    let loc = Loc::body().field("name");
    match name.filter(|name| !name.is_empty()) {
        None => {
            call.errors().missing(&loc, Value::Null);
            false
        }
        Some(name) => check_text(call.errors(), &loc, name, Text::between(1, 200)),
    }
}

async fn text_field(mut field: axum::extract::multipart::Field<'_>) -> ApiResult<String> {
    let mut text = Vec::new();
    while let Some(chunk) =
        field.chunk().await.map_err(|error| multipart_error(error.status(), 0))?
    {
        if text.len() + chunk.len() > TEXT_FIELD_LIMIT {
            return Err(ApiError::Unreadable);
        }
        text.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&text).into_owned())
}

fn multipart_error(status: StatusCode, limit: u64) -> ApiError {
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::TooLarge { limit_bytes: limit.saturating_add(FORM_OVERHEAD_BYTES) }
    } else {
        ApiError::Unreadable
    }
}

/// A file part kept on disk until the form's `name` arrives.
struct Spooled {
    path: PathBuf,
    filename: Option<String>,
    content_type: Option<String>,
}

async fn spool(
    mut field: axum::extract::multipart::Field<'_>,
    filename: Option<String>,
    content_type: Option<String>,
    limit: u64,
) -> ApiResult<Spooled> {
    let path = std::env::temp_dir().join(format!("limina-upload-{}", new_uuid()));
    let spooled = Spooled { path, filename, content_type };
    let result: ApiResult<()> = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&spooled.path)
            .await
            .map_err(internal)?;
        let mut size: u64 = 0;
        while let Some(chunk) =
            field.chunk().await.map_err(|error| multipart_error(error.status(), limit))?
        {
            size = size.saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
            if size > limit {
                return Err(LiminaError::invariant(format!(
                    "Uploaded sources must be at most {limit} bytes."
                ))
                .into());
            }
            file.write_all(&chunk).await.map_err(internal)?;
        }
        file.flush().await.map_err(internal)?;
        Ok(())
    }
    .await;
    match result {
        Ok(()) => Ok(spooled),
        Err(error) => {
            let _ = tokio::fs::remove_file(&spooled.path).await;
            Err(error)
        }
    }
}

/// Reads a multipart file part as an [`AsyncRead`], remembering why the stream failed.
struct FieldReader<S> {
    stream: S,
    chunk: Bytes,
    failure: Option<StatusCode>,
}

impl<S> FieldReader<S> {
    fn new(stream: S) -> Self {
        Self { stream, chunk: Bytes::new(), failure: None }
    }
}

impl<S> AsyncRead for FieldReader<S>
where
    S: Stream<Item = Result<Bytes, axum::extract::multipart::MultipartError>> + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            if !self.chunk.is_empty() {
                let count = buf.remaining().min(self.chunk.len());
                let part = self.chunk.split_to(count);
                buf.put_slice(&part);
                return Poll::Ready(Ok(()));
            }
            match ready!(self.stream.poll_next_unpin(cx)) {
                Some(Ok(chunk)) => self.chunk = chunk,
                Some(Err(error)) => {
                    self.failure = Some(error.status());
                    return Poll::Ready(Err(io::Error::other(error.body_text())));
                }
                None => return Poll::Ready(Ok(())),
            }
        }
    }
}
