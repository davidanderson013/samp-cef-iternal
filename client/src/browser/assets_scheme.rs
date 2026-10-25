use std::ffi::OsString;
use std::fs;
use std::os::raw::{c_int, c_void};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{self, AtomicUsize, Ordering};

use cef::types::string::CefString;
use cef_sys::{
    cef_base_ref_counted_t, cef_callback_t, cef_request_t, cef_resource_handler_t,
    cef_resource_read_callback_t, cef_resource_skip_callback_t, cef_response_t,
    cef_scheme_handler_factory_t, cef_scheme_options_t, cef_scheme_registrar_t, cef_string_t,
};
use percent_encoding::percent_decode_str;
use url::Url;

pub const ASSET_SCHEME: &str = "sampcef";
const ASSET_HOST: &str = "assets";
const BLANK_URL: &str = "about:blank";
const ASSET_SCHEME_OPTIONS: i32 = cef_scheme_options_t::CEF_SCHEME_OPTION_STANDARD
    | cef_scheme_options_t::CEF_SCHEME_OPTION_SECURE
    | cef_scheme_options_t::CEF_SCHEME_OPTION_CORS_ENABLED
    | cef_scheme_options_t::CEF_SCHEME_OPTION_FETCH_ENABLED;

#[repr(C)]
struct Wrapper<T, I> {
    cef_object: T,
    interface: I,
    ref_count: AtomicUsize,
}

impl<T, I> Wrapper<T, I> {
    fn new(mut cef_object: T, interface: I) -> Box<Self> {
        let base = unsafe { &mut *(&mut cef_object as *mut T as *mut cef_base_ref_counted_t) };
        base.size = std::mem::size_of::<T>();
        base.add_ref = Some(add_ref::<T, I>);
        base.has_one_ref = Some(has_one_ref::<T, I>);
        base.has_at_least_one_ref = Some(has_at_least_one_ref::<T, I>);
        base.release = Some(release::<T, I>);

        Box::new(Self {
            cef_object,
            interface,
            ref_count: AtomicUsize::new(1),
        })
    }

    fn into_cef(self: Box<Self>) -> *mut T {
        Box::into_raw(self) as *mut T
    }

    fn unwrap<'a>(ptr: *mut T) -> &'a mut Self {
        unsafe { &mut *(ptr as *mut Self) }
    }
}

extern "system" fn add_ref<T, I>(this: *mut cef_base_ref_counted_t) {
    let obj: &mut Wrapper<T, I> = Wrapper::unwrap(this as *mut T);
    obj.ref_count.fetch_add(1, Ordering::Relaxed);
}

extern "system" fn has_one_ref<T, I>(this: *mut cef_base_ref_counted_t) -> c_int {
    let obj: &mut Wrapper<T, I> = Wrapper::unwrap(this as *mut T);
    if obj.ref_count.load(Ordering::Relaxed) == 1 {
        1
    } else {
        0
    }
}

extern "system" fn has_at_least_one_ref<T, I>(this: *mut cef_base_ref_counted_t) -> c_int {
    let obj: &mut Wrapper<T, I> = Wrapper::unwrap(this as *mut T);
    if obj.ref_count.load(Ordering::Relaxed) >= 1 {
        1
    } else {
        0
    }
}

extern "system" fn release<T, I>(this: *mut cef_base_ref_counted_t) -> c_int {
    let obj: &mut Wrapper<T, I> = Wrapper::unwrap(this as *mut T);

    if obj.ref_count.fetch_sub(1, Ordering::Release) != 1 {
        0
    } else {
        atomic::fence(Ordering::Acquire);
        let _ = unsafe { Box::from_raw(this as *mut Wrapper<T, I>) };
        1
    }
}

#[derive(Default)]
struct AssetSchemeHandlerFactory;

#[derive(Default)]
struct AssetResourceHandler {
    body: Vec<u8>,
    offset: usize,
    mime_type: String,
    charset: String,
    status_code: i32,
    status_text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ContentType {
    mime_type: &'static str,
    charset: Option<&'static str>,
}

const TEXT_PLAIN_UTF8: ContentType = ContentType {
    mime_type: "text/plain",
    charset: Some("utf-8"),
};

impl AssetResourceHandler {
    fn reset(&mut self) {
        self.body.clear();
        self.offset = 0;
        self.mime_type.clear();
        self.charset.clear();
        self.status_code = 0;
        self.status_text.clear();
    }

    fn set_response(
        &mut self, status_code: i32, status_text: &str, content_type: ContentType, body: Vec<u8>,
    ) {
        self.body = body;
        self.offset = 0;
        self.mime_type.clear();
        self.mime_type.push_str(content_type.mime_type);
        self.charset.clear();
        if let Some(charset) = content_type.charset {
            self.charset.push_str(charset);
        }
        self.status_code = status_code;
        self.status_text.clear();
        self.status_text.push_str(status_text);
    }

    fn prepare_response(&mut self, request: *mut cef_request_t) {
        self.reset();

        let method = request_method(request);
        if !matches!(method.as_str(), "GET" | "HEAD") {
            self.set_response(
                405,
                "Method Not Allowed",
                TEXT_PLAIN_UTF8,
                b"Method Not Allowed".to_vec(),
            );
            return;
        }

        let request_url = request_url(request);
        let parsed = match Url::parse(&request_url) {
            Ok(url) => url,
            Err(_) => {
                self.set_response(400, "Bad Request", TEXT_PLAIN_UTF8, Vec::new());
                return;
            }
        };

        let Some(path) = resolve_asset_request_path(&parsed) else {
            self.set_response(403, "Forbidden", TEXT_PLAIN_UTF8, Vec::new());
            return;
        };

        // Prefer the decrypted in-memory asset bundle (ui.bin) over plain
        // files on disk. This is the actual content-protection step: as
        // long as the bundle is loaded, nothing ever gets read back from
        // a plaintext file, and the decrypted bytes only ever exist in
        // this process' memory for the duration of the response.
        let relative = fs::canonicalize(crate::utils::assets_dir())
            .ok()
            .and_then(|root| path.strip_prefix(&root).ok().map(|p| p.to_path_buf()));

        if let Some(relative) = relative.as_deref() {
            let rel_str = relative.to_string_lossy().replace('\\', "/");

            if let Some(body) = crate::browser::asset_bundle::read(&rel_str) {
                let body = if method == "HEAD" { Vec::new() } else { body };
                self.set_response(200, "OK", content_type_for_path(&path), body);
                return;
            }

            // The bundle is loaded but doesn't have this file: treat it
            // as missing rather than silently falling through to disk,
            // so a stale/partial bundle can't be bypassed by dropping a
            // plain file next to it.
            if crate::browser::asset_bundle::is_loaded() {
                self.set_response(404, "Not Found", TEXT_PLAIN_UTF8, Vec::new());
                return;
            }
        }

        // No bundle installed yet (e.g. still connecting, or a dev build
        // running without the server-provided manifest) — fall back to
        // plain files on disk so local iteration keeps working.
        match fs::read(&path) {
            Ok(body) => {
                let body = if method == "HEAD" { Vec::new() } else { body };
                self.set_response(200, "OK", content_type_for_path(&path), body);
            }

            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                self.set_response(404, "Not Found", TEXT_PLAIN_UTF8, Vec::new());
            }

            Err(_) => {
                self.set_response(500, "Internal Server Error", TEXT_PLAIN_UTF8, Vec::new());
            }
        }
    }
}

/// # Safety
/// `registrar` must be the valid registrar supplied by CEF to the app callback.
pub unsafe fn register_custom_scheme(registrar: *mut cef_scheme_registrar_t) {
    let result = unsafe {
        cef::scheme::register_custom_scheme(registrar, ASSET_SCHEME, ASSET_SCHEME_OPTIONS)
    };
    tracing::debug!(registered = result, "CEF asset scheme registered");
}

pub fn register_scheme_handler_factory() {
    let scheme = CefString::new(ASSET_SCHEME);
    let domain = CefString::new(ASSET_HOST);
    let factory = create_scheme_handler_factory();

    let result = unsafe {
        cef_sys::cef_register_scheme_handler_factory(
            scheme.as_cef_string(),
            domain.as_cef_string(),
            factory,
        )
    };

    if result == 0 {
        tracing::error!("cannot register CEF asset scheme handler");
    }
}

pub fn resolve_browser_url(value: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        return BLANK_URL.to_string();
    }

    if let Some((local_path, query, fragment)) = parse_file_url(value) {
        return file_path_to_asset_url(&local_path, query.as_deref(), fragment.as_deref())
            .unwrap_or_else(|| {
                tracing::warn!(url = %value, "blocked file URL outside assets directory");
                BLANK_URL.to_string()
            });
    }

    if let Some((query, fragment, path)) = parse_local_path_input(value) {
        return file_path_to_asset_url(&path, query.as_deref(), fragment.as_deref())
            .unwrap_or_else(|| {
                tracing::warn!(path = %value, "blocked local path outside assets directory");
                BLANK_URL.to_string()
            });
    }

    if is_valid_asset_url(value) || has_url_scheme(value) {
        return value.to_string();
    }

    BLANK_URL.to_string()
}

fn create_scheme_handler_factory() -> *mut cef_scheme_handler_factory_t {
    let mut object: cef_scheme_handler_factory_t = unsafe { std::mem::zeroed() };
    object.create = Some(factory_create);

    Wrapper::new(object, AssetSchemeHandlerFactory).into_cef()
}

fn create_resource_handler() -> *mut cef_resource_handler_t {
    let mut object: cef_resource_handler_t = unsafe { std::mem::zeroed() };
    object.open = Some(resource_open);
    object.process_request = Some(resource_process_request);
    object.get_response_headers = Some(resource_get_response_headers);
    object.skip = Some(resource_skip);
    object.read = Some(resource_read);
    object.read_response = Some(resource_read_response);
    object.cancel = Some(resource_cancel);

    Wrapper::new(object, AssetResourceHandler::default()).into_cef()
}

extern "system" fn factory_create(
    _this: *mut cef_scheme_handler_factory_t, _browser: *mut cef_sys::cef_browser_t,
    _frame: *mut cef_sys::cef_frame_t, _scheme_name: *const cef_string_t,
    _request: *mut cef_request_t,
) -> *mut cef_resource_handler_t {
    create_resource_handler()
}

extern "system" fn resource_open(
    this: *mut cef_resource_handler_t, request: *mut cef_request_t, handle_request: *mut c_int,
    _callback: *mut cef_callback_t,
) -> c_int {
    if request.is_null() {
        return 0;
    }

    let obj: &mut Wrapper<cef_resource_handler_t, AssetResourceHandler> = Wrapper::unwrap(this);
    obj.interface.prepare_response(request);

    if !handle_request.is_null() {
        unsafe {
            *handle_request = 1;
        }
    }

    1
}

extern "system" fn resource_process_request(
    this: *mut cef_resource_handler_t, request: *mut cef_request_t, callback: *mut cef_callback_t,
) -> c_int {
    if request.is_null() {
        return 0;
    }

    let obj: &mut Wrapper<cef_resource_handler_t, AssetResourceHandler> = Wrapper::unwrap(this);
    obj.interface.prepare_response(request);

    if !callback.is_null() {
        unsafe {
            if let Some(cont) = (*callback).cont {
                cont(callback);
            }
        }
    }

    1
}

extern "system" fn resource_get_response_headers(
    this: *mut cef_resource_handler_t, response: *mut cef_response_t, response_length: *mut i64,
    _redirect_url: *mut cef_string_t,
) {
    if response.is_null() {
        return;
    }

    let obj: &mut Wrapper<cef_resource_handler_t, AssetResourceHandler> = Wrapper::unwrap(this);
    let mime_type = CefString::new(&obj.interface.mime_type);
    let charset = CefString::new(&obj.interface.charset);
    let status_text = CefString::new(&obj.interface.status_text);

    unsafe {
        if let Some(set_status) = (*response).set_status {
            set_status(response, obj.interface.status_code);
        }

        if let Some(set_status_text) = (*response).set_status_text {
            set_status_text(response, status_text.as_cef_string());
        }

        if let Some(set_mime_type) = (*response).set_mime_type {
            set_mime_type(response, mime_type.as_cef_string());
        }

        if !obj.interface.charset.is_empty()
            && let Some(set_charset) = (*response).set_charset
        {
            set_charset(response, charset.as_cef_string());
        }

        if !response_length.is_null() {
            *response_length = obj.interface.body.len() as i64;
        }
    }

    tracing::trace!(
        status = obj.interface.status_code,
        mime_type = %obj.interface.mime_type,
        charset = %obj.interface.charset,
        bytes = obj.interface.body.len(),
        "asset response prepared"
    );
}

extern "system" fn resource_skip(
    this: *mut cef_resource_handler_t, bytes_to_skip: i64, bytes_skipped: *mut i64,
    _callback: *mut cef_resource_skip_callback_t,
) -> c_int {
    let obj: &mut Wrapper<cef_resource_handler_t, AssetResourceHandler> = Wrapper::unwrap(this);
    let remaining = obj
        .interface
        .body
        .len()
        .saturating_sub(obj.interface.offset);
    let skipped = remaining.min(bytes_to_skip.max(0) as usize);
    obj.interface.offset += skipped;

    if !bytes_skipped.is_null() {
        unsafe {
            *bytes_skipped = skipped as i64;
        }
    }

    if skipped > 0 { 1 } else { 0 }
}

extern "system" fn resource_read(
    this: *mut cef_resource_handler_t, data_out: *mut c_void, bytes_to_read: c_int,
    bytes_read: *mut c_int, _callback: *mut cef_resource_read_callback_t,
) -> c_int {
    read_body_chunk(this, data_out, bytes_to_read, bytes_read)
}

extern "system" fn resource_read_response(
    this: *mut cef_resource_handler_t, data_out: *mut c_void, bytes_to_read: c_int,
    bytes_read: *mut c_int, _callback: *mut cef_callback_t,
) -> c_int {
    read_body_chunk(this, data_out, bytes_to_read, bytes_read)
}

extern "system" fn resource_cancel(this: *mut cef_resource_handler_t) {
    let obj: &mut Wrapper<cef_resource_handler_t, AssetResourceHandler> = Wrapper::unwrap(this);
    obj.interface.reset();
}

fn read_body_chunk(
    this: *mut cef_resource_handler_t, data_out: *mut c_void, bytes_to_read: c_int,
    bytes_read: *mut c_int,
) -> c_int {
    let obj: &mut Wrapper<cef_resource_handler_t, AssetResourceHandler> = Wrapper::unwrap(this);
    let remaining = obj
        .interface
        .body
        .len()
        .saturating_sub(obj.interface.offset);
    let count = remaining.min(bytes_to_read.max(0) as usize);

    if !bytes_read.is_null() {
        unsafe {
            *bytes_read = count as c_int;
        }
    }

    if count == 0 {
        return 0;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(
            obj.interface.body.as_ptr().add(obj.interface.offset),
            data_out.cast::<u8>(),
            count,
        );
    }

    obj.interface.offset += count;
    1
}

fn request_url(request: *mut cef_request_t) -> String {
    unsafe {
        (*request)
            .get_url
            .map(|get_url| CefString::from(get_url(request)).to_string())
            .unwrap_or_default()
    }
}

fn request_method(request: *mut cef_request_t) -> String {
    unsafe {
        (*request)
            .get_method
            .map(|get_method| CefString::from(get_method(request)).to_string())
            .unwrap_or_else(|| String::from("GET"))
            .to_ascii_uppercase()
    }
}

fn parse_local_path_input(value: &str) -> Option<(Option<String>, Option<String>, PathBuf)> {
    if has_url_scheme(value) {
        return None;
    }

    let (path_part, query, fragment) = split_local_suffix(value);
    let path = Path::new(path_part);

    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        resolve_relative_local_path(path)?
    };

    Some((query, fragment, resolved))
}

fn parse_file_url(value: &str) -> Option<(PathBuf, Option<String>, Option<String>)> {
    let url = Url::parse(value).ok()?;
    if url.scheme() != "file" {
        return None;
    }

    let path = url.to_file_path().ok()?;
    Some((
        path,
        url.query().map(str::to_owned),
        url.fragment().map(str::to_owned),
    ))
}

fn resolve_relative_local_path(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    let first = components.next();
    let second = components.next();

    match (first, second) {
        (Some(Component::Normal(first)), Some(Component::Normal(second)))
            if component_eq(first, "cef") && component_eq(second, "assets") =>
        {
            Some(crate::utils::assets_dir().join(components.as_path()))
        }

        (Some(Component::Normal(first)), _) if component_eq(first, "cef") => None,
        _ => Some(crate::utils::assets_dir().join(path)),
    }
}

fn file_path_to_asset_url(
    path: &Path, query: Option<&str>, fragment: Option<&str>,
) -> Option<String> {
    // Content now normally comes from the in-memory encrypted bundle
    // (asset_bundle.rs), not from files on disk, so cef/assets/ is often
    // empty or was never created yet (e.g. first launch after a migration
    // script moved/removed it while the game was closed). canonicalize()
    // fails outright on a missing directory, which would otherwise block
    // every single asset path as "outside assets directory" and blank the
    // whole UI. App::new() already creates this directory on startup, but
    // we don't rely on init order here: create it defensively on first use
    // too, so a stale/empty directory never breaks path resolution.
    if !crate::utils::assets_dir().exists() {
        let _ = fs::create_dir_all(crate::utils::assets_dir());
    }
    let root = match fs::canonicalize(crate::utils::assets_dir()) {
        Ok(root) => root,
        Err(error) => {
            tracing::warn!(
                %error,
                assets_dir = %crate::utils::assets_dir().display(),
                "cannot canonicalize assets directory; blocking asset path"
            );
            return None;
        }
    };
    let normalized = normalize_path(path)?;
    let secured = canonicalize_with_missing_tail(&normalized)?;
    let relative = secured.strip_prefix(&root).ok()?;

    let mut url = Url::parse(&format!("{ASSET_SCHEME}://{ASSET_HOST}/")).ok()?;
    {
        let mut segments = url.path_segments_mut().ok()?;
        segments.clear();

        for component in relative.components() {
            if let Component::Normal(segment) = component {
                let segment = segment.to_string_lossy();
                segments.push(segment.as_ref());
            }
        }
    }

    url.set_query(query);
    url.set_fragment(fragment);
    Some(url.into())
}

fn resolve_asset_request_path(url: &Url) -> Option<PathBuf> {
    if url.scheme() != ASSET_SCHEME || url.host_str() != Some(ASSET_HOST) {
        return None;
    }

    let mut relative = PathBuf::new();

    for segment in url.path().split('/') {
        if segment.is_empty() {
            continue;
        }

        let decoded = percent_decode_str(segment).decode_utf8().ok()?;
        if decoded == "." {
            continue;
        }

        if decoded == ".." {
            return None;
        }

        relative.push(decoded.as_ref());
    }

    if relative.as_os_str().is_empty() || url.path().ends_with('/') {
        relative.push("index.html");
    }

    let candidate = crate::utils::assets_dir().join(relative);
    let normalized = normalize_path(&candidate)?;
    canonicalize_with_missing_tail(&normalized).and_then(|secured| {
        let root = fs::canonicalize(crate::utils::assets_dir()).ok()?;
        secured.strip_prefix(&root).ok()?;
        Some(secured)
    })
}

fn normalize_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}

            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }

            Component::Normal(segment) => normalized.push(segment),
        }
    }

    Some(normalized)
}

fn canonicalize_with_missing_tail(path: &Path) -> Option<PathBuf> {
    let normalized = normalize_path(path)?;
    let mut missing = Vec::<OsString>::new();
    let mut current = normalized.as_path();

    loop {
        if current.exists() {
            let mut canonical = fs::canonicalize(current).ok()?;

            for component in missing.iter().rev() {
                canonical.push(component);
            }

            return Some(canonical);
        }

        let name = current.file_name()?.to_os_string();
        missing.push(name);
        current = current.parent()?;
    }
}

fn split_local_suffix(value: &str) -> (&str, Option<String>, Option<String>) {
    let (without_fragment, fragment) = match value.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment.to_owned())),
        None => (value, None),
    };

    let (path, query) = match without_fragment.split_once('?') {
        Some((path, query)) => (path, Some(query.to_owned())),
        None => (without_fragment, None),
    };

    (path, query, fragment)
}

fn has_url_scheme(value: &str) -> bool {
    let Some(idx) = value.find(':') else {
        return false;
    };

    if idx == 1 {
        let rest = &value[idx + 1..];

        if value.as_bytes()[0].is_ascii_alphabetic()
            && (rest.starts_with('/') || rest.starts_with('\\'))
        {
            return false;
        }
    }

    let mut chars = value[..idx].chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
}

fn is_valid_asset_url(value: &str) -> bool {
    Url::parse(value)
        .ok()
        .map(|url| url.scheme() == ASSET_SCHEME && url.host_str() == Some(ASSET_HOST))
        .unwrap_or(false)
}

fn component_eq(component: &std::ffi::OsStr, expected: &str) -> bool {
    component.to_string_lossy().eq_ignore_ascii_case(expected)
}

fn content_type_for_path(path: &Path) -> ContentType {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("css") => ContentType {
            mime_type: "text/css",
            charset: Some("utf-8"),
        },
        Some("csv") => ContentType {
            mime_type: "text/csv",
            charset: Some("utf-8"),
        },
        Some("gif") => ContentType {
            mime_type: "image/gif",
            charset: None,
        },
        Some("htm") | Some("html") => ContentType {
            mime_type: "text/html",
            charset: Some("utf-8"),
        },
        Some("jpeg") | Some("jpg") => ContentType {
            mime_type: "image/jpeg",
            charset: None,
        },
        Some("js") | Some("mjs") => ContentType {
            mime_type: "text/javascript",
            charset: Some("utf-8"),
        },
        Some("json") => ContentType {
            mime_type: "application/json",
            charset: Some("utf-8"),
        },
        Some("mp3") => ContentType {
            mime_type: "audio/mpeg",
            charset: None,
        },
        Some("m4a") => ContentType {
            mime_type: "audio/mp4",
            charset: None,
        },
        Some("mp4") => ContentType {
            mime_type: "video/mp4",
            charset: None,
        },
        Some("ogg") => ContentType {
            mime_type: "audio/ogg",
            charset: None,
        },
        Some("png") => ContentType {
            mime_type: "image/png",
            charset: None,
        },
        Some("svg") => ContentType {
            mime_type: "image/svg+xml",
            charset: None,
        },
        Some("txt") => TEXT_PLAIN_UTF8,
        Some("wasm") => ContentType {
            mime_type: "application/wasm",
            charset: None,
        },
        Some("webm") => ContentType {
            mime_type: "video/webm",
            charset: None,
        },
        Some("webp") => ContentType {
            mime_type: "image/webp",
            charset: None,
        },
        Some("woff") => ContentType {
            mime_type: "font/woff",
            charset: None,
        },
        Some("woff2") => ContentType {
            mime_type: "font/woff2",
            charset: None,
        },
        Some("xml") => ContentType {
            mime_type: "application/xml",
            charset: Some("utf-8"),
        },
        _ => ContentType {
            mime_type: "application/octet-stream",
            charset: None,
        },
    }
}

#[cfg(test)]
mod content_type_tests {
    use super::{ContentType, content_type_for_path};
    use std::path::Path;

    #[test]
    fn text_types_keep_charset_separate_from_mime_type() {
        assert_eq!(
            content_type_for_path(Path::new("index.HTML")),
            ContentType {
                mime_type: "text/html",
                charset: Some("utf-8"),
            }
        );
        assert_eq!(
            content_type_for_path(Path::new("app.js")),
            ContentType {
                mime_type: "text/javascript",
                charset: Some("utf-8"),
            }
        );
    }

    #[test]
    fn binary_types_do_not_declare_a_charset() {
        assert_eq!(
            content_type_for_path(Path::new("movie.webm")),
            ContentType {
                mime_type: "video/webm",
                charset: None,
            }
        );
        assert_eq!(
            content_type_for_path(Path::new("movie.mp4")),
            ContentType {
                mime_type: "video/mp4",
                charset: None,
            }
        );
    }
}
