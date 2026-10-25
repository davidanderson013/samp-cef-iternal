use std::borrow::Cow;
use std::net::{IpAddr, SocketAddr};
use std::sync::Once;

use server_core::{CoreEvent, EventValue, ServerCore as InnerServerCore};
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

static TRACING_INIT: Once = Once::new();
const LOG_LEVEL_ENV: &str = "SAMP_CEF_LOG_LEVEL";

fn server_log_level() -> (LevelFilter, Option<String>) {
    let Ok(value) = std::env::var(LOG_LEVEL_ENV) else {
        return (LevelFilter::INFO, None);
    };

    match value.trim().to_ascii_lowercase().parse::<LevelFilter>() {
        Ok(level) => (level, None),
        Err(_) => (LevelFilter::INFO, Some(value)),
    }
}

fn initialize_tracing() {
    TRACING_INIT.call_once(|| {
        let (log_level, invalid_log_level) = server_log_level();
        let filter = Targets::new()
            .with_default(LevelFilter::WARN)
            .with_target("cef_openmp", log_level)
            .with_target("cef_server_core", log_level);
        let format = tracing_subscriber::fmt::layer().compact().with_ansi(false);
        if tracing_subscriber::registry()
            .with(filter)
            .with(format)
            .try_init()
            .is_ok()
        {
            tracing::info!(level = %log_level, "server logging initialized");
            if let Some(value) = invalid_log_level {
                tracing::warn!(
                    value,
                    environment = LOG_LEVEL_ENV,
                    "unknown server log level; using info"
                );
            }
        }
    });
}

#[cxx::bridge(namespace = "samp_cef::openmp")]
mod ffi {
    enum EventKind {
        None = 0,
        EmitEvent = 1,
        PlayerInitialized = 2,
        BrowserCreated = 3,
    }

    struct ServerEvent {
        kind: EventKind,
        player_id: i32,
        browser_id: u32,
        code: i32,
        success: bool,
        event: String,
        arguments: String,
    }

    extern "Rust" {
        type ServerCore;
        type EventArguments;

        fn create_server_core(bind: &str, port: u16) -> Box<ServerCore>;

        fn is_running(self: &ServerCore) -> bool;
        fn last_error(self: &ServerCore) -> String;

        fn allow_connection(self: &mut ServerCore, player_id: i32, ip: &str) -> bool;
        fn remove_connection(self: &mut ServerCore, player_id: i32);

        // Loads the encrypted CEF UI bundle ("ui.bin") + its key so it can
        // be handed out to clients on join. Returns false (and sets
        // last_error()) if the files can't be read/parsed; this is not
        // fatal, the component keeps running and clients fall back to
        // plain files under cef/assets/. Call once after create_server_core.
        fn load_asset_bundle(self: &mut ServerCore, bundle_path: &str, key_path: &str) -> bool;

        fn create_browser(
            self: &mut ServerCore, player_id: i32, browser_id: i32, url: &str, hidden: bool,
            focused: bool,
        );
        fn destroy_browser(self: &mut ServerCore, player_id: i32, browser_id: i32);
        fn hide_browser(self: &ServerCore, player_id: i32, browser_id: i32, hide: bool);
        fn focus_browser(self: &ServerCore, player_id: i32, browser_id: i32, focused: bool);
        fn emit_event(self: &ServerCore, player_id: i32, event: &str, arguments: &EventArguments);
        fn always_listen_keys(self: &ServerCore, player_id: i32, browser_id: i32, listen: bool);
        fn has_plugin(self: &ServerCore, player_id: i32) -> bool;
        fn create_external_browser(
            self: &ServerCore, player_id: i32, browser_id: i32, texture: &str, url: &str,
            scale: i32,
        );
        fn append_to_object(self: &ServerCore, player_id: i32, browser_id: i32, object_id: i32);
        fn remove_from_object(self: &ServerCore, player_id: i32, browser_id: i32, object_id: i32);
        fn toggle_dev_tools(self: &ServerCore, player_id: i32, browser_id: i32, enabled: bool);
        fn set_audio_settings(
            self: &ServerCore, player_id: i32, browser_id: u32, max_distance: f32,
            reference_distance: f32,
        );
        fn load_url(self: &ServerCore, player_id: i32, browser_id: u32, url: &str);
        fn poll_event(self: &mut ServerCore) -> ServerEvent;

        fn new_event_arguments() -> Box<EventArguments>;
        fn push_string(self: &mut EventArguments, value: &str);
        fn push_integer(self: &mut EventArguments, value: i32);
        fn push_float(self: &mut EventArguments, value: f32);
    }
}

pub struct ServerCore {
    inner: Option<InnerServerCore>,
    last_error: String,
}

pub struct EventArguments {
    values: Vec<EventValue<'static>>,
}

fn create_server_core(bind: &str, port: u16) -> Box<ServerCore> {
    initialize_tracing();

    let bind = if bind.is_empty() { "0.0.0.0" } else { bind };
    let ip = match bind.parse::<IpAddr>() {
        Ok(ip) => ip,
        Err(error) => {
            return Box::new(ServerCore {
                inner: None,
                last_error: format!("invalid CEF bind address `{bind}`: {error}"),
            });
        }
    };

    let addr = SocketAddr::from((ip, port));

    match InnerServerCore::new(addr) {
        Ok(inner) => Box::new(ServerCore {
            inner: Some(inner),
            last_error: String::new(),
        }),
        Err(error) => Box::new(ServerCore {
            inner: None,
            last_error: error,
        }),
    }
}

impl ServerCore {
    fn is_running(&self) -> bool {
        self.inner.is_some()
    }

    fn last_error(&self) -> String {
        self.last_error.clone()
    }

    fn allow_connection(&mut self, player_id: i32, ip: &str) -> bool {
        let Some(inner) = self.inner.as_mut() else {
            return false;
        };

        let Ok(ip) = ip.parse::<IpAddr>() else {
            return false;
        };

        inner.allow_connection(player_id, ip);
        true
    }

    fn remove_connection(&mut self, player_id: i32) {
        if let Some(inner) = self.inner.as_mut() {
            inner.remove_connection(player_id);
        }
    }

    fn load_asset_bundle(&mut self, bundle_path: &str, key_path: &str) -> bool {
        let Some(inner) = self.inner.as_mut() else {
            return false;
        };

        match inner.load_asset_bundle(
            std::path::Path::new(bundle_path),
            std::path::Path::new(key_path),
        ) {
            Ok(()) => true,
            Err(error) => {
                self.last_error = error;
                false
            }
        }
    }

    fn create_browser(
        &mut self, player_id: i32, browser_id: i32, url: &str, hidden: bool, focused: bool,
    ) {
        if let Some(inner) = self.inner.as_mut() {
            inner.create_browser(player_id, browser_id, url.to_owned(), hidden, focused);
        }
    }

    fn destroy_browser(&mut self, player_id: i32, browser_id: i32) {
        if let Some(inner) = self.inner.as_mut() {
            inner.destroy_browser(player_id, browser_id);
        }
    }

    fn hide_browser(&self, player_id: i32, browser_id: i32, hide: bool) {
        if let Some(inner) = self.inner.as_ref() {
            inner.hide_browser(player_id, browser_id, hide);
        }
    }

    fn focus_browser(&self, player_id: i32, browser_id: i32, focused: bool) {
        if let Some(inner) = self.inner.as_ref() {
            inner.focus_browser(player_id, browser_id, focused);
        }
    }

    fn emit_event(&self, player_id: i32, event: &str, arguments: &EventArguments) {
        if let Some(inner) = self.inner.as_ref() {
            inner.emit_event(player_id, event, arguments.values.clone());
        }
    }

    fn always_listen_keys(&self, player_id: i32, browser_id: i32, listen: bool) {
        if let Some(inner) = self.inner.as_ref() {
            inner.always_listen_keys(player_id, browser_id, listen);
        }
    }

    fn has_plugin(&self, player_id: i32) -> bool {
        self.inner
            .as_ref()
            .map(|inner| inner.has_plugin(player_id))
            .unwrap_or(false)
    }

    fn create_external_browser(
        &self, player_id: i32, browser_id: i32, texture: &str, url: &str, scale: i32,
    ) {
        if let Some(inner) = self.inner.as_ref() {
            inner.create_external_browser(
                player_id,
                browser_id,
                texture.to_owned(),
                url.to_owned(),
                scale,
            );
        }
    }

    fn append_to_object(&self, player_id: i32, browser_id: i32, object_id: i32) {
        if let Some(inner) = self.inner.as_ref() {
            inner.append_to_object(player_id, browser_id, object_id);
        }
    }

    fn remove_from_object(&self, player_id: i32, browser_id: i32, object_id: i32) {
        if let Some(inner) = self.inner.as_ref() {
            inner.remove_from_object(player_id, browser_id, object_id);
        }
    }

    fn toggle_dev_tools(&self, player_id: i32, browser_id: i32, enabled: bool) {
        if let Some(inner) = self.inner.as_ref() {
            inner.toggle_dev_tools(player_id, browser_id, enabled);
        }
    }

    fn set_audio_settings(
        &self, player_id: i32, browser_id: u32, max_distance: f32, reference_distance: f32,
    ) {
        if let Some(inner) = self.inner.as_ref() {
            inner.set_audio_settings(player_id, browser_id, max_distance, reference_distance);
        }
    }

    fn load_url(&self, player_id: i32, browser_id: u32, url: &str) {
        if let Some(inner) = self.inner.as_ref() {
            inner.load_url(player_id, browser_id, url.to_owned());
        }
    }

    fn poll_event(&mut self) -> ffi::ServerEvent {
        let Some(inner) = self.inner.as_mut() else {
            return empty_event();
        };

        match inner.poll_event() {
            Some(CoreEvent::EmitEvent {
                player_id,
                event,
                arguments,
            }) => ffi::ServerEvent {
                kind: ffi::EventKind::EmitEvent,
                player_id,
                event,
                arguments,
                ..empty_event()
            },

            Some(CoreEvent::PlayerInitialized { player_id, success }) => ffi::ServerEvent {
                kind: ffi::EventKind::PlayerInitialized,
                player_id,
                success,
                ..empty_event()
            },

            Some(CoreEvent::BrowserCreated {
                player_id,
                browser_id,
                code,
            }) => ffi::ServerEvent {
                kind: ffi::EventKind::BrowserCreated,
                player_id,
                browser_id,
                code,
                ..empty_event()
            },

            None => empty_event(),
        }
    }
}

fn new_event_arguments() -> Box<EventArguments> {
    Box::new(EventArguments { values: Vec::new() })
}

impl EventArguments {
    fn push_string(&mut self, value: &str) {
        self.values.push(EventValue {
            string_value: Some(Cow::Owned(value.to_owned())),
            float_value: None,
            integer_value: None,
        });
    }

    fn push_integer(&mut self, value: i32) {
        self.values.push(EventValue {
            string_value: None,
            float_value: None,
            integer_value: Some(value),
        });
    }

    fn push_float(&mut self, value: f32) {
        self.values.push(EventValue {
            string_value: None,
            float_value: Some(value),
            integer_value: None,
        });
    }
}

fn empty_event() -> ffi::ServerEvent {
    ffi::ServerEvent {
        kind: ffi::EventKind::None,
        player_id: 0,
        browser_id: 0,
        code: 0,
        success: false,
        event: String::new(),
        arguments: String::new(),
    }
}
