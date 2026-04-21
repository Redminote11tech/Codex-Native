use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;

#[cfg(not(target_os = "linux"))]
use std::borrow::Cow;

#[cfg(target_os = "linux")]
use gio::prelude::FileExt;
#[cfg(target_os = "linux")]
use glib::Bytes;
#[cfg(target_os = "linux")]
use gtk::{FileChooserAction, FileChooserDialog, ResponseType, Window, WindowType, prelude::*};
#[cfg(target_os = "linux")]
use reqwest::Method;
#[cfg(target_os = "linux")]
use reqwest::blocking::Client;
#[cfg(target_os = "linux")]
use serde_json::{Value as JsonValue, json};

#[cfg(not(target_os = "linux"))]
use tao::dpi::LogicalSize;
#[cfg(not(target_os = "linux"))]
use tao::event::{Event, WindowEvent};
#[cfg(not(target_os = "linux"))]
use tao::event_loop::{ControlFlow, EventLoop};
#[cfg(not(target_os = "linux"))]
use tao::window::WindowBuilder;

#[cfg(target_os = "linux")]
use webkit2gtk::{
    SecurityManagerExt, SettingsExt, URISchemeRequest, URISchemeRequestExt, URISchemeResponse,
    URISchemeResponseExt, UserContentInjectedFrames, UserContentManager, UserContentManagerExt,
    UserScript, UserScriptInjectionTime, WebContext, WebContextExt, WebView, WebViewExt,
    WebViewExtManual,
};

#[cfg(not(target_os = "linux"))]
use wry::WebViewBuilder;
#[cfg(not(target_os = "linux"))]
use wry::http::header::{CONTENT_SECURITY_POLICY, CONTENT_TYPE};
#[cfg(not(target_os = "linux"))]
use wry::http::{Response, StatusCode};

const SHELL_INIT_SCRIPT: &str = r#"
(() => {
  const buildFlavor = "prod";
  const sharedSnapshot = {
    host_config: {
      id: "local",
      display_name: "Local",
      kind: "local",
    },
    remote_connections: [],
    remote_control_connections: [],
    remote_control_connections_state: {
      available: false,
      authRequired: false,
    },
  };
  const themeListeners = new Set();
  const workerListeners = new Map();
  const sentryInitOptions = {
    appVersion: "0.1.0",
    buildFlavor,
    buildNumber: null,
    codexAppSessionId: "native-shell",
    dsn: null,
  };

  function subscribeWorker(workerId, callback) {
    let listeners = workerListeners.get(workerId);
    if (!listeners) {
      listeners = new Set();
      workerListeners.set(workerId, listeners);
    }
    listeners.add(callback);
    return () => {
      listeners.delete(callback);
      if (listeners.size === 0) {
        workerListeners.delete(workerId);
      }
    };
  }

  window.ipc = {
    postMessage(message) {
      if (window.webkit?.messageHandlers?.codexNative) {
        window.webkit.messageHandlers.codexNative.postMessage(String(message));
        return;
      }
      console.log("[native-shell ipc]", message);
    },
  };

  window.codexWindowType = "electron";
  const NativeWorker = window.Worker;
  if (typeof NativeWorker === "function") {
    window.Worker = function NativeShellWorker(url, options) {
      const workerUrl = typeof url === "string" ? url : String(url);
      try {
        window.ipc.postMessage(JSON.stringify({
          kind: "message-from-view",
          payload: {
            type: "native-worker-created",
            url: workerUrl,
            options: options ?? null,
          },
        }));
      } catch {}

      const worker = new NativeWorker(url, options);
      worker.addEventListener("error", (event) => {
        try {
          window.ipc.postMessage(JSON.stringify({
            kind: "message-from-view",
            payload: {
              type: "native-worker-error",
              url: workerUrl,
              message: event?.message ?? null,
              filename: event?.filename ?? null,
              lineno: event?.lineno ?? null,
              colno: event?.colno ?? null,
              error: event?.error ? String(event.error) : null,
            },
          }));
        } catch {}
      });
      return worker;
    };
    window.Worker.prototype = NativeWorker.prototype;
  }
  window.electronBridge = {
    windowType: "electron",
    sendMessageFromView: async (message) => {
      if (message?.type === "shared-object-set") {
        if (message.value === undefined) {
          delete sharedSnapshot[message.key];
        } else {
          sharedSnapshot[message.key] = message.value;
        }
      }
      window.ipc.postMessage(JSON.stringify({ kind: "message-from-view", payload: message }));
    },
    getPathForFile: () => null,
    sendWorkerMessageFromView: async (workerId, message) => {
      window.ipc.postMessage(JSON.stringify({ kind: "worker-message-from-view", workerId, payload: message }));
    },
    subscribeToWorkerMessages: (workerId, callback) => subscribeWorker(workerId, callback),
    showContextMenu: async (payload) => {
      window.ipc.postMessage(JSON.stringify({ kind: "show-context-menu", payload }));
    },
    showApplicationMenu: async (menuId, x, y) => {
      window.ipc.postMessage(JSON.stringify({ kind: "show-application-menu", payload: { menuId, x, y } }));
    },
    getFastModeRolloutMetrics: async () => null,
    getSharedObjectSnapshotValue: (key) => sharedSnapshot[key],
    getSystemThemeVariant: () => "dark",
    subscribeToSystemThemeVariant: (callback) => {
      themeListeners.add(callback);
      return () => themeListeners.delete(callback);
    },
    triggerSentryTestError: async () => {
      throw new Error("Sentry test error requested in native shell shim");
    },
    getSentryInitOptions: () => sentryInitOptions,
    getAppSessionId: () => sentryInitOptions.codexAppSessionId,
    getBuildFlavor: () => buildFlavor,
  };
})();
"#;

#[cfg(target_os = "linux")]
const SCRIPT_MESSAGE_HANDLER: &str = "codexNative";

#[cfg(target_os = "linux")]
struct AssetLoadError {
    status_code: u32,
    reason: &'static str,
    message: String,
}

#[cfg(target_os = "linux")]
type PersistedAtomState = Rc<RefCell<HashMap<String, JsonValue>>>;
#[cfg(target_os = "linux")]
type JsonMapState = Rc<RefCell<HashMap<String, JsonValue>>>;
#[cfg(target_os = "linux")]
type SharedObjectState = Rc<RefCell<HashMap<String, JsonValue>>>;
#[cfg(target_os = "linux")]
const DEFAULT_HOST_ID: &str = "local";
#[cfg(target_os = "linux")]
const WORKSPACE_ROOTS_STATE_KEY: &str = "native-workspace-roots";
#[cfg(target_os = "linux")]
const ACTIVE_WORKSPACE_ROOTS_STATE_KEY: &str = "active-workspace-roots";
#[cfg(target_os = "linux")]
const WORKSPACE_ROOT_OPTIONS_STATE_KEY: &str = "workspace-root-options";
#[cfg(target_os = "linux")]
const WORKSPACE_ROOT_LABELS_STATE_KEY: &str = "workspace-root-labels";
#[cfg(target_os = "linux")]
const MAX_WORKSPACE_ROOTS: usize = 16;
#[cfg(target_os = "linux")]
const EXPERIMENTAL_FEATURE_ENABLEMENT_STATE_KEY: &str = "native-experimental-feature-enablement";
#[cfg(target_os = "linux")]
const STATSIG_DEFAULT_ENABLE_FEATURES_ATOM_KEY: &str = "statsig_default_enable_features";
#[cfg(target_os = "linux")]
const AVATAR_OVERLAY_GATE_KEY: &str = "2679188970";
#[cfg(target_os = "linux")]
const HEARTBEAT_AUTOMATIONS_GATE_KEY: &str = "1488233300";
const HOME_HERO_MASCOT_MARKER: &str = "home.hero.letsBuild";
const HOME_HERO_ICON_SNIPPET: &str =
    "children:(0,$.jsx)(Zh,{className:`h-12 w-12 text-token-foreground/20`})";
const HOME_HERO_MASCOT_SNIPPET: &str = "children:(0,$.jsx)(aee,{assetRef:ree().selectedAvatar?.assetRef,className:`relative z-10 h-20 w-20`,state:`idle`})";
#[cfg(target_os = "linux")]
#[derive(Default)]
struct LocalAuthSnapshot {
    account: Option<JsonValue>,
}
#[cfg(target_os = "linux")]
enum AppServerBridgeEvent {
    Response(JsonValue),
    Notification { method: String, params: JsonValue },
    Request(JsonValue),
    Fatal(String),
}
#[cfg(target_os = "linux")]
struct AppServerBridge {
    child: Arc<Mutex<Child>>,
    writer: Arc<Mutex<ChildStdin>>,
}

#[cfg(target_os = "linux")]
impl AppServerBridge {
    fn send_json(&self, value: &JsonValue) -> Result<(), String> {
        let mut line = value.to_string();
        line.push('\n');
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| "failed to lock app-server stdin".to_string())?;
        writer
            .write_all(line.as_bytes())
            .map_err(|error| format!("failed to write to app-server stdin: {error}"))?;
        writer
            .flush()
            .map_err(|error| format!("failed to flush app-server stdin: {error}"))?;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Drop for AppServerBridge {
    fn drop(&mut self) {
        let Ok(mut child) = self.child.lock() else {
            return;
        };

        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
            }
            Err(_) => {}
        }
    }
}

pub fn run(web_root: PathBuf) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        run_linux_gtk(web_root)
    }

    #[cfg(not(target_os = "linux"))]
    {
        run_wry_shell(web_root)
    }
}

#[cfg(target_os = "linux")]
fn run_linux_gtk(web_root: PathBuf) -> Result<(), String> {
    ensure_web_root(&web_root)?;
    eprintln!(
        "native-shell: gtk startup with web root {}",
        web_root.display()
    );

    let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
        || matches!(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            Some("wayland")
        );
    if is_wayland && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        // Safe here because this runs during process startup before any worker threads are spawned.
        unsafe {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
        eprintln!("native-shell: enabled WEBKIT_DISABLE_DMABUF_RENDERER=1 for Wayland");
    }

    gtk::init().map_err(|error| format!("failed to initialize gtk: {error}"))?;
    eprintln!("native-shell: gtk initialized");

    let window = Window::new(WindowType::Toplevel);
    window.set_title("Codex");
    window.set_default_size(1440, 920);
    let use_client_decorations = std::env::var_os("CODEX_NATIVE_USE_CLIENT_DECORATIONS")
        .map(|value| value != "0")
        .unwrap_or(!is_wayland);
    window.set_decorated(!use_client_decorations);
    eprintln!("native-shell: window created (client_decorations={use_client_decorations})");

    let context = WebContext::default().ok_or("failed to create WebKit web context")?;
    let security_manager = context
        .security_manager()
        .ok_or("failed to access WebKit security manager")?;
    security_manager.register_uri_scheme_as_secure("codex");
    security_manager.register_uri_scheme_as_local("codex");
    security_manager.register_uri_scheme_as_cors_enabled("codex");
    eprintln!("native-shell: webkit context ready");

    let asset_root = web_root.clone();
    context.register_uri_scheme("codex", move |request: &URISchemeRequest| {
        let path = request.path().unwrap_or_else(|| "/index.html".into());
        let relative = if path == "/" {
            "index.html"
        } else {
            path.trim_start_matches('/')
        };

        let (body, mime, status_code, reason) = match load_asset_bytes(&asset_root, relative) {
            Ok((body, mime)) => (body, mime, 200, "OK"),
            Err(error) => (
                error.message.into_bytes(),
                "text/plain; charset=utf-8",
                error.status_code,
                error.reason,
            ),
        };

        let bytes = Bytes::from_owned(body);
        let stream = gio::MemoryInputStream::from_bytes(&bytes);
        let response = URISchemeResponse::new(&stream, bytes.len() as i64);
        response.set_content_type(mime);
        response.set_status(status_code, Some(reason));

        let headers = soup::MessageHeaders::new(soup::MessageHeadersType::Response);
        headers.append("Access-Control-Allow-Origin", "codex://app");
        headers.append("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
        headers.append("Access-Control-Allow-Headers", "*");
        headers.append("Cross-Origin-Resource-Policy", "cross-origin");
        headers.append("Cross-Origin-Embedder-Policy", "unsafe-none");
        response.set_http_headers(headers);

        request.finish_with_response(&response);
    });

    let user_content_manager = UserContentManager::new();
    if !user_content_manager.register_script_message_handler(SCRIPT_MESSAGE_HANDLER) {
        return Err("failed to register WebKit script message handler".to_string());
    }
    let init_script = UserScript::new(
        SHELL_INIT_SCRIPT,
        UserContentInjectedFrames::TopFrame,
        UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    user_content_manager.add_script(&init_script);
    eprintln!("native-shell: initialization script installed");

    let webview =
        WebView::new_with_context_and_user_content_manager(&context, &user_content_manager);
    let persisted_atoms: PersistedAtomState =
        Rc::new(RefCell::new(initial_persisted_atom_state()));
    let global_state: JsonMapState = Rc::new(RefCell::new(HashMap::new()));
    let config_state: JsonMapState = Rc::new(RefCell::new(HashMap::new()));
    let auth_state: JsonMapState = Rc::new(RefCell::new(HashMap::new()));
    let shared_objects: SharedObjectState = Rc::new(RefCell::new(initial_shared_object_state()));
    #[allow(deprecated)]
    let (app_server_events_tx, app_server_events_rx) =
        glib::MainContext::channel::<AppServerBridgeEvent>(glib::Priority::default());
    let app_server = match start_app_server_bridge(app_server_events_tx) {
        Ok(app_server) => Some(Rc::new(app_server)),
        Err(error) => {
            eprintln!("native-shell: failed to start codex app-server: {error}");
            None
        }
    };
    let webview_for_messages = webview.clone();
    let webview_for_app_server_events = webview.clone();
    let window_for_messages = window.clone();
    let persisted_atoms_for_messages = persisted_atoms.clone();
    let global_state_for_messages = global_state.clone();
    let config_state_for_messages = config_state.clone();
    let auth_state_for_messages = auth_state.clone();
    let shared_objects_for_messages = shared_objects.clone();
    let app_server_for_messages = app_server.clone();
    app_server_events_rx.attach(None, move |event| {
        handle_app_server_event(&webview_for_app_server_events, event);
        glib::ControlFlow::Continue
    });
    user_content_manager.connect_script_message_received(
        Some(SCRIPT_MESSAGE_HANDLER),
        move |_manager, result| {
            let payload = result
                .js_value()
                .map(|value| value.to_string().to_string())
                .unwrap_or_else(|| "<unreadable script message>".to_string());

            if let Err(error) = handle_script_message(
                &window_for_messages,
                &webview_for_messages,
                &persisted_atoms_for_messages,
                &global_state_for_messages,
                &config_state_for_messages,
                &auth_state_for_messages,
                &shared_objects_for_messages,
                app_server_for_messages.as_deref(),
                &payload,
            ) {
                eprintln!("native-shell: host bridge error: {error}");
            }
        },
    );
    let settings = WebViewExt::settings(&webview).ok_or("failed to access WebKit settings")?;
    settings.set_enable_developer_extras(true);
    settings.set_enable_write_console_messages_to_stdout(false);
    webview.connect_load_changed(|_, event| {
        eprintln!("native-shell: load event {event:?}");
    });
    webview.connect_load_changed(|view, event| {
        if event != webkit2gtk::LoadEvent::Finished {
            return;
        }

        view.run_javascript(
            "(function(){\
                const root = document.getElementById('root');\
                return JSON.stringify({\
                    href: window.location.href,\
                    title: document.title,\
                    bodyClass: document.body.className,\
                    rootChildCount: root ? root.childElementCount : null,\
                    textSample: (document.body.innerText || '').slice(0, 500)\
                });\
            })()",
            None::<&gio::Cancellable>,
            |result| match result {
                Ok(result) => {
                    let snapshot = result
                        .js_value()
                        .map(|value| value.to_string().to_string())
                        .unwrap_or_else(|| "<no dom snapshot>".to_string());
                    eprintln!("native-shell: dom snapshot {snapshot}");
                }
                Err(error) => {
                    eprintln!("native-shell: dom snapshot failed: {error}");
                }
            },
        );
    });
    webview.connect_load_failed(|_, event, uri, error| {
        eprintln!("native-shell: load failed event={event:?} uri={uri} error={error}");
        false
    });
    eprintln!("native-shell: webview created");

    window.add(&webview);
    let start_mode =
        std::env::var("CODEX_NATIVE_START_MODE").unwrap_or_else(|_| "codex".to_string());
    if let Ok(start_url) = std::env::var("CODEX_NATIVE_START_URL") {
        webview.load_uri(&start_url);
    } else {
        match start_mode.as_str() {
            "simple" => {
                webview.load_html(
                "<!doctype html><html><head><meta charset=\"utf-8\"><title>Codex Native</title><style>html,body{margin:0;background:#111;color:#f5f5f5;font:16px sans-serif}main{min-height:100vh;display:grid;place-items:center}</style></head><body><main><div><h1>Codex Native</h1><p>Wayland/WebKitGTK diagnostic page.</p></div></main></body></html>",
                Some("codex://app/"),
            );
            }
            "file" => {
                let index_uri = gio::File::for_path(web_root.join("index.html")).uri();
                webview.load_uri(index_uri.as_str());
            }
            _ => {
                let start_url = start_local_asset_server(web_root.clone())?;
                eprintln!("native-shell: local asset server ready at {start_url}");
                webview.load_uri(&start_url);
            }
        }
    }
    eprintln!("native-shell: initial page requested (mode={start_mode})");
    window.show_all();
    eprintln!("native-shell: window shown");

    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        glib::Propagation::Proceed
    });

    gtk::main();
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn run_wry_shell(web_root: PathBuf) -> Result<(), String> {
    ensure_web_root(&web_root)?;

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("Codex")
        .with_decorations(false)
        .with_inner_size(LogicalSize::new(1440.0, 920.0))
        .build(&event_loop)
        .map_err(|error| format!("failed to create window: {error}"))?;

    let asset_root = web_root.clone();
    let _webview = WebViewBuilder::new()
        .with_initialization_script(SHELL_INIT_SCRIPT)
        .with_ipc_handler(|request| {
            eprintln!("ipc> {}", request.body());
        })
        .with_custom_protocol("codex".into(), move |_webview_id, request| {
            serve_asset(&asset_root, request.uri().path())
        })
        .with_url("codex://app/index.html")
        .build(&window)
        .map_err(|error| format!("failed to build webview: {error}"))?;

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            ..
        } = event
        {
            *control_flow = ControlFlow::Exit;
        }
    });
}

fn ensure_web_root(web_root: &Path) -> Result<(), String> {
    if !web_root.exists() {
        return Err(format!("web root does not exist: {}", web_root.display()));
    }

    let index_path = web_root.join("index.html");
    if !index_path.is_file() {
        return Err(format!(
            "web root is missing index.html: {}",
            index_path.display()
        ));
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn load_asset_bytes(
    root: &Path,
    request_path: &str,
) -> Result<(Vec<u8>, &'static str), AssetLoadError> {
    let relative = request_path
        .trim_start_matches('/')
        .strip_prefix("app/")
        .unwrap_or_else(|| request_path.trim_start_matches('/'));

    let safe_path = sanitize_relative_path(relative).ok_or_else(|| AssetLoadError {
        status_code: 403,
        reason: "Forbidden",
        message: "forbidden".to_string(),
    })?;
    let file_path = root.join(safe_path);
    let bytes = fs::read(&file_path).map_err(|error| {
        let (status_code, reason) = if error.kind() == std::io::ErrorKind::NotFound {
            (404, "Not Found")
        } else {
            (500, "Internal Server Error")
        };
        AssetLoadError {
            status_code,
            reason,
            message: format!("asset read failed for {}: {error}", file_path.display()),
        }
    })?;
    let bytes = apply_frontend_asset_overrides(&file_path, bytes);

    Ok((bytes, guess_mime(&file_path)))
}

#[cfg(not(target_os = "linux"))]
fn serve_asset(root: &Path, request_path: &str) -> Response<Cow<'static, [u8]>> {
    let relative = request_path
        .trim_start_matches('/')
        .strip_prefix("app/")
        .unwrap_or_else(|| request_path.trim_start_matches('/'));

    let Some(safe_path) = sanitize_relative_path(relative) else {
        return simple_response(
            StatusCode::FORBIDDEN,
            "text/plain; charset=utf-8",
            Cow::Borrowed(b"forbidden"),
        );
    };

    let file_path = root.join(safe_path);
    let bytes = match fs::read(&file_path) {
        Ok(bytes) => apply_frontend_asset_overrides(&file_path, bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return simple_response(
                StatusCode::NOT_FOUND,
                "text/plain; charset=utf-8",
                Cow::Borrowed(b"not found"),
            );
        }
        Err(error) => {
            return simple_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "text/plain; charset=utf-8",
                Cow::Owned(format!("asset read failed: {error}").into_bytes()),
            );
        }
    };

    let mime = guess_mime(&file_path);
    let mut response = Response::builder().status(StatusCode::OK);
    response = response.header(CONTENT_TYPE, mime);
    if file_path.file_name().and_then(|name| name.to_str()) == Some("index.html") {
        response = response.header(CONTENT_SECURITY_POLICY, "default-src 'self' codex: data: blob: https: 'unsafe-inline' 'unsafe-eval'; img-src 'self' codex: data: blob: https:; font-src 'self' codex: data:; media-src 'self' codex: data: blob:; connect-src 'self' codex: https://ab.chatgpt.com https://cdn.openai.com https://chatgpt.com;");
    }
    response.body(Cow::Owned(bytes)).unwrap_or_else(|_| {
        simple_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "text/plain; charset=utf-8",
            Cow::Borrowed(b"response build failed"),
        )
    })
}

#[cfg(not(target_os = "linux"))]
fn simple_response(
    status: StatusCode,
    mime: &'static str,
    body: Cow<'static, [u8]>,
) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, mime)
        .body(body)
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(b"response build failed")))
}

fn apply_frontend_asset_overrides(file_path: &Path, bytes: Vec<u8>) -> Vec<u8> {
    if file_path.extension().and_then(|extension| extension.to_str()) != Some("js") {
        return bytes;
    }

    patch_home_hero_mascot(bytes)
}

fn patch_home_hero_mascot(bytes: Vec<u8>) -> Vec<u8> {
    let Ok(text) = String::from_utf8(bytes.clone()) else {
        return bytes;
    };

    if text.contains(HOME_HERO_MASCOT_SNIPPET) || !text.contains(HOME_HERO_MASCOT_MARKER) {
        return text.into_bytes();
    }

    text.replacen(HOME_HERO_ICON_SNIPPET, HOME_HERO_MASCOT_SNIPPET, 1)
        .into_bytes()
}

fn sanitize_relative_path(path: &str) -> Option<PathBuf> {
    let candidate = Path::new(path);
    let mut sanitized = PathBuf::new();

    for component in candidate.components() {
        match component {
            Component::Normal(value) => sanitized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    Some(sanitized)
}

fn guess_mime(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("gif") => "image/gif",
        Some("html") => "text/html; charset=utf-8",
        Some("ico") => "image/x-icon",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("map") => "application/json; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("txt") => "text/plain; charset=utf-8",
        Some("wav") => "audio/wav",
        Some("webp") => "image/webp",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(target_os = "linux")]
fn start_local_asset_server(root: PathBuf) -> Result<String, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("failed to bind local asset server: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("failed to read local asset server address: {error}"))?;
    let root = Arc::new(root);

    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                continue;
            };
            let root = root.clone();
            thread::spawn(move || {
                if let Err(error) = handle_asset_http_connection(stream, root) {
                    eprintln!("native-shell: local asset server error: {error}");
                }
            });
        }
    });

    Ok(format!("http://127.0.0.1:{}/index.html", address.port()))
}

#[cfg(target_os = "linux")]
fn handle_asset_http_connection(stream: TcpStream, root: Arc<PathBuf>) -> Result<(), String> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|error| format!("failed to read request line: {error}"))?;
    if request_line.trim().is_empty() {
        return Ok(());
    }

    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or("request line missing method")?
        .to_string();
    let target = parts
        .next()
        .ok_or("request line missing target")?
        .to_string();

    loop {
        let mut header_line = String::new();
        reader
            .read_line(&mut header_line)
            .map_err(|error| format!("failed to read request header: {error}"))?;
        if header_line == "\r\n" || header_line.is_empty() {
            break;
        }
    }

    let mut stream = reader.into_inner();
    if method != "GET" && method != "HEAD" {
        write_http_response(
            &mut stream,
            405,
            "Method Not Allowed",
            "text/plain; charset=utf-8",
            if method == "HEAD" {
                &[]
            } else {
                b"method not allowed"
            },
        )?;
        return Ok(());
    }

    let request_path = target
        .split_once('?')
        .map(|(path, _)| path)
        .unwrap_or(target.as_str());
    let relative = if request_path == "/" {
        "index.html"
    } else {
        request_path.trim_start_matches('/')
    };
    let (body, mime, status, reason) = match load_asset_bytes(&root, relative) {
        Ok((body, mime)) => (body, mime, 200, "OK"),
        Err(error) => (
            error.message.into_bytes(),
            "text/plain; charset=utf-8",
            error.status_code,
            error.reason,
        ),
    };
    let body = if method == "HEAD" { Vec::new() } else { body };
    write_http_response(&mut stream, status, reason, mime, &body)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn write_http_response(
    stream: &mut TcpStream,
    status: u32,
    reason: &str,
    mime: &str,
    body: &[u8],
) -> Result<(), String> {
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .map_err(|error| format!("failed to write response headers: {error}"))?;
    stream
        .write_all(body)
        .map_err(|error| format!("failed to write response body: {error}"))?;
    stream
        .flush()
        .map_err(|error| format!("failed to flush response: {error}"))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn handle_script_message(
    window: &Window,
    webview: &WebView,
    persisted_atoms: &PersistedAtomState,
    global_state: &JsonMapState,
    config_state: &JsonMapState,
    auth_state: &JsonMapState,
    shared_objects: &SharedObjectState,
    app_server: Option<&AppServerBridge>,
    payload_text: &str,
) -> Result<(), String> {
    let envelope: JsonValue = serde_json::from_str(payload_text)
        .map_err(|error| format!("invalid script message json: {error}"))?;

    let kind = envelope
        .get("kind")
        .and_then(JsonValue::as_str)
        .ok_or("script message is missing kind")?;

    match kind {
        "message-from-view" => {
            let payload = envelope
                .get("payload")
                .ok_or("message-from-view is missing payload")?;
            handle_view_message(
                window,
                webview,
                persisted_atoms,
                global_state,
                config_state,
                auth_state,
                shared_objects,
                app_server,
                payload,
            )
        }
        _ => Ok(()),
    }
}

#[cfg(target_os = "linux")]
fn handle_view_message(
    window: &Window,
    webview: &WebView,
    persisted_atoms: &PersistedAtomState,
    global_state: &JsonMapState,
    config_state: &JsonMapState,
    auth_state: &JsonMapState,
    shared_objects: &SharedObjectState,
    app_server: Option<&AppServerBridge>,
    payload: &JsonValue,
) -> Result<(), String> {
    let message_type = payload
        .get("type")
        .and_then(JsonValue::as_str)
        .ok_or("view message is missing type")?;

    match message_type {
        "persisted-atom-sync-request" => {
            let state = JsonValue::Object(
                persisted_atoms
                    .borrow()
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            );
            dispatch_message_to_view(
                webview,
                &json!({
                    "type": "persisted-atom-sync",
                    "state": state,
                }),
            );
            Ok(())
        }
        "persisted-atom-update" => {
            let key = payload
                .get("key")
                .and_then(JsonValue::as_str)
                .ok_or("persisted-atom-update is missing key")?
                .to_string();
            let deleted = payload
                .get("deleted")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            let value = payload.get("value").cloned().unwrap_or(JsonValue::Null);

            if deleted {
                persisted_atoms.borrow_mut().remove(&key);
            } else {
                persisted_atoms
                    .borrow_mut()
                    .insert(key.clone(), value.clone());
            }

            dispatch_message_to_view(
                webview,
                &json!({
                    "type": "persisted-atom-updated",
                    "key": key,
                    "value": value,
                    "deleted": deleted,
                }),
            );
            Ok(())
        }
        "shared-object-subscribe" => {
            let key = payload
                .get("key")
                .and_then(JsonValue::as_str)
                .ok_or("shared-object-subscribe is missing key")?;
            dispatch_shared_object_update(webview, shared_objects, key);
            Ok(())
        }
        "shared-object-unsubscribe" => Ok(()),
        "shared-object-set" => {
            let key = payload
                .get("key")
                .and_then(JsonValue::as_str)
                .ok_or("shared-object-set is missing key")?;
            let value = payload.get("value").cloned().unwrap_or(JsonValue::Null);
            shared_objects
                .borrow_mut()
                .insert(key.to_string(), value.clone());
            dispatch_message_to_view(
                webview,
                &json!({
                    "type": "shared-object-updated",
                    "key": key,
                    "value": value,
                }),
            );
            Ok(())
        }
        "log-message" | "ready" | "view-focused" => Ok(()),
        "electron-window-focus-request" => {
            dispatch_message_to_view(
                webview,
                &json!({
                    "type": "electron-window-focus-changed",
                    "isFocused": true,
                }),
            );
            Ok(())
        }
        "electron-onboarding-skip-workspace" => {
            let selected_root = preferred_workspace_root(global_state);

            if let Some(root) = selected_root.as_deref() {
                let state = select_workspace_root(global_state, root);
                dispatch_workspace_root_updates(
                    webview,
                    &state.active_roots,
                    &state.workspace_root_options,
                    &state.labels,
                );
            }

            dispatch_message_to_view(
                webview,
                &json!({
                    "type": "electron-onboarding-skip-workspace-result",
                    "success": selected_root.is_some(),
                    "root": selected_root,
                }),
            );
            Ok(())
        }
        "electron-add-new-workspace-root-option" => {
            let selected_root = payload
                .get("root")
                .and_then(JsonValue::as_str)
                .and_then(normalize_workspace_root)
                .or_else(|| prompt_for_workspace_root(window, global_state));

            let Some(root) = selected_root else {
                return Ok(());
            };

            let state = select_workspace_root(global_state, &root);
            dispatch_workspace_root_updates(
                webview,
                &state.active_roots,
                &state.workspace_root_options,
                &state.labels,
            );
            dispatch_workspace_root_option_added(webview, &root);
            Ok(())
        }
        "workspace-root-option-picked" => {
            let root = payload
                .get("root")
                .and_then(JsonValue::as_str)
                .ok_or("workspace-root-option-picked is missing root")?;
            let selected_root = normalize_workspace_root(root)
                .ok_or("workspace-root-option-picked requires an existing absolute path")?;
            let state = select_workspace_root(global_state, &selected_root);
            dispatch_workspace_root_updates(
                webview,
                &state.active_roots,
                &state.workspace_root_options,
                &state.labels,
            );
            dispatch_workspace_root_option_added(webview, &selected_root);
            Ok(())
        }
        "electron-set-active-workspace-root" => {
            let root = payload
                .get("root")
                .and_then(JsonValue::as_str)
                .ok_or("electron-set-active-workspace-root is missing root")?;
            let selected_root = normalize_workspace_root(root)
                .ok_or("electron-set-active-workspace-root requires an existing absolute path")?;
            let state = select_workspace_root(global_state, &selected_root);
            dispatch_workspace_root_updates(
                webview,
                &state.active_roots,
                &state.workspace_root_options,
                &state.labels,
            );
            Ok(())
        }
        "electron-clear-active-workspace-root" => {
            let state = clear_active_workspace_roots(global_state);
            dispatch_workspace_root_updates(
                webview,
                &state.active_roots,
                &state.workspace_root_options,
                &state.labels,
            );
            Ok(())
        }
        "electron-update-workspace-root-options" => {
            let roots = payload
                .get("roots")
                .and_then(JsonValue::as_array)
                .ok_or("electron-update-workspace-root-options is missing roots")?;
            let normalized_roots: Vec<&str> = roots.iter().filter_map(JsonValue::as_str).collect();
            let state = replace_workspace_root_options(global_state, &normalized_roots);
            dispatch_workspace_root_updates(
                webview,
                &state.active_roots,
                &state.workspace_root_options,
                &state.labels,
            );
            Ok(())
        }
        "electron-rename-workspace-root-option" => {
            let root = payload
                .get("root")
                .and_then(JsonValue::as_str)
                .ok_or("electron-rename-workspace-root-option is missing root")?;
            let label = payload
                .get("label")
                .and_then(JsonValue::as_str)
                .unwrap_or_default();
            let normalized_root = normalize_workspace_root(root).ok_or(
                "electron-rename-workspace-root-option requires an existing absolute path",
            )?;
            let state = rename_workspace_root_option(global_state, &normalized_root, label);
            dispatch_workspace_root_updates(
                webview,
                &state.active_roots,
                &state.workspace_root_options,
                &state.labels,
            );
            Ok(())
        }
        "fetch" => handle_fetch_request(
            webview,
            persisted_atoms,
            global_state,
            config_state,
            auth_state,
            payload,
        ),
        "mcp-request" => handle_mcp_request(
            webview,
            global_state,
            config_state,
            auth_state,
            app_server,
            payload,
        ),
        "mcp-response" => handle_mcp_response(app_server, payload),
        "thread-prewarm-start" => handle_thread_prewarm_start(webview, payload),
        _ => Ok(()),
    }
}

#[cfg(target_os = "linux")]
fn handle_fetch_request(
    webview: &WebView,
    persisted_atoms: &PersistedAtomState,
    global_state: &JsonMapState,
    config_state: &JsonMapState,
    auth_state: &JsonMapState,
    payload: &JsonValue,
) -> Result<(), String> {
    let request_id = payload
        .get("requestId")
        .and_then(JsonValue::as_str)
        .ok_or("fetch request is missing requestId")?;
    let url = payload
        .get("url")
        .and_then(JsonValue::as_str)
        .ok_or("fetch request is missing url")?;
    let method = payload
        .get("method")
        .and_then(JsonValue::as_str)
        .unwrap_or("GET");
    let body = payload.get("body").and_then(JsonValue::as_str);
    let headers = parse_fetch_headers(payload);

    match url {
        url if is_statsig_initialize_url(url) => {
            match forward_statsig_initialize_request(url, method, &headers, body) {
                Ok(body) => dispatch_fetch_success(webview, request_id, body),
                Err(error) => {
                    eprintln!("native-shell: statsig initialize failed, using fallback: {error}");
                    dispatch_fetch_success(
                        webview,
                        request_id,
                        default_statsig_initialize_response(),
                    );
                }
            }
            Ok(())
        }
        url if is_statsig_events_url(url) || is_statsig_exception_url(url) => {
            dispatch_fetch_success(webview, request_id, json!({ "success": true }));
            Ok(())
        }
        "/wham/accounts/check" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "account_ordering": [],
                    "accounts": {},
                }),
            );
            Ok(())
        }
        url if url.starts_with("/accounts/check/") => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "account_ordering": [],
                    "accounts": {},
                }),
            );
            Ok(())
        }
        "/wham/usage" => {
            let snapshot = read_effective_auth_snapshot(auth_state);
            let plan_type = snapshot
                .account
                .as_ref()
                .and_then(|account| account.get("planType"))
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown");
            let unlimited_credits = matches!(
                plan_type,
                "plus"
                    | "pro"
                    | "team"
                    | "business"
                    | "enterprise"
                    | "enterprise_cbp_usage_based"
                    | "edu"
                    | "self_serve_business_usage_based"
            );
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "rate_limit": {
                        "allowed": true,
                        "limit_reached": false,
                        "primary_window": JsonValue::Null,
                        "secondary_window": JsonValue::Null,
                        "rate_limit_name": JsonValue::Null,
                    },
                    "additional_rate_limits": [],
                    "credits": {
                        "has_credits": unlimited_credits,
                        "unlimited": unlimited_credits,
                        "balance": 0,
                    },
                    "plan_type": plan_type,
                    "spend_control": {
                        "reached": false,
                    },
                }),
            );
            Ok(())
        }
        "/subscriptions/auto_top_up/settings" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "is_enabled": false,
                    "recharge_threshold": JsonValue::Null,
                    "recharge_target": JsonValue::Null,
                }),
            );
            Ok(())
        }
        "/subscriptions/auto_top_up/enable" => {
            let request = body
                .and_then(parse_json_body)
                .unwrap_or(JsonValue::Null);
            let recharge_threshold = request
                .get("recharge_threshold")
                .cloned()
                .unwrap_or(JsonValue::Null);
            let recharge_target = request
                .get("recharge_target")
                .cloned()
                .unwrap_or(JsonValue::Null);
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "is_enabled": true,
                    "recharge_threshold": recharge_threshold,
                    "recharge_target": recharge_target,
                    "immediate_top_up_status": "succeeded",
                }),
            );
            Ok(())
        }
        "/subscriptions/auto_top_up/update" => {
            let request = body
                .and_then(parse_json_body)
                .unwrap_or(JsonValue::Null);
            let recharge_threshold = request
                .get("recharge_threshold")
                .cloned()
                .unwrap_or(JsonValue::Null);
            let recharge_target = request
                .get("recharge_target")
                .cloned()
                .unwrap_or(JsonValue::Null);
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "is_enabled": true,
                    "recharge_threshold": recharge_threshold,
                    "recharge_target": recharge_target,
                    "immediate_top_up_status": "succeeded",
                }),
            );
            Ok(())
        }
        "/subscriptions/auto_top_up/disable" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "is_enabled": false,
                    "recharge_threshold": JsonValue::Null,
                    "recharge_target": JsonValue::Null,
                    "immediate_top_up_status": "disabled",
                }),
            );
            Ok(())
        }
        "/accounts/send_add_credits_nudge_email" => {
            dispatch_fetch_success(webview, request_id, json!({ "ok": true }));
            Ok(())
        }
        url if url.starts_with("/checkout_pricing_config/configs/") => {
            let currency_code = url
                .rsplit('/')
                .next()
                .filter(|value| !value.is_empty())
                .unwrap_or("USD");
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "currency_config": {
                        "symbol_code": currency_code,
                        "minor_unit_exponent": 2,
                        "amount_per_credit": JsonValue::Null,
                    },
                }),
            );
            Ok(())
        }
        url if url.starts_with("/wham/tasks/list") => {
            dispatch_fetch_success(webview, request_id, json!({ "items": [] }));
            Ok(())
        }
        "vscode://codex/get-global-state" => {
            let key = body
                .and_then(parse_json_body)
                .and_then(|json| {
                    json.get("key")
                        .and_then(JsonValue::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            let value = read_store_value_or_default(global_state, &key, default_global_state_value);
            dispatch_fetch_success(webview, request_id, json!({ "value": value }));
            Ok(())
        }
        "vscode://codex/get-configuration" => {
            let key = body
                .and_then(parse_json_body)
                .and_then(|json| {
                    json.get("key")
                        .and_then(JsonValue::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_default();
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "value": read_store_value_or_default(
                        config_state,
                        &key,
                        default_configuration_value,
                    ),
                }),
            );
            Ok(())
        }
        "vscode://codex/set-global-state" => {
            let json = body
                .and_then(parse_json_body)
                .ok_or("set-global-state is missing json body")?;
            let key = json
                .get("key")
                .and_then(JsonValue::as_str)
                .ok_or("set-global-state is missing key")?;
            let value = json.get("value").cloned().unwrap_or(JsonValue::Null);
            global_state
                .borrow_mut()
                .insert(key.to_string(), value.clone());
            dispatch_fetch_success(webview, request_id, json!({ "ok": true, "value": value }));
            Ok(())
        }
        "vscode://codex/set-configuration" => {
            let json = body
                .and_then(parse_json_body)
                .ok_or("set-configuration is missing json body")?;
            let key = json
                .get("key")
                .and_then(JsonValue::as_str)
                .ok_or("set-configuration is missing key")?;
            let value = json.get("value").cloned().unwrap_or(JsonValue::Null);
            config_state
                .borrow_mut()
                .insert(key.to_string(), value.clone());
            dispatch_fetch_success(webview, request_id, json!({ "ok": true, "value": value }));
            Ok(())
        }
        "vscode://codex/ipc-request" => {
            let response = body
                .and_then(parse_json_body)
                .map(|request| handle_native_ipc_request(webview, persisted_atoms, global_state, &request))
                .unwrap_or_else(|| ipc_error_response("ipc-request is missing json body"));
            dispatch_fetch_success(webview, request_id, response);
            Ok(())
        }
        "vscode://codex/list-pinned-threads" => {
            dispatch_fetch_success(webview, request_id, json!([]));
            Ok(())
        }
        "vscode://codex/list-automations" => {
            dispatch_fetch_success(webview, request_id, json!([]));
            Ok(())
        }
        "vscode://codex/list-pending-automation-run-threads" => {
            dispatch_fetch_success(webview, request_id, json!([]));
            Ok(())
        }
        "vscode://codex/recommended-skills" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "skills": [],
                    "repoRoot": codex_home_dir().join("skills"),
                }),
            );
            Ok(())
        }
        "vscode://codex/email-domain-mail-provider" => {
            let provider = body
                .and_then(parse_json_body)
                .and_then(|request| {
                    request
                        .get("domain")
                        .and_then(JsonValue::as_str)
                        .map(infer_mail_provider)
                })
                .unwrap_or("other");
            dispatch_fetch_success(webview, request_id, json!({ "provider": provider }));
            Ok(())
        }
        "vscode://codex/fast-mode-rollout-metrics" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "estimatedSavedMs": 0,
                    "rolloutCountWithCompletedTurns": 0,
                }),
            );
            Ok(())
        }
        url if url.starts_with("vscode://codex/codex-agents-md") => {
            let should_save = method != "GET" || url.ends_with("-save");
            let response = if should_save {
                save_codex_agents_md(body)
            } else {
                load_codex_agents_md()
            };
            match response {
                Ok(value) => dispatch_fetch_success(webview, request_id, value),
                Err(error) => dispatch_fetch_error(webview, request_id, 500, error),
            }
            Ok(())
        }
        "vscode://codex/active-workspace-roots" => {
            let roots = current_active_workspace_roots(global_state);
            dispatch_fetch_success(webview, request_id, json!({ "roots": roots }));
            Ok(())
        }
        "vscode://codex/workspace-root-options" => {
            let roots = current_workspace_root_options(global_state);
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "roots": roots,
                    "labels": current_workspace_root_labels(global_state, &roots),
                }),
            );
            Ok(())
        }
        "vscode://codex/paths-exist" => {
            let existing_paths = body
                .and_then(parse_json_body)
                .and_then(|json| {
                    json.get("paths")
                        .and_then(JsonValue::as_array)
                        .map(|paths| {
                            paths
                                .iter()
                                .filter_map(JsonValue::as_str)
                                .filter(|path| Path::new(path).exists())
                                .map(str::to_owned)
                                .collect::<Vec<_>>()
                        })
                })
                .unwrap_or_default();
            dispatch_fetch_success(
                webview,
                request_id,
                json!({ "existingPaths": existing_paths }),
            );
            Ok(())
        }
        "vscode://codex/read-file-binary" => {
            let request = body
                .and_then(parse_json_body)
                .ok_or("read-file-binary is missing json body")?;
            let path = request
                .get("path")
                .and_then(JsonValue::as_str)
                .ok_or("read-file-binary is missing path")?;
            match read_local_file_binary(path) {
                Ok((contents_base64, mime_type)) => {
                    dispatch_fetch_success(
                        webview,
                        request_id,
                        json!({
                            "contentsBase64": contents_base64,
                            "mimeType": mime_type,
                        }),
                    );
                }
                Err((status_code, message)) => {
                    dispatch_fetch_error(webview, request_id, status_code, message);
                }
            }
            Ok(())
        }
        "vscode://codex/workspace-directory-entries" => {
            let entries = body
                .and_then(parse_json_body)
                .map(|request| read_workspace_directory_entries(global_state, request))
                .unwrap_or_default();
            dispatch_fetch_success(webview, request_id, json!({ "entries": entries }));
            Ok(())
        }
        "vscode://codex/git-origins" => {
            dispatch_fetch_success(webview, request_id, json!({ "origins": [] }));
            Ok(())
        }
        "vscode://codex/codex-home" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({ "path": std::env::var("HOME").unwrap_or_else(|_| "/home/jade".to_string()) + "/.codex" }),
            );
            Ok(())
        }
        "vscode://codex/os-info" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "platform": "linux",
                    "arch": std::env::consts::ARCH,
                    "hasWsl": false,
                }),
            );
            Ok(())
        }
        "vscode://codex/locale-info" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "locale": "en-US",
                    "osLocale": "en-US",
                }),
            );
            Ok(())
        }
        "vscode://codex/inbox-items" => {
            dispatch_fetch_success(webview, request_id, json!({ "items": [] }));
            Ok(())
        }
        "vscode://codex/ide-context" => {
            dispatch_fetch_success(webview, request_id, json!({ "ideContext": null }));
            Ok(())
        }
        "vscode://codex/mcp-codex-config" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "config": build_mcp_config(config_state),
                }),
            );
            Ok(())
        }
        "vscode://codex/worktree-shell-environment-config" => {
            dispatch_fetch_success(webview, request_id, json!({}));
            Ok(())
        }
        "vscode://codex/developer-instructions" => {
            let instructions = body
                .and_then(parse_json_body)
                .and_then(|request| request.get("baseInstructions").cloned())
                .unwrap_or(JsonValue::Null);
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "instructions": instructions,
                }),
            );
            Ok(())
        }
        "vscode://codex/ambient-suggestions" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "file": {
                        "generatedAtMs": null,
                        "currentSuggestionIds": [],
                        "suggestions": [],
                    },
                    "items": [],
                }),
            );
            Ok(())
        }
        "vscode://codex/ambient-suggestions-generation-statuses" => {
            dispatch_fetch_success(webview, request_id, json!({ "statuses": [] }));
            Ok(())
        }
        "vscode://codex/local-environments" => {
            dispatch_fetch_success(webview, request_id, json!({ "environments": [] }));
            Ok(())
        }
        "vscode://codex/set-remote-control-connections-enabled" => {
            dispatch_fetch_success(webview, request_id, json!({ "ok": true }));
            Ok(())
        }
        "vscode://codex/open-in-targets" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "preferredTarget": null,
                    "targets": [],
                }),
            );
            Ok(())
        }
        "vscode://codex/extension-info" => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "appName": "Codex",
                    "platform": "linux",
                    "windowType": "electron",
                }),
            );
            Ok(())
        }
        "vscode://codex/is-copilot-api-available" => {
            dispatch_fetch_success(webview, request_id, json!({ "available": false }));
            Ok(())
        }
        "vscode://codex/get-copilot-api-proxy-info" => {
            dispatch_fetch_success(webview, request_id, JsonValue::Null);
            Ok(())
        }
        "vscode://codex/account-info" => {
            let snapshot = read_effective_auth_snapshot(auth_state);
            let plan = snapshot
                .account
                .as_ref()
                .and_then(|account| account.get("planType"))
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown");
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "plan": plan,
                }),
            );
            Ok(())
        }
        url if parse_aip_connector_metadata_request(url).is_some() => {
            let connector_id = parse_aip_connector_metadata_request(url)
                .ok_or("connector metadata request is missing connector id")?;
            dispatch_fetch_success(
                webview,
                request_id,
                connector_stub_metadata_response(connector_id),
            );
            Ok(())
        }
        url if parse_aip_connector_logo_request(url).is_some() => {
            let (connector_id, theme) = parse_aip_connector_logo_request(url)
                .ok_or("connector logo request is missing connector id")?;
            dispatch_fetch_success(
                webview,
                request_id,
                connector_stub_logo_response(connector_id, theme),
            );
            Ok(())
        }
        url if parse_aip_connector_tos_request(url).is_some() => {
            dispatch_fetch_success(
                webview,
                request_id,
                json!({
                    "bodyMarkdown": JsonValue::Null,
                    "title": "Terms of service",
                    "url": JsonValue::Null,
                }),
            );
            Ok(())
        }
        _ => {
            eprintln!("native-shell: unsupported native fetch url: {url}");
            dispatch_fetch_error(
                webview,
                request_id,
                404,
                format!("unsupported native fetch url: {url}"),
            );
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
fn handle_native_ipc_request(
    webview: &WebView,
    persisted_atoms: &PersistedAtomState,
    global_state: &JsonMapState,
    request: &JsonValue,
) -> JsonValue {
    let Some(method) = request.get("method").and_then(JsonValue::as_str) else {
        return ipc_error_response("ipc-request is missing method");
    };
    let params = request.get("params").cloned().unwrap_or(JsonValue::Null);

    match method {
        "set-default-feature-overrides" => {
            let overrides = params.get("overrides").cloned().unwrap_or(JsonValue::Null);
            let deleted = overrides.is_null();
            if deleted {
                persisted_atoms
                    .borrow_mut()
                    .remove(STATSIG_DEFAULT_ENABLE_FEATURES_ATOM_KEY);
            } else {
                persisted_atoms.borrow_mut().insert(
                    STATSIG_DEFAULT_ENABLE_FEATURES_ATOM_KEY.to_string(),
                    overrides.clone(),
                );
            }
            dispatch_message_to_view(
                webview,
                &json!({
                    "type": "persisted-atom-updated",
                    "key": STATSIG_DEFAULT_ENABLE_FEATURES_ATOM_KEY,
                    "value": overrides,
                    "deleted": deleted,
                }),
            );
            ipc_success_response(json!({}))
        }
        "set-experimental-feature-enablement-for-host" => {
            let host_id = params
                .get("hostId")
                .and_then(JsonValue::as_str)
                .unwrap_or(DEFAULT_HOST_ID);
            let enablement = params
                .get("enablement")
                .cloned()
                .unwrap_or_else(|| json!({}));
            write_experimental_feature_enablement(global_state, host_id, enablement);
            ipc_success_response(json!({}))
        }
        "active-workspace-roots" => ipc_success_response(json!({
            "roots": current_active_workspace_roots(global_state),
        })),
        "workspace-root-options" => {
            let roots = current_workspace_root_options(global_state);
            ipc_success_response(json!({
                "roots": roots,
                "labels": current_workspace_root_labels(global_state, &roots),
            }))
        }
        "remote-workspace-directory-entries" => {
            let directories_only = params
                .get("directoriesOnly")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false);
            let mut entries = read_workspace_directory_entries(global_state, params.clone());
            if directories_only {
                entries.retain(|entry| {
                    entry.get("type").and_then(JsonValue::as_str) == Some("directory")
                });
            }
            ipc_success_response(json!(entries))
        }
        "set-remote-control-connections-enabled"
        | "open-file"
        | "import-external-agent-config"
        | "thread-follower-set-collaboration-mode" => ipc_success_response(json!({})),
        "open-in-targets" => ipc_success_response(json!([])),
        "detect-external-agent-config" => ipc_success_response(JsonValue::Null),
        _ => {
            eprintln!(
                "native-shell: unsupported ipc request method: {} params={}",
                method, params
            );
            ipc_error_response(format!("unsupported ipc request method: {method}"))
        }
    }
}

#[cfg(target_os = "linux")]
fn ipc_success_response(result: JsonValue) -> JsonValue {
    json!({
        "requestId": "",
        "type": "response",
        "resultType": "success",
        "result": result,
    })
}

#[cfg(target_os = "linux")]
fn ipc_error_response(error: impl Into<String>) -> JsonValue {
    json!({
        "requestId": "",
        "type": "response",
        "resultType": "error",
        "error": error.into(),
    })
}

#[cfg(target_os = "linux")]
fn handle_mcp_request(
    webview: &WebView,
    global_state: &JsonMapState,
    config_state: &JsonMapState,
    auth_state: &JsonMapState,
    app_server: Option<&AppServerBridge>,
    payload: &JsonValue,
) -> Result<(), String> {
    let host_id = payload
        .get("hostId")
        .and_then(JsonValue::as_str)
        .unwrap_or("local");
    let request = payload
        .get("request")
        .ok_or("mcp-request is missing request")?;
    let request_id = request
        .get("id")
        .and_then(JsonValue::as_str)
        .ok_or("mcp-request is missing request.id")?;
    let method = request
        .get("method")
        .and_then(JsonValue::as_str)
        .ok_or("mcp-request is missing request.method")?;

    if let Some(app_server) = app_server {
        if !is_locally_handled_mcp_method(method) {
            let request = rewrite_app_server_request(global_state, request);
            return app_server.send_json(&request);
        }
    }

    match method {
        "thread/list" => {
            dispatch_mcp_success(
                webview,
                host_id,
                request_id,
                json!({
                    "data": [],
                    "nextCursor": null,
                }),
            );
            Ok(())
        }
        "account/read" => {
            let snapshot = read_effective_auth_snapshot(auth_state);
            dispatch_mcp_success(
                webview,
                host_id,
                request_id,
                json!({
                    "account": snapshot.account,
                    "requiresOpenaiAuth": snapshot.account.is_none(),
                }),
            );
            Ok(())
        }
        "account/login/start" => {
            let params = request.get("params").cloned().unwrap_or(JsonValue::Null);
            let login_type = params
                .get("type")
                .and_then(JsonValue::as_str)
                .ok_or("account/login/start is missing type")?;

            match login_type {
                "apiKey" => {
                    auth_state
                        .borrow_mut()
                        .insert("authMode".to_string(), json!("apikey"));
                    auth_state.borrow_mut().insert(
                        "account".to_string(),
                        json!({
                            "type": "apiKey",
                        }),
                    );
                    dispatch_mcp_success(
                        webview,
                        host_id,
                        request_id,
                        json!({
                            "type": "apiKey",
                        }),
                    );
                    Ok(())
                }
                "chatgptAuthTokens" => {
                    let access_token = params
                        .get("accessToken")
                        .and_then(JsonValue::as_str)
                        .ok_or("chatgptAuthTokens login is missing accessToken")?;
                    let claims = decode_jwt_claims(access_token)
                        .ok_or("failed to decode chatgpt access token claims")?;
                    let email = extract_chatgpt_email(&claims)
                        .ok_or("chatgptAuthTokens login is missing email claims")?;
                    let plan_type = normalize_plan_type(
                        params
                            .get("chatgptPlanType")
                            .and_then(JsonValue::as_str)
                            .or_else(|| extract_chatgpt_plan_type(&claims)),
                    );
                    let account = json!({
                        "type": "chatgpt",
                        "email": email,
                        "planType": plan_type,
                    });
                    auth_state
                        .borrow_mut()
                        .insert("authMode".to_string(), json!("chatgptAuthTokens"));
                    auth_state
                        .borrow_mut()
                        .insert("account".to_string(), account);
                    dispatch_mcp_success(
                        webview,
                        host_id,
                        request_id,
                        json!({
                            "type": "chatgptAuthTokens",
                        }),
                    );
                    Ok(())
                }
                "chatgpt" | "chatgptDeviceCode" => {
                    dispatch_mcp_error(
                        webview,
                        host_id,
                        request_id,
                        format!(
                            "native login flow for {login_type} is not implemented yet; reuse ~/.codex/auth.json or supply chatgptAuthTokens"
                        ),
                    );
                    Ok(())
                }
                _ => {
                    dispatch_mcp_error(
                        webview,
                        host_id,
                        request_id,
                        format!("unsupported account/login/start type: {login_type}"),
                    );
                    Ok(())
                }
            }
        }
        "account/login/cancel" => {
            dispatch_mcp_success(
                webview,
                host_id,
                request_id,
                json!({
                    "status": "notFound",
                }),
            );
            Ok(())
        }
        "account/logout" => {
            auth_state.borrow_mut().clear();
            dispatch_mcp_success(webview, host_id, request_id, json!({}));
            Ok(())
        }
        "config/read" => {
            dispatch_mcp_success(
                webview,
                host_id,
                request_id,
                json!({
                    "config": build_mcp_config(config_state),
                }),
            );
            Ok(())
        }
        "config/value/write" => {
            let params = request.get("params").cloned().unwrap_or(JsonValue::Null);
            apply_config_write(config_state, &params)?;
            dispatch_mcp_success(webview, host_id, request_id, json!({}));
            Ok(())
        }
        "config/batchWrite" => {
            let params = request.get("params").cloned().unwrap_or(JsonValue::Null);
            apply_config_batch_write(config_state, &params)?;
            dispatch_mcp_success(webview, host_id, request_id, json!({}));
            Ok(())
        }
        "configRequirements/read" => {
            dispatch_mcp_success(
                webview,
                host_id,
                request_id,
                json!({
                    "requirements": [],
                }),
            );
            Ok(())
        }
        "collaborationMode/list" => {
            dispatch_mcp_success(
                webview,
                host_id,
                request_id,
                json!({
                    "data": [],
                }),
            );
            Ok(())
        }
        "model/list" => {
            dispatch_mcp_success(
                webview,
                host_id,
                request_id,
                json!({
                    "data": native_model_catalog(),
                    "nextCursor": null,
                }),
            );
            Ok(())
        }
        "experimentalFeature/list" => {
            let data = experimental_feature_list(global_state, host_id);
            dispatch_mcp_success(
                webview,
                host_id,
                request_id,
                json!({
                    "data": data,
                    "nextCursor": null,
                }),
            );
            Ok(())
        }
        "experimentalFeature/enablement/set" => {
            let params = request.get("params").cloned().unwrap_or(JsonValue::Null);
            let enablement = params
                .get("enablement")
                .cloned()
                .unwrap_or_else(|| json!({}));
            write_experimental_feature_enablement(global_state, host_id, enablement);
            dispatch_mcp_success(webview, host_id, request_id, json!({}));
            Ok(())
        }
        "mcpServerStatus/list" => {
            dispatch_mcp_success(
                webview,
                host_id,
                request_id,
                json!({
                    "data": [],
                    "nextCursor": null,
                }),
            );
            Ok(())
        }
        _ => {
            if let Some(app_server) = app_server {
                app_server.send_json(request)
            } else {
                dispatch_mcp_error(
                    webview,
                    host_id,
                    request_id,
                    format!("unsupported native mcp method: {method}"),
                );
                Ok(())
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn handle_mcp_response(
    app_server: Option<&AppServerBridge>,
    payload: &JsonValue,
) -> Result<(), String> {
    let response = payload
        .get("response")
        .ok_or("mcp-response is missing response")?;
    if let Some(app_server) = app_server {
        app_server.send_json(response)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn handle_thread_prewarm_start(webview: &WebView, payload: &JsonValue) -> Result<(), String> {
    let host_id = payload
        .get("hostId")
        .and_then(JsonValue::as_str)
        .unwrap_or(DEFAULT_HOST_ID);
    let request_id = payload
        .get("request")
        .and_then(|request| request.get("id"))
        .and_then(JsonValue::as_str)
        .ok_or("thread-prewarm-start is missing request.id")?;

    dispatch_message_to_view(
        webview,
        &json!({
            "type": "mcp-response",
            "hostId": host_id,
            "message": {
                "id": request_id,
                "error": {
                    "code": -32001,
                    "message": "thread prewarming is disabled in Codex Native to avoid stale workspace reuse",
                },
            },
        }),
    );

    Ok(())
}

#[cfg(target_os = "linux")]
fn dispatch_message_to_view(webview: &WebView, payload: &JsonValue) {
    let script = format!(
        "window.dispatchEvent(new MessageEvent('message', {{ data: {} }}));",
        payload
    );
    webview.run_javascript(&script, None::<&gio::Cancellable>, |result| {
        if let Err(error) = result {
            eprintln!("native-shell: dispatch to view failed: {error}");
        }
    });
}

#[cfg(target_os = "linux")]
fn dispatch_shared_object_update(webview: &WebView, shared_objects: &SharedObjectState, key: &str) {
    let value = shared_objects
        .borrow()
        .get(key)
        .cloned()
        .unwrap_or(JsonValue::Null);
    dispatch_message_to_view(
        webview,
        &json!({
            "type": "shared-object-updated",
            "key": key,
            "value": value,
        }),
    );
}

#[cfg(target_os = "linux")]
fn workspace_query_invalidation_payloads() -> [JsonValue; 2] {
    [
        json!({
            "type": "query-cache-invalidate",
            "queryKey": ["vscode", "active-workspace-roots"],
        }),
        json!({
            "type": "query-cache-invalidate",
            "queryKey": ["vscode", "workspace-root-options"],
        }),
    ]
}

#[cfg(target_os = "linux")]
fn dispatch_workspace_root_updates(
    webview: &WebView,
    active_roots: &[String],
    roots: &[String],
    labels: &JsonValue,
) {
    for message_type in [
        "active-workspace-roots-changed",
        "active-workspace-roots-updated",
    ] {
        dispatch_message_to_view(
            webview,
            &json!({
                "type": message_type,
                "roots": active_roots,
            }),
        );
    }

    for message_type in [
        "workspace-root-options-changed",
        "workspace-root-options-updated",
    ] {
        dispatch_message_to_view(
            webview,
            &json!({
                "type": message_type,
                "roots": roots,
                "labels": labels,
            }),
        );
    }

    for payload in workspace_query_invalidation_payloads() {
        dispatch_message_to_view(webview, &payload);
    }
}

#[cfg(target_os = "linux")]
fn dispatch_workspace_root_option_added(webview: &WebView, root: &str) {
    dispatch_message_to_view(
        webview,
        &json!({
            "type": "workspace-root-option-added",
            "root": root,
        }),
    );
}

#[cfg(target_os = "linux")]
fn dispatch_fetch_success(webview: &WebView, request_id: &str, body: JsonValue) {
    dispatch_message_to_view(
        webview,
        &json!({
            "type": "fetch-response",
            "requestId": request_id,
            "responseType": "success",
            "status": 200,
            "headers": {
                "content-type": "application/json; charset=utf-8",
            },
            "bodyJsonString": body.to_string(),
        }),
    );
}

#[cfg(target_os = "linux")]
fn dispatch_fetch_error(webview: &WebView, request_id: &str, status: u16, error: String) {
    dispatch_message_to_view(
        webview,
        &json!({
            "type": "fetch-response",
            "requestId": request_id,
            "responseType": "error",
            "status": status,
            "error": error,
        }),
    );
}

#[cfg(target_os = "linux")]
fn dispatch_mcp_success(webview: &WebView, host_id: &str, request_id: &str, value: JsonValue) {
    dispatch_message_to_view(
        webview,
        &json!({
            "type": "mcp-response",
            "hostId": host_id,
            "message": {
                "id": request_id,
                "result": value,
            },
        }),
    );
}

#[cfg(target_os = "linux")]
fn dispatch_mcp_error(webview: &WebView, host_id: &str, request_id: &str, message: String) {
    dispatch_message_to_view(
        webview,
        &json!({
            "type": "mcp-response",
            "hostId": host_id,
            "message": {
                "id": request_id,
                "error": {
                    "message": message,
                },
            },
        }),
    );
}

#[cfg(target_os = "linux")]
fn parse_json_body(body: &str) -> Option<JsonValue> {
    serde_json::from_str(body).ok()
}

#[cfg(target_os = "linux")]
fn infer_mail_provider(domain: &str) -> &'static str {
    let domain = domain.trim().to_ascii_lowercase();
    match domain.as_str() {
        "gmail.com" | "googlemail.com" | "google.com" => "google",
        "outlook.com"
        | "hotmail.com"
        | "live.com"
        | "msn.com"
        | "office365.com"
        | "microsoft.com" => "microsoft",
        _ => "other",
    }
}

#[cfg(target_os = "linux")]
fn codex_agents_md_path() -> PathBuf {
    codex_home_dir().join("AGENTS.md")
}

#[cfg(target_os = "linux")]
fn load_codex_agents_md() -> Result<JsonValue, String> {
    let path = codex_agents_md_path();
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(format!(
                "failed to read {}: {error}",
                path.display()
            ));
        }
    };
    Ok(json!({
        "path": path,
        "contents": contents,
    }))
}

#[cfg(target_os = "linux")]
fn save_codex_agents_md(body: Option<&str>) -> Result<JsonValue, String> {
    let path = codex_agents_md_path();
    let request = body.and_then(parse_json_body).unwrap_or(JsonValue::Null);
    let contents = request
        .get("contents")
        .or_else(|| request.get("value"))
        .or_else(|| request.get("raw"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create {}: {error}",
                parent.display()
            )
        })?;
    }

    fs::write(&path, &contents)
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;

    Ok(json!({
        "path": path,
        "contents": contents,
    }))
}

#[cfg(target_os = "linux")]
fn read_local_file_binary(path: &str) -> Result<(String, &'static str), (u16, String)> {
    let requested_path = PathBuf::from(path);
    let resolved_path = if requested_path.is_absolute() {
        requested_path
    } else {
        std::env::current_dir()
            .map_err(|error| (500, format!("failed to read current directory: {error}")))?
            .join(requested_path)
    };
    let canonical_path = fs::canonicalize(&resolved_path).map_err(|error| {
        let status_code = if error.kind() == std::io::ErrorKind::NotFound {
            404
        } else {
            500
        };
        (
            status_code,
            format!(
                "failed to resolve file path {}: {error}",
                resolved_path.display()
            ),
        )
    })?;

    if !is_allowed_native_file_read(&canonical_path) {
        return Err((
            403,
            format!("file access is not allowed: {}", canonical_path.display()),
        ));
    }

    let bytes = fs::read(&canonical_path).map_err(|error| {
        let status_code = if error.kind() == std::io::ErrorKind::NotFound {
            404
        } else {
            500
        };
        (
            status_code,
            format!("failed to read file {}: {error}", canonical_path.display()),
        )
    })?;

    Ok((encode_base64(&bytes), guess_mime(&canonical_path)))
}

#[cfg(target_os = "linux")]
fn is_allowed_native_file_read(path: &Path) -> bool {
    let cwd = std::env::current_dir().ok();
    let codex_home = fs::canonicalize(codex_home_dir()).ok();
    let temp_dir = fs::canonicalize(std::env::temp_dir()).ok();

    cwd.as_ref().is_some_and(|root| path.starts_with(root))
        || codex_home
            .as_ref()
            .is_some_and(|root| path.starts_with(root))
        || temp_dir.as_ref().is_some_and(|root| path.starts_with(root))
}

#[cfg(target_os = "linux")]
fn encode_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        let combined = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);

        output.push(TABLE[((combined >> 18) & 0x3f) as usize] as char);
        output.push(TABLE[((combined >> 12) & 0x3f) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[((combined >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(combined & 0x3f) as usize] as char
        } else {
            '='
        });
    }

    output
}

#[cfg(target_os = "linux")]
fn parse_query_parameter<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|segment| {
        let (candidate_key, value) = segment.split_once('=')?;
        if candidate_key == key {
            Some(value)
        } else {
            None
        }
    })
}

#[cfg(target_os = "linux")]
fn parse_aip_connector_metadata_request(url: &str) -> Option<&str> {
    let connector_path = url.strip_prefix("/aip/connectors/")?;
    let connector_id = connector_path.split('?').next().unwrap_or(connector_path);

    if connector_id.is_empty() || connector_id.contains('/') {
        return None;
    }

    Some(connector_id)
}

#[cfg(target_os = "linux")]
fn parse_aip_connector_logo_request(url: &str) -> Option<(&str, &str)> {
    let connector_path = url.strip_prefix("/aip/connectors/")?;
    let (path_without_query, query) = connector_path
        .split_once('?')
        .unwrap_or((connector_path, ""));
    let connector_id = path_without_query.strip_suffix("/logo")?;

    if connector_id.is_empty() || connector_id.contains('/') {
        return None;
    }

    Some((
        connector_id,
        parse_query_parameter(query, "theme").unwrap_or("light"),
    ))
}

#[cfg(target_os = "linux")]
fn parse_aip_connector_tos_request(url: &str) -> Option<&str> {
    let connector_path = url.strip_prefix("/aip/connectors/")?;
    let path_without_query = connector_path.split('?').next().unwrap_or(connector_path);
    let connector_id = path_without_query.strip_suffix("/tos")?;

    if connector_id.is_empty() || connector_id.contains('/') {
        return None;
    }

    Some(connector_id)
}

#[cfg(target_os = "linux")]
fn connector_stub_metadata_response(connector_id: &str) -> JsonValue {
    let name = connector_stub_name(connector_id);

    json!({
        "id": connector_id,
        "name": name,
        "display_name": name,
        "description": JsonValue::Null,
        "logo_url": format!("connectors://{connector_id}/logo?theme=light"),
        "logo_url_dark": format!("connectors://{connector_id}/logo?theme=dark"),
        "link_params_schema": JsonValue::Null,
        "supported_auth": [
            {
                "type": "oauth",
            }
        ],
        "branding": {
            "developer": JsonValue::Null,
        },
        "app_metadata": {
            "developer": JsonValue::Null,
        },
    })
}

#[cfg(target_os = "linux")]
fn connector_stub_logo_response(connector_id: &str, theme: &str) -> JsonValue {
    let (background, panel, foreground) = if theme.eq_ignore_ascii_case("dark") {
        ("#0f172a", "#1e293b", "#f8fafc")
    } else {
        ("#f8fafc", "#e2e8f0", "#0f172a")
    };
    let glyph = if connector_id.starts_with("asdk_app_") {
        "A"
    } else {
        "C"
    };
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="96" viewBox="0 0 96 96" fill="none"><rect width="96" height="96" rx="24" fill="{background}"/><rect x="12" y="12" width="72" height="72" rx="18" fill="{panel}"/><text x="48" y="58" text-anchor="middle" font-family="Arial, sans-serif" font-size="36" font-weight="700" fill="{foreground}">{glyph}</text></svg>"#
    );

    json!({
        "contentType": "image/svg+xml",
        "base64": encode_base64(svg.as_bytes()),
    })
}

#[cfg(target_os = "linux")]
fn connector_stub_name(connector_id: &str) -> String {
    let (prefix, raw_suffix) = if let Some(suffix) = connector_id.strip_prefix("asdk_app_") {
        ("App", suffix)
    } else if let Some(suffix) = connector_id.strip_prefix("connector_") {
        ("Connector", suffix)
    } else {
        ("Connector", connector_id)
    };
    let short_suffix = raw_suffix.chars().take(6).collect::<String>();

    if short_suffix.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix} {short_suffix}")
    }
}

#[cfg(target_os = "linux")]
fn read_workspace_directory_entries(
    global_state: &JsonMapState,
    request: JsonValue,
) -> Vec<JsonValue> {
    let workspace_root = request
        .get("workspaceRoot")
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
        .or_else(|| {
            current_active_workspace_roots(global_state)
                .into_iter()
                .next()
                .map(PathBuf::from)
        });
    let include_hidden = request
        .get("includeHidden")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let directory_path = request
        .get("directoryPath")
        .and_then(JsonValue::as_str)
        .map(PathBuf::from);

    let Some(root) = workspace_root else {
        return Vec::new();
    };

    let target = directory_path.unwrap_or_else(|| root.clone());
    let canonical_root = fs::canonicalize(&root).unwrap_or(root.clone());
    let canonical_target = fs::canonicalize(&target).unwrap_or(target);

    if !canonical_target.starts_with(&canonical_root) || !canonical_target.is_dir() {
        return Vec::new();
    }

    let mut entries = fs::read_dir(&canonical_target)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_hidden = name.starts_with('.');

            if !include_hidden && is_hidden {
                return None;
            }

            let entry_type = entry.file_type().ok()?;
            let path_string = path.to_string_lossy().to_string();
            let kind = if entry_type.is_dir() {
                "directory"
            } else {
                "file"
            };

            Some(json!({
                "name": name,
                "path": path_string,
                "type": kind,
                "isHidden": is_hidden,
            }))
        })
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        let left_type = left
            .get("type")
            .and_then(JsonValue::as_str)
            .unwrap_or("file");
        let right_type = right
            .get("type")
            .and_then(JsonValue::as_str)
            .unwrap_or("file");
        let left_name = left.get("name").and_then(JsonValue::as_str).unwrap_or("");
        let right_name = right.get("name").and_then(JsonValue::as_str).unwrap_or("");

        left_type
            .cmp(right_type)
            .reverse()
            .then_with(|| left_name.to_lowercase().cmp(&right_name.to_lowercase()))
    });

    entries
}

#[cfg(target_os = "linux")]
fn is_statsig_initialize_url(url: &str) -> bool {
    url.starts_with("https://ab.chatgpt.com/v1/initialize")
        || url.starts_with("https://featureassets.org/v1/initialize")
}

#[cfg(target_os = "linux")]
fn is_statsig_events_url(url: &str) -> bool {
    url.starts_with("https://ab.chatgpt.com/v1/rgstr")
        || url.starts_with("https://prodregistryv2.org/v1/rgstr")
}

#[cfg(target_os = "linux")]
fn is_statsig_exception_url(url: &str) -> bool {
    url.starts_with("https://statsigapi.net/v1/sdk_exception")
}

#[cfg(target_os = "linux")]
fn parse_fetch_headers(payload: &JsonValue) -> Vec<(String, String)> {
    payload
        .get("headers")
        .and_then(JsonValue::as_object)
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(key, value)| {
                    value
                        .as_str()
                        .map(|value| (key.to_ascii_lowercase(), value.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn forward_statsig_initialize_request(
    url: &str,
    method: &str,
    headers: &[(String, String)],
    body: Option<&str>,
) -> Result<JsonValue, String> {
    let method = Method::from_bytes(method.as_bytes())
        .map_err(|error| format!("invalid statsig method {method}: {error}"))?;
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| format!("failed to build statsig client: {error}"))?;
    let mut request = client.request(method, url);

    for (key, value) in headers {
        if key == "host" || key == "content-length" || key == "x-codex-base64" {
            continue;
        }
        request = request.header(key, value);
    }

    if let Some(body) = body {
        request = request.body(body.to_string());
    }

    let response = request
        .send()
        .map_err(|error| format!("statsig request failed: {error}"))?;
    let status = response.status();

    if status == reqwest::StatusCode::NO_CONTENT {
        return Ok(default_statsig_initialize_response());
    }

    if !status.is_success() {
        return Err(format!("statsig initialize returned status {status}"));
    }

    response
        .json::<JsonValue>()
        .map_err(|error| format!("failed to parse statsig json response: {error}"))
}

#[cfg(target_os = "linux")]
fn default_statsig_initialize_response() -> JsonValue {
    json!({
        "has_updates": false,
        "data": {
            "user": {
                "userID": null,
                "customIDs": {},
                "custom": {
                    "origin": "codex_vscode",
                }
            },
            "feature_gates": {},
            "dynamic_configs": {},
            "layer_configs": {},
            "sdkParams": {},
            "time": 0
        }
    })
}

#[cfg(target_os = "linux")]
fn default_statsig_feature_overrides() -> JsonValue {
    json!({
        AVATAR_OVERLAY_GATE_KEY: true,
        HEARTBEAT_AUTOMATIONS_GATE_KEY: true,
        "apps": true,
        "memories": true,
        "plugins": true,
        "tool_search": true,
        "tool_suggest": true,
    })
}

#[cfg(target_os = "linux")]
fn initial_persisted_atom_state() -> HashMap<String, JsonValue> {
    HashMap::from([(
        STATSIG_DEFAULT_ENABLE_FEATURES_ATOM_KEY.to_string(),
        default_statsig_feature_overrides(),
    )])
}

#[cfg(target_os = "linux")]
fn write_experimental_feature_enablement(
    global_state: &JsonMapState,
    host_id: &str,
    enablement: JsonValue,
) {
    let mut state = global_state.borrow_mut();
    let store = state
        .entry(EXPERIMENTAL_FEATURE_ENABLEMENT_STATE_KEY.to_string())
        .or_insert_with(|| json!({}));

    if !store.is_object() {
        *store = json!({});
    }

    if let Some(object) = store.as_object_mut() {
        object.insert(host_id.to_string(), enablement);
    }
}

#[cfg(target_os = "linux")]
fn experimental_feature_list(global_state: &JsonMapState, host_id: &str) -> Vec<JsonValue> {
    global_state
        .borrow()
        .get(EXPERIMENTAL_FEATURE_ENABLEMENT_STATE_KEY)
        .and_then(JsonValue::as_object)
        .and_then(|hosts| hosts.get(host_id))
        .and_then(JsonValue::as_object)
        .map(|enablement| {
            enablement
                .iter()
                .map(|(key, value)| {
                    json!({
                        "id": key,
                        "enabled": value.as_bool().unwrap_or(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn read_store_value_or_default(
    store: &JsonMapState,
    key: &str,
    default_fn: fn(&str) -> JsonValue,
) -> JsonValue {
    if let Some(value) = store.borrow().get(key).cloned() {
        return value;
    }

    let value = default_fn(key);
    store.borrow_mut().insert(key.to_string(), value.clone());
    value
}

#[cfg(target_os = "linux")]
fn build_mcp_config(config_state: &JsonMapState) -> JsonValue {
    let mut config = json!({
        "model": null,
        "review_model": null,
        "model_context_window": null,
        "model_auto_compact_token_limit": null,
        "model_provider": null,
        "approval_policy": null,
        "approvals_reviewer": null,
        "sandbox_mode": null,
        "sandbox_workspace_write": null,
        "forced_chatgpt_workspace_id": null,
        "forced_login_method": null,
        "web_search": null,
        "tools": null,
        "profile": null,
        "profiles": {},
        "instructions": null,
        "developer_instructions": null,
        "compact_prompt": null,
        "model_reasoning_effort": null,
        "model_reasoning_summary": null,
        "service_tier": null,
        "model_verbosity": null,
        "analytics": null,
        "mcp_servers": {},
        "apps": {
            "_default": {
                "enabled": true,
                "destructive_enabled": false,
                "open_world_enabled": false,
                "default_tools_approval_mode": null,
                "default_tools_enabled": null,
                "tools": null
            }
        }
    });

    for (key, value) in config_state.borrow().iter() {
        set_json_path(&mut config, key, value.clone());
    }

    normalize_mcp_config(&mut config);
    config
}

#[cfg(target_os = "linux")]
fn normalize_mcp_config(config: &mut JsonValue) {
    let Some(object) = config.as_object_mut() else {
        return;
    };

    if matches!(object.get("analytics"), Some(value) if value.is_null() || !value.is_object()) {
        object.remove("analytics");
    }

    prune_json_nulls(config);
}

#[cfg(target_os = "linux")]
fn prune_json_nulls(value: &mut JsonValue) {
    match value {
        JsonValue::Object(object) => {
            for child in object.values_mut() {
                prune_json_nulls(child);
            }
            object.retain(|_, child| !child.is_null());
        }
        JsonValue::Array(items) => {
            for child in items.iter_mut() {
                prune_json_nulls(child);
            }
        }
        _ => {}
    }
}

#[cfg(target_os = "linux")]
fn set_json_path(root: &mut JsonValue, path: &str, value: JsonValue) {
    let mut current = root;
    let mut segments = path.split('.').peekable();

    while let Some(segment) = segments.next() {
        let is_last = segments.peek().is_none();

        if !current.is_object() {
            *current = json!({});
        }

        let object = current
            .as_object_mut()
            .expect("json path traversal must operate on objects");

        if is_last {
            object.insert(segment.to_string(), value);
            return;
        }

        current = object
            .entry(segment.to_string())
            .or_insert_with(|| json!({}));
    }
}

#[cfg(target_os = "linux")]
fn apply_config_write(config_state: &JsonMapState, params: &JsonValue) -> Result<(), String> {
    let key_path = params
        .get("keyPath")
        .and_then(JsonValue::as_str)
        .ok_or("config/value/write is missing keyPath")?;
    let value = params.get("value").cloned().unwrap_or(JsonValue::Null);
    config_state
        .borrow_mut()
        .insert(key_path.to_string(), value);
    Ok(())
}

#[cfg(target_os = "linux")]
fn apply_config_batch_write(config_state: &JsonMapState, params: &JsonValue) -> Result<(), String> {
    let edits = params
        .get("edits")
        .and_then(JsonValue::as_array)
        .ok_or("config/batchWrite is missing edits")?;

    for edit in edits {
        apply_config_write(config_state, edit)?;
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn native_model_catalog() -> JsonValue {
    json!([
        {
            "id": "gpt-5.4",
            "name": "GPT-5.4",
            "display_name": "GPT-5.4",
            "provider": "openai",
            "available": true,
            "deprecated": false,
            "default_reasoning_effort": "medium",
            "reasoning_effort_options": ["minimal", "low", "medium", "high"],
            "supports_reasoning_effort": true,
            "supports_reasoning_summary": true,
            "service_tiers": ["default"]
        },
        {
            "id": "gpt-5.3-codex",
            "name": "GPT-5.3-Codex",
            "display_name": "GPT-5.3-Codex",
            "provider": "openai",
            "available": true,
            "deprecated": false,
            "default_reasoning_effort": "medium",
            "reasoning_effort_options": ["low", "medium", "high"],
            "supports_reasoning_effort": true,
            "supports_reasoning_summary": true,
            "service_tiers": ["default"]
        },
        {
            "id": "gpt-5.4-mini",
            "name": "GPT-5.4-Mini",
            "display_name": "GPT-5.4-Mini",
            "provider": "openai",
            "available": true,
            "deprecated": false,
            "default_reasoning_effort": "medium",
            "reasoning_effort_options": ["low", "medium", "high"],
            "supports_reasoning_effort": true,
            "supports_reasoning_summary": true,
            "service_tiers": ["default"]
        }
    ])
}

#[cfg(target_os = "linux")]
fn default_global_state_value(key: &str) -> JsonValue {
    match key {
        WORKSPACE_ROOTS_STATE_KEY => json!([]),
        ACTIVE_WORKSPACE_ROOTS_STATE_KEY => json!([]),
        WORKSPACE_ROOT_OPTIONS_STATE_KEY => json!([]),
        WORKSPACE_ROOT_LABELS_STATE_KEY => json!({}),
        "projectless-thread-ids" => json!([]),
        "thread-workspace-root-hints" => json!({}),
        "use-copilot-auth-if-available" => json!(false),
        "mac-menu-bar-enabled" => json!(false),
        "selected-remote-host-id" => JsonValue::Null,
        "remote-projects" => json!([]),
        "active-remote-project-id" => JsonValue::Null,
        "project-order" => json!([]),
        "copilot-default-model" => JsonValue::Null,
        "queued-follow-ups" => json!([]),
        "notifications-turn-mode" => JsonValue::Null,
        "notifications-permissions-enabled" => json!(false),
        "notifications-questions-enabled" => json!(false),
        _ => JsonValue::Null,
    }
}

#[cfg(target_os = "linux")]
fn default_configuration_value(key: &str) -> JsonValue {
    match key {
        "browserAgent" => json!(false),
        "preventSleepWhileRunning" => json!(false),
        "appearanceTheme" => json!("system"),
        "appearanceLightChromeTheme" => JsonValue::Null,
        "appearanceDarkChromeTheme" => JsonValue::Null,
        "sansFontSize" => json!(14),
        "codeFontSize" => json!(13),
        "usePointerCursors" => json!(true),
        "localeOverride" => JsonValue::Null,
        _ => JsonValue::Null,
    }
}

#[cfg(target_os = "linux")]
fn initial_shared_object_state() -> HashMap<String, JsonValue> {
    HashMap::from([
        (
            "host_config".to_string(),
            json!({
                "id": DEFAULT_HOST_ID,
                "display_name": "Local",
                "kind": "local",
            }),
        ),
        ("remote_connections".to_string(), json!([])),
        ("remote_control_connections".to_string(), json!([])),
        (
            "remote_control_connections_state".to_string(),
            json!({
                "available": false,
                "authRequired": false,
            }),
        ),
    ])
}

#[cfg(target_os = "linux")]
fn normalized_workspace_root_list(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .map(|roots| {
            roots
                .iter()
                .filter_map(JsonValue::as_str)
                .filter_map(normalize_workspace_root)
                .fold(Vec::new(), |mut acc, root| {
                    if !acc.contains(&root) {
                        acc.push(root);
                    }
                    acc
                })
        })
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn current_workspace_root_options(global_state: &JsonMapState) -> Vec<String> {
    let state = global_state.borrow();
    let options = state
        .get(WORKSPACE_ROOT_OPTIONS_STATE_KEY)
        .or_else(|| state.get(WORKSPACE_ROOTS_STATE_KEY));
    normalized_workspace_root_list(options)
}

#[cfg(target_os = "linux")]
fn current_active_workspace_roots(global_state: &JsonMapState) -> Vec<String> {
    let state = global_state.borrow();
    if let Some(active_roots) = state.get(ACTIVE_WORKSPACE_ROOTS_STATE_KEY) {
        return normalized_workspace_root_list(Some(active_roots));
    }

    let fallback = state
        .get(WORKSPACE_ROOT_OPTIONS_STATE_KEY)
        .or_else(|| state.get(WORKSPACE_ROOTS_STATE_KEY));
    normalized_workspace_root_list(fallback)
        .into_iter()
        .take(1)
        .collect()
}

#[cfg(target_os = "linux")]
fn selected_workspace_root(global_state: &JsonMapState) -> Option<String> {
    current_active_workspace_roots(global_state)
        .into_iter()
        .next()
}

#[cfg(target_os = "linux")]
fn current_workspace_root_labels(global_state: &JsonMapState, roots: &[String]) -> JsonValue {
    let labels = global_state
        .borrow()
        .get(WORKSPACE_ROOT_LABELS_STATE_KEY)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();

    JsonValue::Object(
        roots
            .iter()
            .map(|root| {
                let label = labels
                    .get(root)
                    .and_then(JsonValue::as_str)
                    .filter(|label| !label.trim().is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        Path::new(root)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .filter(|name| !name.is_empty())
                            .unwrap_or(root)
                            .to_string()
                    });
                (root.clone(), JsonValue::String(label))
            })
            .collect(),
    )
}

#[cfg(target_os = "linux")]
fn rewrite_app_server_request(global_state: &JsonMapState, request: &JsonValue) -> JsonValue {
    let Some(method) = request.get("method").and_then(JsonValue::as_str) else {
        return request.clone();
    };

    if method != "thread/start" {
        return request.clone();
    }

    let Some(selected_root) = selected_workspace_root(global_state) else {
        return request.clone();
    };

    let home_root = std::env::var_os("HOME")
        .and_then(|path| PathBuf::from(path).to_str().map(str::to_owned))
        .and_then(|path| normalize_workspace_root(&path));
    let selected_root_is_home = home_root.as_deref() == Some(selected_root.as_str());

    if selected_root_is_home {
        return request.clone();
    }

    let mut rewritten = request.clone();
    let Some(params) = rewritten
        .get_mut("params")
        .and_then(JsonValue::as_object_mut)
    else {
        return rewritten;
    };

    let should_override_cwd = match params.get("cwd").and_then(JsonValue::as_str) {
        None => true,
        Some(cwd) if cwd.trim().is_empty() => true,
        Some(cwd) => {
            let normalized_cwd = normalize_workspace_root(cwd).unwrap_or_else(|| cwd.to_string());
            match home_root.as_deref() {
                Some(home_root) => normalized_cwd == home_root,
                None => cwd == std::env::var("HOME").unwrap_or_default(),
            }
        }
    };

    if should_override_cwd {
        eprintln!(
            "native-shell: rewriting thread/start cwd to selected workspace root {}",
            selected_root
        );
        params.insert("cwd".to_string(), json!(selected_root));
    }

    let workspace_root_target = if should_override_cwd {
        selected_root.clone()
    } else {
        params
            .get("cwd")
            .and_then(JsonValue::as_str)
            .and_then(normalize_workspace_root)
            .unwrap_or_else(|| selected_root.clone())
    };

    let should_override_workspace_roots = match params.get("workspaceRoots") {
        None => true,
        Some(JsonValue::Array(roots)) if roots.is_empty() => true,
        Some(JsonValue::Array(roots)) => {
            let normalized_roots: Vec<String> = roots
                .iter()
                .filter_map(JsonValue::as_str)
                .map(|root| normalize_workspace_root(root).unwrap_or_else(|| root.to_string()))
                .collect();

            if normalized_roots.is_empty() {
                true
            } else if normalized_roots
                .iter()
                .any(|root| root == &workspace_root_target)
            {
                false
            } else {
                normalized_roots
                    .iter()
                    .all(|root| match home_root.as_deref() {
                        Some(home_root) => root == home_root,
                        None => root == &std::env::var("HOME").unwrap_or_default(),
                    })
            }
        }
        Some(_) => false,
    };

    if should_override_workspace_roots {
        eprintln!(
            "native-shell: rewriting thread/start workspaceRoots to selected workspace root {}",
            workspace_root_target
        );
        params.insert("workspaceRoots".to_string(), json!([workspace_root_target]));
    }

    rewritten
}

#[cfg(target_os = "linux")]
struct WorkspaceRootState {
    active_roots: Vec<String>,
    workspace_root_options: Vec<String>,
    labels: JsonValue,
}

#[cfg(target_os = "linux")]
fn write_workspace_root_state(
    global_state: &JsonMapState,
    active_roots: Vec<String>,
    workspace_root_options: Vec<String>,
) -> WorkspaceRootState {
    let mut labels = global_state
        .borrow()
        .get(WORKSPACE_ROOT_LABELS_STATE_KEY)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    labels.retain(|root, _| {
        workspace_root_options
            .iter()
            .any(|candidate| candidate == root)
    });

    {
        let mut state = global_state.borrow_mut();
        state.insert(
            ACTIVE_WORKSPACE_ROOTS_STATE_KEY.to_string(),
            json!(active_roots.clone()),
        );
        state.insert(
            WORKSPACE_ROOT_OPTIONS_STATE_KEY.to_string(),
            json!(workspace_root_options.clone()),
        );
        state.insert(
            WORKSPACE_ROOTS_STATE_KEY.to_string(),
            json!(workspace_root_options.clone()),
        );
        state.insert(
            WORKSPACE_ROOT_LABELS_STATE_KEY.to_string(),
            JsonValue::Object(labels),
        );
    }

    WorkspaceRootState {
        active_roots,
        labels: current_workspace_root_labels(global_state, &workspace_root_options),
        workspace_root_options,
    }
}

#[cfg(target_os = "linux")]
fn select_workspace_root(global_state: &JsonMapState, root: &str) -> WorkspaceRootState {
    let mut roots = current_workspace_root_options(global_state);
    roots.retain(|existing| existing != root);
    roots.insert(0, root.to_string());
    roots.truncate(MAX_WORKSPACE_ROOTS);
    write_workspace_root_state(global_state, vec![root.to_string()], roots)
}

#[cfg(target_os = "linux")]
fn clear_active_workspace_roots(global_state: &JsonMapState) -> WorkspaceRootState {
    let roots = current_workspace_root_options(global_state);
    write_workspace_root_state(global_state, Vec::new(), roots)
}

#[cfg(target_os = "linux")]
fn replace_workspace_root_options(
    global_state: &JsonMapState,
    roots: &[&str],
) -> WorkspaceRootState {
    let mut normalized_roots = Vec::new();
    for root in roots {
        if let Some(root) = normalize_workspace_root(root) {
            if !normalized_roots.contains(&root) {
                normalized_roots.push(root);
            }
        }
    }
    normalized_roots.truncate(MAX_WORKSPACE_ROOTS);

    let active_roots = current_active_workspace_roots(global_state)
        .into_iter()
        .filter(|root| normalized_roots.iter().any(|candidate| candidate == root))
        .collect();

    write_workspace_root_state(global_state, active_roots, normalized_roots)
}

#[cfg(target_os = "linux")]
fn rename_workspace_root_option(
    global_state: &JsonMapState,
    root: &str,
    label: &str,
) -> WorkspaceRootState {
    if current_workspace_root_options(global_state)
        .iter()
        .any(|existing| existing == root)
    {
        let mut state = global_state.borrow_mut();
        let labels = state
            .entry(WORKSPACE_ROOT_LABELS_STATE_KEY.to_string())
            .or_insert_with(|| json!({}));
        if let Some(labels) = labels.as_object_mut() {
            if label.trim().is_empty() {
                labels.remove(root);
            } else {
                labels.insert(root.to_string(), json!(label.trim()));
            }
        }
    }

    write_workspace_root_state(
        global_state,
        current_active_workspace_roots(global_state),
        current_workspace_root_options(global_state),
    )
}

#[cfg(target_os = "linux")]
fn normalize_workspace_root(root: &str) -> Option<String> {
    let trimmed = root.trim();
    if trimmed.is_empty() {
        return None;
    }

    let expanded = if trimmed == "~" {
        std::env::var_os("HOME").map(PathBuf::from)?
    } else if let Some(suffix) = trimmed.strip_prefix("~/") {
        let mut home = PathBuf::from(std::env::var_os("HOME")?);
        home.push(suffix);
        home
    } else {
        PathBuf::from(trimmed)
    };

    if !expanded.is_absolute() || !expanded.is_dir() {
        return None;
    }

    fs::canonicalize(&expanded)
        .ok()
        .or(Some(expanded))
        .and_then(|path| path.to_str().map(str::to_owned))
}

#[cfg(target_os = "linux")]
fn preferred_workspace_root(global_state: &JsonMapState) -> Option<String> {
    if let Some(root) = current_workspace_root_options(global_state)
        .into_iter()
        .next()
    {
        return Some(root);
    }

    if let Ok(current_dir) = std::env::current_dir() {
        let current_dir = current_dir.to_string_lossy().to_string();
        if current_dir != "/" {
            if let Some(root) = normalize_workspace_root(&current_dir) {
                return Some(root);
            }
        }
    }

    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    for candidate in [
        home.join("Projects"),
        home.join("Code"),
        home.join("src"),
        home.join("dev"),
        home.join("Development"),
        home.join("Documents"),
        home.clone(),
    ] {
        if let Some(root) = candidate.to_str().and_then(normalize_workspace_root) {
            return Some(root);
        }
    }

    None
}

#[cfg(target_os = "linux")]
fn prompt_for_workspace_root(window: &Window, global_state: &JsonMapState) -> Option<String> {
    let dialog = FileChooserDialog::new(
        Some("Select Project Folder"),
        Some(window),
        FileChooserAction::SelectFolder,
    );
    dialog.add_buttons(&[
        ("Cancel", ResponseType::Cancel),
        ("Open", ResponseType::Accept),
    ]);
    dialog.set_modal(true);

    if let Some(current_root) = preferred_workspace_root(global_state) {
        let _ = dialog.set_current_folder(current_root);
    } else if let Some(home) = std::env::var_os("HOME") {
        let _ = dialog.set_current_folder(PathBuf::from(home));
    }

    let response = dialog.run();
    let selected_root = if response == ResponseType::Accept {
        dialog
            .filename()
            .and_then(|path| path.to_str().and_then(normalize_workspace_root))
    } else {
        None
    };
    dialog.close();
    selected_root
}

#[cfg(target_os = "linux")]
fn is_locally_handled_mcp_method(method: &str) -> bool {
    matches!(
        method,
        "account/read"
            | "account/login/start"
            | "account/login/cancel"
            | "account/logout"
            | "experimentalFeature/enablement/set"
    )
}

#[cfg(target_os = "linux")]
fn start_app_server_bridge(
    event_tx: glib::Sender<AppServerBridgeEvent>,
) -> Result<AppServerBridge, String> {
    let mut child = Command::new("codex")
        .args([
            "app-server",
            "-c",
            "analytics={ enabled = false }",
            "--listen",
            "stdio://",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to spawn codex app-server: {error}"))?;

    let stdin = child
        .stdin
        .take()
        .ok_or("failed to capture codex app-server stdin")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("failed to capture codex app-server stdout")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("failed to capture codex app-server stderr")?;
    let child = Arc::new(Mutex::new(child));
    let writer = Arc::new(Mutex::new(stdin));
    let bridge = AppServerBridge {
        child: child.clone(),
        writer: writer.clone(),
    };

    bridge.send_json(&json!({
        "id": "native-shell:init",
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "codex-native",
                "version": "0.1.0",
            },
            "capabilities": {
                "experimentalApi": true,
            },
        },
    }))?;

    let stderr_tx = event_tx.clone();
    thread::spawn(move || {
        let stderr_reader = BufReader::new(stderr);
        for line in stderr_reader.lines() {
            match line {
                Ok(line) => {
                    if !line.trim().is_empty() {
                        eprintln!("native-shell: codex app-server stderr: {line}");
                    }
                }
                Err(error) => {
                    let _ = stderr_tx.send(AppServerBridgeEvent::Fatal(format!(
                        "failed reading app-server stderr: {error}"
                    )));
                    break;
                }
            }
        }
    });

    thread::spawn(move || {
        let stdout_reader = BufReader::new(stdout);
        for line in stdout_reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(error) => {
                    let _ = event_tx.send(AppServerBridgeEvent::Fatal(format!(
                        "failed reading app-server stdout: {error}"
                    )));
                    return;
                }
            };

            if line.trim().is_empty() {
                continue;
            }

            let message = match serde_json::from_str::<JsonValue>(&line) {
                Ok(message) => message,
                Err(error) => {
                    let _ = event_tx.send(AppServerBridgeEvent::Fatal(format!(
                        "invalid app-server json: {error}"
                    )));
                    continue;
                }
            };

            if message.get("id").and_then(JsonValue::as_str) == Some("native-shell:init") {
                if message.get("error").is_some() {
                    let _ = event_tx.send(AppServerBridgeEvent::Fatal(
                        message
                            .get("error")
                            .and_then(|error| error.get("message"))
                            .and_then(JsonValue::as_str)
                            .unwrap_or("codex app-server initialize failed")
                            .to_string(),
                    ));
                }
                continue;
            }

            let event = if message.get("method").is_some() && message.get("id").is_some() {
                AppServerBridgeEvent::Request(message)
            } else if let Some(method) = message.get("method").and_then(JsonValue::as_str) {
                AppServerBridgeEvent::Notification {
                    method: method.to_string(),
                    params: message.get("params").cloned().unwrap_or(JsonValue::Null),
                }
            } else if message.get("id").is_some()
                && (message.get("result").is_some() || message.get("error").is_some())
            {
                AppServerBridgeEvent::Response(message)
            } else {
                AppServerBridgeEvent::Fatal(format!(
                    "unsupported app-server message shape: {}",
                    message
                ))
            };

            let _ = event_tx.send(event);
        }

        let _ = event_tx.send(AppServerBridgeEvent::Fatal(
            "codex app-server stdout closed".to_string(),
        ));
    });

    Ok(bridge)
}

#[cfg(target_os = "linux")]
fn handle_app_server_event(webview: &WebView, event: AppServerBridgeEvent) {
    match event {
        AppServerBridgeEvent::Response(message) => {
            dispatch_message_to_view(
                webview,
                &json!({
                    "type": "mcp-response",
                    "hostId": DEFAULT_HOST_ID,
                    "message": message,
                }),
            );
        }
        AppServerBridgeEvent::Notification { method, params } => {
            dispatch_message_to_view(
                webview,
                &json!({
                    "type": "mcp-notification",
                    "hostId": DEFAULT_HOST_ID,
                    "method": method,
                    "params": params,
                }),
            );
        }
        AppServerBridgeEvent::Request(request) => {
            dispatch_message_to_view(
                webview,
                &json!({
                    "type": "mcp-request",
                    "hostId": DEFAULT_HOST_ID,
                    "request": request,
                }),
            );
        }
        AppServerBridgeEvent::Fatal(error_message) => {
            dispatch_message_to_view(
                webview,
                &json!({
                    "type": "codex-app-server-fatal-error",
                    "hostId": DEFAULT_HOST_ID,
                    "errorMessage": error_message,
                }),
            );
        }
    }
}

#[cfg(target_os = "linux")]
fn read_effective_auth_snapshot(auth_state: &JsonMapState) -> LocalAuthSnapshot {
    let stored_auth_mode = auth_state
        .borrow()
        .get("authMode")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);
    let stored_account = auth_state.borrow().get("account").cloned();

    if stored_auth_mode.is_some() || stored_account.is_some() {
        return LocalAuthSnapshot {
            account: stored_account,
        };
    }

    read_local_auth_snapshot()
}

#[cfg(target_os = "linux")]
fn read_local_auth_snapshot() -> LocalAuthSnapshot {
    let auth_path = codex_home_dir().join("auth.json");
    let Ok(contents) = fs::read_to_string(&auth_path) else {
        return LocalAuthSnapshot::default();
    };
    let Ok(auth_json) = serde_json::from_str::<JsonValue>(&contents) else {
        return LocalAuthSnapshot::default();
    };

    let auth_mode = auth_json
        .get("auth_mode")
        .and_then(JsonValue::as_str)
        .map(str::to_owned);

    if let Some(api_key) = auth_json.get("OPENAI_API_KEY").and_then(JsonValue::as_str) {
        if !api_key.trim().is_empty() {
            return LocalAuthSnapshot {
                account: Some(json!({
                    "type": "apiKey",
                })),
            };
        }
    }

    let id_token_claims = auth_json
        .get("tokens")
        .and_then(|tokens| tokens.get("id_token"))
        .and_then(JsonValue::as_str)
        .and_then(decode_jwt_claims);
    let access_token_claims = auth_json
        .get("tokens")
        .and_then(|tokens| tokens.get("access_token"))
        .and_then(JsonValue::as_str)
        .and_then(decode_jwt_claims);

    if auth_mode.as_deref() == Some("chatgpt") {
        let email = id_token_claims
            .as_ref()
            .and_then(extract_chatgpt_email)
            .or_else(|| access_token_claims.as_ref().and_then(extract_chatgpt_email));
        let plan_type = id_token_claims
            .as_ref()
            .and_then(extract_chatgpt_plan_type)
            .or_else(|| {
                access_token_claims
                    .as_ref()
                    .and_then(extract_chatgpt_plan_type)
            });

        if let Some(email) = email {
            return LocalAuthSnapshot {
                account: Some(json!({
                    "type": "chatgpt",
                    "email": email,
                    "planType": normalize_plan_type(plan_type),
                })),
            };
        }
    }

    LocalAuthSnapshot { account: None }
}

#[cfg(target_os = "linux")]
fn codex_home_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("CODEX_HOME") {
        return PathBuf::from(path);
    }

    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".codex");
    }

    PathBuf::from("/home/jade/.codex")
}

#[cfg(target_os = "linux")]
fn decode_jwt_claims(token: &str) -> Option<JsonValue> {
    let mut segments = token.split('.');
    let _header = segments.next()?;
    let payload = segments.next()?;
    let bytes = decode_base64_url(payload)?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(target_os = "linux")]
fn decode_base64_url(input: &str) -> Option<Vec<u8>> {
    let mut decoded = Vec::with_capacity(input.len() * 3 / 4);
    let mut chunk = [0u8; 4];
    let mut chunk_len = 0usize;

    for byte in input.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            b'=' => break,
            _ => return None,
        };

        chunk[chunk_len] = value;
        chunk_len += 1;

        if chunk_len == 4 {
            decoded.push((chunk[0] << 2) | (chunk[1] >> 4));
            decoded.push((chunk[1] << 4) | (chunk[2] >> 2));
            decoded.push((chunk[2] << 6) | chunk[3]);
            chunk_len = 0;
        }
    }

    match chunk_len {
        0 => {}
        2 => {
            decoded.push((chunk[0] << 2) | (chunk[1] >> 4));
        }
        3 => {
            decoded.push((chunk[0] << 2) | (chunk[1] >> 4));
            decoded.push((chunk[1] << 4) | (chunk[2] >> 2));
        }
        _ => return None,
    }

    Some(decoded)
}

#[cfg(target_os = "linux")]
fn extract_chatgpt_email(claims: &JsonValue) -> Option<String> {
    if let Some(email) = claims.get("email").and_then(JsonValue::as_str) {
        return Some(email.to_string());
    }

    claims
        .get("https://api.openai.com/profile")
        .and_then(|profile| profile.get("email"))
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

#[cfg(target_os = "linux")]
fn extract_chatgpt_plan_type<'a>(claims: &'a JsonValue) -> Option<&'a str> {
    claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_plan_type"))
        .and_then(JsonValue::as_str)
}

#[cfg(target_os = "linux")]
fn normalize_plan_type(plan_type: Option<&str>) -> &'static str {
    match plan_type.unwrap_or("unknown") {
        "free" => "free",
        "go" => "go",
        "plus" => "plus",
        "pro" => "pro",
        "team" => "team",
        "self_serve_business_usage_based" => "self_serve_business_usage_based",
        "business" => "business",
        "enterprise_cbp_usage_based" => "enterprise_cbp_usage_based",
        "enterprise" => "enterprise",
        "edu" => "edu",
        _ => "unknown",
    }
}

#[cfg(all(target_os = "linux", test))]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_global_state_with_workspace(root: &str) -> JsonMapState {
        let state: JsonMapState = Rc::new(RefCell::new(HashMap::new()));
        state.borrow_mut().insert(
            WORKSPACE_ROOTS_STATE_KEY.to_string(),
            json!([root.to_string()]),
        );
        state.borrow_mut().insert(
            WORKSPACE_ROOT_OPTIONS_STATE_KEY.to_string(),
            json!([root.to_string()]),
        );
        state.borrow_mut().insert(
            ACTIVE_WORKSPACE_ROOTS_STATE_KEY.to_string(),
            json!([root.to_string()]),
        );
        state
    }

    fn temp_workspace_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("codex-native-{name}-{unique}"));
        fs::create_dir_all(&path).expect("temporary workspace should be created");
        path
    }

    #[test]
    fn rewrite_thread_start_uses_selected_workspace_when_cwd_missing() {
        let selected_root = temp_workspace_dir("rewrite-missing-cwd");
        let selected_root = fs::canonicalize(&selected_root)
            .expect("selected root should canonicalize")
            .to_string_lossy()
            .to_string();
        let global_state = test_global_state_with_workspace(&selected_root);
        let request = json!({
            "id": "req-1",
            "method": "thread/start",
            "params": {
                "model": "gpt-5.4"
            }
        });

        let rewritten = rewrite_app_server_request(&global_state, &request);

        assert_eq!(
            rewritten
                .get("params")
                .and_then(|params| params.get("cwd"))
                .and_then(JsonValue::as_str),
            Some(selected_root.as_str())
        );

        let _ = fs::remove_dir_all(PathBuf::from(&selected_root));
    }

    #[test]
    fn rewrite_thread_start_replaces_home_cwd_with_selected_workspace() {
        let selected_root = temp_workspace_dir("rewrite-home-cwd");
        let selected_root = fs::canonicalize(&selected_root)
            .expect("selected root should canonicalize")
            .to_string_lossy()
            .to_string();
        let global_state = test_global_state_with_workspace(&selected_root);
        let home = std::env::var("HOME").expect("HOME should be set for shell tests");
        let request = json!({
            "id": "req-2",
            "method": "thread/start",
            "params": {
                "cwd": home,
                "workspaceRoots": [std::env::var("HOME").expect("HOME should be set for shell tests")]
            }
        });

        let rewritten = rewrite_app_server_request(&global_state, &request);

        assert_eq!(
            rewritten
                .get("params")
                .and_then(|params| params.get("cwd"))
                .and_then(JsonValue::as_str),
            Some(selected_root.as_str())
        );
        assert_eq!(
            rewritten
                .get("params")
                .and_then(|params| params.get("workspaceRoots"))
                .and_then(JsonValue::as_array)
                .and_then(|roots| roots.first())
                .and_then(JsonValue::as_str),
            Some(selected_root.as_str())
        );

        let _ = fs::remove_dir_all(PathBuf::from(&selected_root));
    }

    #[test]
    fn rewrite_thread_start_keeps_explicit_project_cwd() {
        let selected_root = temp_workspace_dir("rewrite-selected-root");
        let selected_root = fs::canonicalize(&selected_root)
            .expect("selected root should canonicalize")
            .to_string_lossy()
            .to_string();
        let explicit_root = temp_workspace_dir("rewrite-explicit-root");
        let explicit_root = fs::canonicalize(&explicit_root)
            .expect("explicit root should canonicalize")
            .to_string_lossy()
            .to_string();
        let global_state = test_global_state_with_workspace(&selected_root);
        let request = json!({
            "id": "req-3",
            "method": "thread/start",
            "params": {
                "cwd": explicit_root
            }
        });

        let rewritten = rewrite_app_server_request(&global_state, &request);

        assert_eq!(
            rewritten
                .get("params")
                .and_then(|params| params.get("cwd"))
                .and_then(JsonValue::as_str),
            Some(explicit_root.as_str())
        );
        assert_eq!(
            rewritten
                .get("params")
                .and_then(|params| params.get("workspaceRoots"))
                .and_then(JsonValue::as_array)
                .and_then(|roots| roots.first())
                .and_then(JsonValue::as_str),
            Some(explicit_root.as_str())
        );

        let _ = fs::remove_dir_all(PathBuf::from(&selected_root));
        let _ = fs::remove_dir_all(PathBuf::from(&explicit_root));
    }

    #[test]
    fn rewrite_thread_start_keeps_explicit_project_workspace_roots() {
        let selected_root = temp_workspace_dir("rewrite-selected-workspace-root");
        let selected_root = fs::canonicalize(&selected_root)
            .expect("selected root should canonicalize")
            .to_string_lossy()
            .to_string();
        let explicit_root = temp_workspace_dir("rewrite-explicit-workspace-root");
        let explicit_root = fs::canonicalize(&explicit_root)
            .expect("explicit root should canonicalize")
            .to_string_lossy()
            .to_string();
        let global_state = test_global_state_with_workspace(&selected_root);
        let request = json!({
            "id": "req-4",
            "method": "thread/start",
            "params": {
                "workspaceRoots": [explicit_root]
            }
        });

        let rewritten = rewrite_app_server_request(&global_state, &request);

        assert_eq!(
            rewritten
                .get("params")
                .and_then(|params| params.get("workspaceRoots"))
                .and_then(JsonValue::as_array)
                .and_then(|roots| roots.first())
                .and_then(JsonValue::as_str),
            Some(explicit_root.as_str())
        );

        let _ = fs::remove_dir_all(PathBuf::from(&selected_root));
        let _ = fs::remove_dir_all(PathBuf::from(&explicit_root));
    }

    #[test]
    fn workspace_query_invalidation_payloads_target_workspace_queries() {
        let payloads = workspace_query_invalidation_payloads();

        assert_eq!(
            payloads[0],
            json!({
                "type": "query-cache-invalidate",
                "queryKey": ["vscode", "active-workspace-roots"],
            })
        );
        assert_eq!(
            payloads[1],
            json!({
                "type": "query-cache-invalidate",
                "queryKey": ["vscode", "workspace-root-options"],
            })
        );
    }

    #[test]
    fn clearing_active_workspace_keeps_saved_workspace_options() {
        let first_root = temp_workspace_dir("workspace-state-first");
        let first_root = fs::canonicalize(&first_root)
            .expect("first root should canonicalize")
            .to_string_lossy()
            .to_string();
        let second_root = temp_workspace_dir("workspace-state-second");
        let second_root = fs::canonicalize(&second_root)
            .expect("second root should canonicalize")
            .to_string_lossy()
            .to_string();
        let global_state = test_global_state_with_workspace(&first_root);

        let selected_state = select_workspace_root(&global_state, &second_root);
        assert_eq!(selected_state.active_roots, vec![second_root.clone()]);
        assert_eq!(
            selected_state.workspace_root_options,
            vec![second_root.clone(), first_root.clone()]
        );

        let cleared_state = clear_active_workspace_roots(&global_state);
        assert!(cleared_state.active_roots.is_empty());
        assert_eq!(
            cleared_state.workspace_root_options,
            vec![second_root.clone(), first_root.clone()]
        );
        assert_eq!(
            current_active_workspace_roots(&global_state),
            Vec::<String>::new()
        );
        assert_eq!(
            current_workspace_root_options(&global_state),
            vec![second_root.clone(), first_root.clone()]
        );

        let _ = fs::remove_dir_all(PathBuf::from(&first_root));
        let _ = fs::remove_dir_all(PathBuf::from(&second_root));
    }

    #[test]
    fn initial_persisted_atoms_seed_statsig_feature_overrides() {
        let state = initial_persisted_atom_state();
        let overrides = state
            .get(STATSIG_DEFAULT_ENABLE_FEATURES_ATOM_KEY)
            .and_then(JsonValue::as_object)
            .expect("statsig overrides should exist");

        assert_eq!(
            overrides.get(AVATAR_OVERLAY_GATE_KEY).and_then(JsonValue::as_bool),
            Some(true)
        );
        assert_eq!(
            overrides
                .get(HEARTBEAT_AUTOMATIONS_GATE_KEY)
                .and_then(JsonValue::as_bool),
            Some(true)
        );
    }

    #[test]
    fn experimental_feature_enablement_is_stored_per_host() {
        let state: JsonMapState = Rc::new(RefCell::new(HashMap::new()));

        write_experimental_feature_enablement(
            &state,
            "local",
            json!({
                "plugins": true,
                "tool_search": false,
            }),
        );

        let features = experimental_feature_list(&state, "local");
        let mut by_id = HashMap::new();
        for feature in features {
            let id = feature
                .get("id")
                .and_then(JsonValue::as_str)
                .expect("feature id should be a string")
                .to_string();
            let enabled = feature
                .get("enabled")
                .and_then(JsonValue::as_bool)
                .expect("feature enabled should be a bool");
            by_id.insert(id, enabled);
        }

        assert_eq!(by_id.get("plugins"), Some(&true));
        assert_eq!(by_id.get("tool_search"), Some(&false));
    }

    #[test]
    fn patch_home_hero_mascot_replaces_placeholder_icon() {
        let source = format!(
            "prefix {} middle {} suffix",
            HOME_HERO_MASCOT_MARKER, HOME_HERO_ICON_SNIPPET
        );

        let patched = patch_home_hero_mascot(source.into_bytes());
        let patched = String::from_utf8(patched).expect("patched hero should remain utf-8");

        assert!(patched.contains(HOME_HERO_MASCOT_SNIPPET));
        assert!(!patched.contains(HOME_HERO_ICON_SNIPPET));
    }

    #[test]
    fn patch_home_hero_mascot_skips_unrelated_assets() {
        let source = b"console.log('plain asset')".to_vec();

        let patched = apply_frontend_asset_overrides(Path::new("assets/plain.js"), source.clone());

        assert_eq!(patched, source);
    }
}
