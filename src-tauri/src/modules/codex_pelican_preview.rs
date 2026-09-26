//! Untrusted results are served on an ephemeral loopback origin, never asset:// or
//! the main WebView. HTTP CSP sandbox applies even to the top-level document.
use std::sync::{Arc, LazyLock};
use tauri::WindowBuilder;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch, Semaphore};
use tokio::time::{timeout, Duration};

const PREFIX: &str = "pelican-preview-";
static SLOTS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(2)));
static REPLACE_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static NEXT_ORDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static BROWSER_STOPS: LazyLock<
    tokio::sync::Mutex<std::collections::VecDeque<watch::Sender<bool>>>,
> = LazyLock::new(|| tokio::sync::Mutex::new(std::collections::VecDeque::new()));
struct PreviewSession {
    webview: wry::WebView,
    window: tauri::Window,
    order: u64,
    _permit: tokio::sync::OwnedSemaphorePermit,
    _stop: watch::Sender<bool>,
}

async fn reserve_slot(app: &tauri::AppHandle) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
    if let Ok(permit) = SLOTS.clone().try_acquire_owned() {
        return Ok(permit);
    }
    let (tx, rx) = oneshot::channel();
    app.run_on_main_thread(move || {
        PREVIEWS.with(|previews| {
            let mut previews = previews.borrow_mut();
            let oldest = previews
                .iter()
                .min_by_key(|(_, session)| session.order)
                .map(|(label, _)| label.clone());
            if let Some(label) = oldest {
                if let Some(session) = previews.remove(&label) {
                    let _ = session.window.destroy();
                }
            }
        });
        let _ = tx.send(());
    })
    .map_err(|error| error.to_string())?;
    tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .map_err(|_| "PELICAN_PREVIEW_REPLACE_TIMEOUT".to_string())?
        .map_err(|_| "PELICAN_PREVIEW_REPLACE_INTERRUPTED".to_string())?;
    SLOTS
        .clone()
        .try_acquire_owned()
        .map_err(|_| "PELICAN_PREVIEW_REPLACE_FAILED".to_string())
}
thread_local! {
    // Wry objects are main-thread-only. Never pass a WebView across thread boundaries.
    static PREVIEWS: std::cell::RefCell<std::collections::HashMap<String, PreviewSession>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

pub async fn close_all(app: &tauri::AppHandle) -> Result<(), String> {
    let _replace_guard = REPLACE_LOCK.lock().await;
    BROWSER_STOPS.lock().await.clear();
    let (tx, rx) = oneshot::channel();
    app.run_on_main_thread(move || {
        PREVIEWS.with(|previews| {
            let sessions: Vec<_> = previews
                .borrow_mut()
                .drain()
                .map(|(_, session)| session)
                .collect();
            for session in sessions {
                let _ = session.window.destroy();
            }
        });
        let _ = tx.send(());
    })
    .map_err(|error| error.to_string())?;
    timeout(Duration::from_secs(5), rx)
        .await
        .map_err(|_| "PELICAN_PREVIEW_CLOSE_TIMEOUT".to_string())?
        .map_err(|_| "PELICAN_PREVIEW_CLOSE_INTERRUPTED".to_string())
}

// Browser previews share the native preview's nonce route and HTTP sandbox.
// Keep at most two listeners, expire them after 15 minutes, and close them when
// test data is cleared. Opening a browser never creates a second native window.
#[tauri::command]
pub async fn codex_pelican_browser(batch_id: String, item_id: String) -> Result<String, String> {
    let artifact = super::codex_pelican::artifact(batch_id, item_id).await?;
    start_browser_preview(
        artifact.html.ok_or("pelican.noHtml")?,
        Duration::from_secs(900),
    )
    .await
}

async fn start_browser_preview(html: String, lifetime: Duration) -> Result<String, String> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| error.to_string())?;
    let host = Arc::new(
        listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .to_string(),
    );
    let path = Arc::new(format!("/{}/result.html", uuid::Uuid::new_v4()));
    let url = format!("http://{host}{path}");
    let response = Arc::new(response_bytes(&format!(
        "<script>{PREVIEW_LOCKDOWN}</script>{html}"
    )));
    let (stop, mut stopped) = watch::channel(false);
    let mut sessions = BROWSER_STOPS.lock().await;
    sessions.retain(|sender| !sender.is_closed());
    while sessions.len() >= 2 {
        sessions.pop_front();
    }
    sessions.push_back(stop);
    drop(sessions);
    tokio::spawn(async move {
        let deadline = tokio::time::sleep(lifetime);
        tokio::pin!(deadline);
        let slots = Arc::new(Semaphore::new(4));
        loop {
            tokio::select! {
                biased;
                _ = stopped.changed() => break,
                _ = &mut deadline => break,
                accepted = listener.accept() => {
                    let Ok((socket, _)) = accepted else { break; };
                    let Ok(permit) = slots.clone().try_acquire_owned() else { continue; };
                    let (path, host, response) = (path.clone(), host.clone(), response.clone());
                    tokio::spawn(async move {
                        let _permit = permit;
                        serve_connection(socket, path, host, response).await;
                    });
                }
            }
        }
    });
    Ok(url)
}
// Inline code is required by model-generated SVG animations. Opaque origin plus
// no remote resources, child frames, workers, forms, objects, or connections.
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src 'none'; connect-src 'none'; frame-src 'none'; child-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; webrtc 'block'; sandbox allow-scripts";
// CSP connect-src does not cover WebRTC on all system WebViews. Execute before
// authored code and make networking constructors immutable. No child realms or
// workers are allowed by the response policy, so they cannot supply fresh ones.
const PREVIEW_LOCKDOWN: &str = r#"(() => {
  for (const key of ['RTCPeerConnection', 'webkitRTCPeerConnection', 'mozRTCPeerConnection', 'RTCDataChannel', 'WebTransport']) {
    try { Object.defineProperty(globalThis, key, {value: undefined, writable: false, configurable: false}); } catch (_) {}
  }
  for (const key of ['alert', 'confirm', 'prompt', 'print']) {
    try { Object.defineProperty(globalThis, key, {value: () => undefined, writable: false, configurable: false}); } catch (_) {}
  }
})()"#;

fn response_bytes(html: &str) -> Vec<u8> {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nContent-Security-Policy: {}\r\nX-Content-Type-Options: nosniff\r\nX-DNS-Prefetch-Control: off\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\nCross-Origin-Resource-Policy: same-origin\r\nPermissions-Policy: camera=(), microphone=(), geolocation=(), clipboard-read=(), clipboard-write=()\r\nConnection: close\r\n\r\n",
        html.len(), CSP,
    );
    [header.as_bytes(), html.as_bytes()].concat()
}

fn valid_request(bytes: &[u8], path: &str, host: &str) -> bool {
    let Ok(request) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut lines = request.split("\r\n");
    if lines.next() != Some(format!("GET {path} HTTP/1.1").as_str()) {
        return false;
    }
    let hosts: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect();
    hosts == [host]
}

async fn serve_connection(
    mut socket: TcpStream,
    path: Arc<String>,
    host: Arc<String>,
    response: Arc<Vec<u8>>,
) {
    let result = timeout(Duration::from_secs(5), async {
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        while request.len() < 8192 && !request.windows(4).any(|w| w == b"\r\n\r\n") {
            let read = socket.read(&mut buffer).await?;
            if read == 0 {
                return Ok::<_, std::io::Error>(());
            }
            request.extend_from_slice(&buffer[..read]);
        }
        if request.len() < 8192 && valid_request(&request, &path, &host) {
            socket.write_all(&response).await?;
        } else {
            socket
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await?;
        }
        socket.shutdown().await
    })
    .await;
    if let Ok(Err(error)) = result {
        crate::modules::logger::log_warn(&format!("[PelicanPreview] connection: {error}"));
    }
}

#[tauri::command]
pub async fn codex_pelican_preview(
    app: tauri::AppHandle,
    batch_id: String,
    item_id: String,
) -> Result<(), String> {
    // Serialize replacement and creation so concurrent clicks cannot consume a slot before
    // their new window is registered and force a third request into an avoidable error.
    let _replace_guard = REPLACE_LOCK.lock().await;
    let artifact = super::codex_pelican::artifact(batch_id.clone(), item_id).await?;
    let html = artifact.html.ok_or("pelican.noHtml")?;
    let permit = reserve_slot(&app).await?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|e| e.to_string())?;
    let host = Arc::new(
        listener
            .local_addr()
            .map_err(|e| e.to_string())?
            .to_string(),
    );
    let nonce = uuid::Uuid::new_v4().to_string();
    let path = Arc::new(format!("/{nonce}/result.html"));
    let url = format!("http://{host}{path}");
    let allowed_url = url.clone();
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let response = Arc::new(response_bytes(&html));
    let listener_task = tokio::spawn(async move {
        let connection_slots = Arc::new(Semaphore::new(4));
        loop {
            tokio::select! {
                biased;
                _ = stop_rx.changed() => break,
                accepted = listener.accept() => {
                    let Ok((socket, _)) = accepted else { break; };
                    let Ok(connection_permit) = connection_slots.clone().try_acquire_owned() else { continue; };
                    let (path, host, response) = (path.clone(), host.clone(), response.clone());
                    tokio::spawn(async move {
                        let _permit = connection_permit;
                        serve_connection(socket, path, host, response).await;
                    });
                }
            }
        }
    });
    let label = format!("{PREFIX}{nonce}");
    let config = super::config::get_user_config();
    let title = super::i18n::translate(&config.language, "pelican.preview", &[]);
    let app_for_window = app.clone();
    let (tx, rx) = oneshot::channel();
    let scheduling = app.run_on_main_thread(move || {
        if tx.is_closed() {
            return;
        }
        let result = (|| -> Result<tauri::Window, String> {
            // A native shell only: the renderer is deliberately NOT a Tauri Webview.
            // No invoke key, plugin initialization, asset protocol, or IPC handler.
            let window = WindowBuilder::new(&app_for_window, &label)
                .title(title)
                .inner_size(1000.0, 760.0)
                .min_inner_size(480.0, 360.0)
                .build()
                .map_err(|e| e.to_string())?;
            let builder = wry::WebViewBuilder::new()
                .with_url(url)
                .with_incognito(true)
                .with_initialization_script_for_main_only(PREVIEW_LOCKDOWN, false)
                .with_devtools(false)
                .with_clipboard(false)
                .with_drag_drop_handler(|_| true)
                .with_navigation_handler(move |url| url == allowed_url)
                .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
                .with_download_started_handler(|_, _| false);
            #[cfg(target_os = "linux")]
            let result = {
                use wry::WebViewBuilderExtUnix;
                window
                    .default_vbox()
                    .map_err(|e| e.to_string())
                    .and_then(|container| builder.build_gtk(&container).map_err(|e| e.to_string()))
            };
            #[cfg(not(target_os = "linux"))]
            let result = builder.build(&window).map_err(|e| e.to_string());
            let webview = match result {
                Ok(webview) => webview,
                Err(error) => {
                    let _ = window.destroy();
                    return Err(error);
                }
            };
            PREVIEWS.with(|previews| {
                previews.borrow_mut().insert(
                    label.clone(),
                    PreviewSession {
                        webview,
                        window: window.clone(),
                        order: NEXT_ORDER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                        _permit: permit,
                        _stop: stop_tx,
                    },
                )
            });
            Ok(window)
        })();
        match result {
            Ok(window) => {
                let app_for_events = app_for_window.clone();
                window.on_window_event(move |event| {
                    if !matches!(
                        event,
                        tauri::WindowEvent::Destroyed | tauri::WindowEvent::Resized(_)
                    ) {
                        return;
                    }
                    let label = label.clone();
                    let size = match event {
                        tauri::WindowEvent::Resized(size) => Some(*size),
                        _ => None,
                    };
                    let _ = app_for_events.run_on_main_thread(move || {
                        PREVIEWS.with(|previews| {
                            let mut previews = previews.borrow_mut();
                            if let Some(size) = size {
                                if let Some(session) = previews.get(&label) {
                                    let _ = session.webview.set_bounds(wry::Rect {
                                        position: wry::dpi::PhysicalPosition::new(0, 0).into(),
                                        size: wry::dpi::PhysicalSize::new(size.width, size.height)
                                            .into(),
                                    });
                                }
                            } else {
                                // Drop releases the renderer, preview slot, and listener sender.
                                previews.remove(&label);
                            }
                        })
                    });
                });
                // If the invoking task disappeared, do not leave an orphan window.
                if tx.send(Ok(())).is_err() {
                    let _ = window.destroy();
                }
            }
            Err(error) => {
                let _ = tx.send(Err(error.to_string()));
            }
        }
    });
    if let Err(error) = scheduling {
        listener_task.abort();
        return Err(error.to_string());
    }
    timeout(Duration::from_secs(30), rx)
        .await
        .map_err(|_| "PELICAN_TIMEOUT".to_string())?
        .map_err(|_| "Preview window creation interrupted".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn browser_previews_are_sandboxed_bounded_and_expire() {
        async fn fetch(url: &str) -> String {
            let url = tauri::Url::parse(url).unwrap();
            let host = format!("127.0.0.1:{}", url.port().unwrap());
            let mut stream = TcpStream::connect(&host).await.unwrap();
            stream
                .write_all(
                    format!("GET {} HTTP/1.1\r\nHost: {host}\r\n\r\n", url.path()).as_bytes(),
                )
                .await
                .unwrap();
            let mut result = String::new();
            stream.read_to_string(&mut result).await.unwrap();
            result
        }
        async fn wait_closed(url: &str) {
            let url = tauri::Url::parse(url).unwrap();
            timeout(Duration::from_secs(2), async {
                loop {
                    if TcpStream::connect(("127.0.0.1", url.port().unwrap()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("preview listener must close");
        }
        let first = start_browser_preview("<svg></svg>".into(), Duration::from_secs(10))
            .await
            .unwrap();
        let response = fetch(&first).await;
        assert!(response.contains("Content-Security-Policy:"));
        assert!(response.contains("sandbox allow-scripts"));
        assert!(response.contains(PREVIEW_LOCKDOWN));
        assert!(response.ends_with("<svg></svg>"));
        let second = start_browser_preview("second".into(), Duration::from_secs(10))
            .await
            .unwrap();
        let third = start_browser_preview("third".into(), Duration::from_millis(100))
            .await
            .unwrap();
        wait_closed(&first).await;
        assert!(fetch(&second).await.ends_with("second"));
        wait_closed(&third).await;
        BROWSER_STOPS.lock().await.clear();
        wait_closed(&second).await;
    }

    #[test]
    fn loopback_route_checks_host_method_and_nonce() {
        let request = b"GET /secret/result.html HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n";
        assert!(valid_request(
            request,
            "/secret/result.html",
            "127.0.0.1:1234"
        ));
        assert!(!valid_request(
            request,
            "/other/result.html",
            "127.0.0.1:1234"
        ));
        assert!(!valid_request(
            request,
            "/secret/result.html",
            "localhost:1234"
        ));
        assert!(!valid_request(
            b"POST /secret/result.html HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n",
            "/secret/result.html",
            "127.0.0.1:1234"
        ));
        assert!(!valid_request(
            b"GET /secret/result.html HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nHost: attacker\r\n\r\n",
            "/secret/result.html",
            "127.0.0.1:1234"
        ));
    }

    #[test]
    fn generated_html_cannot_override_http_security_headers() {
        let html = "<meta http-equiv='Content-Security-Policy' content='default-src *'><script>fetch('http://127.0.0.1:1234/')</script>鹈鹕";
        let bytes = response_bytes(html);
        let text = String::from_utf8(bytes).unwrap();
        let (headers, body) = text.split_once("\r\n\r\n").unwrap();
        assert_eq!(body, html);
        assert!(headers.contains("connect-src 'none'"));
        assert!(headers.contains("sandbox allow-scripts"));
        assert!(!headers.contains("allow-same-origin"));
        assert!(headers.contains(&format!("Content-Length: {}", html.len())));
    }

    #[tokio::test]
    async fn preview_server_serves_only_the_artifact_and_no_cors_access() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let request = format!("GET /nonce/result.html HTTP/1.1\r\nHost: {address}\r\n\r\n");
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            serve_connection(
                socket,
                Arc::new("/nonce/result.html".into()),
                Arc::new(address.to_string()),
                Arc::new(response_bytes("<svg/>")),
            )
            .await;
        });
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        server.await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.ends_with("<svg/>"));
        assert!(!response.contains("Access-Control-Allow-Origin"));
    }
}
