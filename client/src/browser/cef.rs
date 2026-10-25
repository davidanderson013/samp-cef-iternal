use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::CloseHandle;
use winapi::um::libloaderapi::GetModuleHandleA;
use winapi::um::synchapi::CreateMutexW;

use cef::app::App;
use cef::command_line::CommandLine;
use cef::handlers::browser_process::BrowserProcessHandler;
use cef::handlers::render_process::RenderProcessHandler;
use cef::types::string::CefString;

use crossbeam_channel::Sender;

use crate::app::Event;
use crate::browser::client::{WebClient, WebClientRef};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

static ROOT_CACHE_MUTEX: AtomicUsize = AtomicUsize::new(0);

fn cache_mutex_name(cef_dir: &Path) -> Vec<u16> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cef_dir.to_string_lossy().to_lowercase().hash(&mut hasher);
    format!("Local\\samp-cef-root-cache-{:016x}", hasher.finish())
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

fn select_root_cache_path(cef_dir: &Path) -> PathBuf {
    let mutex_name = cache_mutex_name(cef_dir);
    let mutex = unsafe { CreateMutexW(std::ptr::null_mut(), 0, mutex_name.as_ptr()) };

    if mutex.is_null() {
        tracing::error!("cannot create CEF cache ownership mutex");
        return cef_dir
            .join("user_data")
            .join("instances")
            .join(std::process::id().to_string());
    }

    let secondary_instance = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    ROOT_CACHE_MUTEX.store(mutex as usize, Ordering::Release);

    if secondary_instance {
        cef_dir
            .join("user_data")
            .join("instances")
            .join(std::process::id().to_string())
    } else {
        cef_dir.join("user_data")
    }
}

fn release_root_cache_mutex() {
    let mutex = ROOT_CACHE_MUTEX.swap(0, Ordering::AcqRel);
    if mutex != 0 {
        unsafe {
            CloseHandle(mutex as *mut _);
        }
    }
}

#[derive(Clone)]
struct DefaultApp {
    event_tx: Sender<Event>,
}

impl RenderProcessHandler for DefaultApp {}
impl BrowserProcessHandler for DefaultApp {
    fn on_context_initialized(&self) {
        tracing::debug!("CEF browser context initialized");
        crate::browser::assets_scheme::register_scheme_handler_factory();
        let _ = self.event_tx.send(Event::CefInitialize);
    }
}

impl App for DefaultApp {
    type RenderProcessHandler = Self;
    type BrowserProcessHandler = Self;

    fn browser_process_handler(&self) -> Option<Self::BrowserProcessHandler> {
        Some(self.clone())
    }

    fn on_before_command_line_processing(
        &self, _process_type: CefString, command_line: CommandLine,
    ) {
        command_line.append_switch("disable-gpu-compositing");
        command_line.append_switch("disable-gpu");
        command_line.append_switch("enable-begin-frame-scheduling");
        command_line.append_switch_with_value("autoplay-policy", "no-user-gesture-required");
        command_line.append_switch("ignore-certificate-errors");

        // TODO: permissions
        command_line.append_switch("enable-media-stream");
    }

    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn on_register_custom_schemes(&self, registrar: *mut cef_sys::cef_scheme_registrar_t) {
        unsafe {
            crate::browser::assets_scheme::register_custom_scheme(registrar);
        }
    }
}

pub fn initialize(event_tx: Sender<Event>) -> bool {
    let instance = unsafe { GetModuleHandleA(std::ptr::null()) };
    let main_args = cef_sys::cef_main_args_t {
        instance: instance as *mut _,
    };

    let mut settings = unsafe { std::mem::zeroed::<cef_sys::cef_settings_t>() };

    let cef_dir = crate::utils::cef_dir();
    let root_cache_path = select_root_cache_path(&cef_dir);
    let cache_path = root_cache_path.join("cache");

    tracing::debug!(directory = %cef_dir.display(), "configuring CEF runtime");

    let path =
        cef::types::string::into_cef_string(&cef_dir.join("cef-renderer.exe").to_string_lossy());
    let cache_path = cef::types::string::into_cef_string(&cache_path.to_string_lossy());
    let locales_dir_path =
        cef::types::string::into_cef_string(&cef_dir.join("locales").to_string_lossy());
    let resources_dir_path = cef::types::string::into_cef_string(&cef_dir.to_string_lossy());
    let log_file = cef::types::string::into_cef_string(&cef_dir.join("cef.log").to_string_lossy());
    let root_cache_path = cef::types::string::into_cef_string(&root_cache_path.to_string_lossy());

    settings.size = std::mem::size_of::<cef_sys::cef_settings_t>();
    settings.no_sandbox = 1;
    settings.browser_subprocess_path = path;
    settings.windowless_rendering_enabled = 1;
    settings.multi_threaded_message_loop = 1;
    settings.log_severity = cef_sys::cef_log_severity_t::LOGSEVERITY_ERROR;
    settings.cache_path = cache_path;
    settings.root_cache_path = root_cache_path;
    settings.locales_dir_path = locales_dir_path;
    settings.resources_dir_path = resources_dir_path;
    settings.log_file = log_file;

    let app = DefaultApp { event_tx };

    let initialized = cef::initialize(Some(&main_args), &settings, Some(app)) != 0;

    if !initialized {
        release_root_cache_mutex();
    }

    tracing::debug!(initialized, "CEF runtime initialization completed");
    initialized
}

pub fn shutdown() {
    cef::shutdown();
    release_root_cache_mutex();
}

pub fn create_browser(client: Arc<WebClient>, url: &str) {
    let mut window_info = unsafe { std::mem::zeroed::<cef_sys::cef_window_info_t>() };

    window_info.size = std::mem::size_of::<cef_sys::cef_window_info_t>();
    window_info.parent_window = client_api::gta::hwnd() as *mut _;
    window_info.windowless_rendering_enabled = 1;

    let url = crate::browser::assets_scheme::resolve_browser_url(url);
    let url = CefString::new(&url);

    let mut settings = unsafe { std::mem::zeroed::<cef_sys::cef_browser_settings_t>() };

    settings.size = std::mem::size_of::<cef_sys::cef_browser_settings_t>();
    settings.windowless_frame_rate = 60;
    settings.javascript_access_clipboard = cef_sys::cef_state_t::STATE_ENABLED;
    settings.javascript_dom_paste = cef_sys::cef_state_t::STATE_ENABLED;
    settings.remote_fonts = cef_sys::cef_state_t::STATE_ENABLED;
    settings.webgl = cef_sys::cef_state_t::STATE_ENABLED;
    settings.javascript = cef_sys::cef_state_t::STATE_ENABLED;

    let client = WebClientRef::from(client);
    let result =
        cef::browser::BrowserHost::create_browser(&window_info, Some(client), &url, &settings);

    tracing::debug!(accepted = result, "CEF browser creation submitted");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_separate_root_cache_for_a_secondary_instance() {
        let cef_dir = PathBuf::from(format!(
            r"C:\samp-cef-root-cache-test-{}",
            std::process::id()
        ));

        let primary_path = select_root_cache_path(&cef_dir);
        assert_eq!(primary_path, cef_dir.join("user_data"));
        release_root_cache_mutex();

        let mutex_name = cache_mutex_name(&cef_dir);
        let primary_mutex = unsafe { CreateMutexW(std::ptr::null_mut(), 0, mutex_name.as_ptr()) };
        assert!(!primary_mutex.is_null());

        let secondary_path = select_root_cache_path(&cef_dir);
        assert_eq!(
            secondary_path,
            cef_dir
                .join("user_data")
                .join("instances")
                .join(std::process::id().to_string())
        );

        release_root_cache_mutex();
        unsafe {
            CloseHandle(primary_mutex);
        }
    }
}
