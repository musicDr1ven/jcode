use anyhow::Result;
use jcode::auth::{AuthState, AuthStatus};
use jcode::message::{ContentBlock, Message, Role, StreamEvent, ToolDefinition};
use jcode::provider::{MultiProvider, Provider};
use jcode::provider_catalog::{
    OPENAI_COMPAT_LOGIN_PROVIDER, login_providers, openai_compatible_profiles,
};
use jcode::tui::login_picker::{LoginPicker, LoginPickerItem, LoginPickerSummary};
use jcode_provider_openrouter_runtime::OpenRouterProvider;
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, mpsc};
use std::time::{Duration, Instant};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tracked_env_vars() -> Vec<String> {
    let mut keys: HashSet<String> = [
        "HOME",
        "APPDATA",
        "XDG_CONFIG_HOME",
        "JCODE_HOME",
        "NO_PROXY",
        "no_proxy",
        "JCODE_OPENROUTER_API_BASE",
        "JCODE_OPENROUTER_API_KEY_NAME",
        "JCODE_OPENROUTER_ENV_FILE",
        "JCODE_OPENROUTER_CACHE_NAMESPACE",
        "JCODE_NEURALWATT_API_BASE",
        "JCODE_ANTHROPIC_API_BASE",
        "JCODE_ANTHROPIC_API_KEY_NAME",
        "JCODE_ANTHROPIC_ENV_FILE",
        "JCODE_ANTHROPIC_AUTH",
        "JCODE_ANTHROPIC_AUTH_HEADER",
        "JCODE_ANTHROPIC_HEADERS",
        "JCODE_ANTHROPIC_MODEL",
        "JCODE_NAMED_PROVIDER_PROFILE",
        "JCODE_PROVIDER_PROFILE_ACTIVE",
        "JCODE_PROVIDER_PROFILE_NAME",
        "JCODE_ACTIVE_PROVIDER",
        "JCODE_RUNTIME_PROVIDER",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "OPENAI_API_KEY",
        "JCODE_OPENROUTER_PROVIDER_FEATURES",
        "JCODE_OPENROUTER_ALLOW_NO_AUTH",
        "JCODE_OPENROUTER_MODEL_CATALOG",
        "JCODE_OPENROUTER_MODEL",
        "JCODE_OPENROUTER_STATIC_MODELS",
        "JCODE_OPENROUTER_AUTH_HEADER",
        "JCODE_OPENROUTER_AUTH_HEADER_NAME",
        "JCODE_OPENROUTER_DYNAMIC_BEARER_PROVIDER",
        "JCODE_OPENROUTER_PROVIDER",
        "JCODE_OPENROUTER_NO_FALLBACK",
        "OPENROUTER_API_KEY",
        "AUTH_FLOW_TEST_KEY",
    ]
    .into_iter()
    .map(ToString::to_string)
    .collect();

    for profile in openai_compatible_profiles() {
        keys.insert(profile.api_key_env.to_string());
    }

    let mut keys: Vec<_> = keys.into_iter().collect();
    keys.sort();
    keys
}

struct TestEnv {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(String, Option<String>)>,
    temp: tempfile::TempDir,
}

impl TestEnv {
    fn new() -> Result<Self> {
        let lock = lock_env();
        let temp = tempfile::Builder::new()
            .prefix("jcode-auth-flow-")
            .tempdir()?;
        let saved = tracked_env_vars()
            .into_iter()
            .map(|key| {
                let value = std::env::var(&key).ok();
                (key, value)
            })
            .collect::<Vec<_>>();

        for (key, _) in &saved {
            jcode::env::remove_var(key);
        }

        jcode::env::set_var("HOME", temp.path());
        jcode::env::set_var("XDG_CONFIG_HOME", temp.path().join("config"));
        jcode::env::set_var("APPDATA", temp.path().join("AppData").join("Roaming"));
        jcode::env::set_var("JCODE_HOME", temp.path().join("jcode-home"));
        jcode::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        jcode::env::set_var("no_proxy", "127.0.0.1,localhost");
        AuthStatus::invalidate_cache();

        Ok(Self {
            _lock: lock,
            saved,
            temp,
        })
    }

    fn configure_openai_compatible_runtime(
        &self,
        api_base: &str,
        cache_namespace: &str,
        key: Option<&str>,
        allow_no_auth: bool,
    ) {
        let _ = self.temp.path();
        jcode::env::set_var("JCODE_OPENROUTER_API_BASE", api_base);
        jcode::env::set_var("JCODE_OPENROUTER_API_KEY_NAME", "AUTH_FLOW_TEST_KEY");
        jcode::env::set_var("JCODE_OPENROUTER_ENV_FILE", "auth-flow-test.env");
        jcode::env::set_var("JCODE_OPENROUTER_CACHE_NAMESPACE", cache_namespace);
        jcode::env::set_var("JCODE_OPENROUTER_PROVIDER_FEATURES", "0");
        jcode::env::set_var("JCODE_OPENROUTER_MODEL_CATALOG", "1");
        if let Some(key) = key {
            jcode::env::set_var("AUTH_FLOW_TEST_KEY", key);
        } else {
            jcode::env::remove_var("AUTH_FLOW_TEST_KEY");
        }
        if allow_no_auth {
            jcode::env::set_var("JCODE_OPENROUTER_ALLOW_NO_AUTH", "1");
        } else {
            jcode::env::remove_var("JCODE_OPENROUTER_ALLOW_NO_AUTH");
        }
        AuthStatus::invalidate_cache();
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        AuthStatus::invalidate_cache();
        for (key, value) in &self.saved {
            if let Some(value) = value {
                jcode::env::set_var(key, value);
            } else {
                jcode::env::remove_var(key);
            }
        }
        AuthStatus::invalidate_cache();
    }
}

struct FakeModelsServer {
    api_base: String,
    requests: mpsc::Receiver<String>,
    request_count: Arc<AtomicUsize>,
}

fn spawn_models_server(
    max_requests: usize,
    body: impl Into<String>,
    delay: Duration,
) -> FakeModelsServer {
    spawn_raw_models_server(max_requests, "200 OK", body, delay)
}

fn spawn_raw_models_server(
    max_requests: usize,
    status: impl Into<String>,
    body: impl Into<String>,
    delay: Duration,
) -> FakeModelsServer {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake provider server");
    listener
        .set_nonblocking(true)
        .expect("set fake server nonblocking");
    let addr = listener.local_addr().expect("fake provider addr");
    let (request_tx, request_rx) = mpsc::channel();
    let request_count = Arc::new(AtomicUsize::new(0));
    let request_count_thread = Arc::clone(&request_count);
    let body = body.into();
    let status = status.into();

    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        while request_count_thread.load(Ordering::SeqCst) < max_requests
            && Instant::now() < deadline
        {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .expect("set read timeout");
                    let mut request = vec![0u8; 8192];
                    let n = stream.read(&mut request).unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..n]).into_owned();
                    request_count_thread.fetch_add(1, Ordering::SeqCst);
                    let _ = request_tx.send(request);

                    if !delay.is_zero() {
                        std::thread::sleep(delay);
                    }
                    let response = format!(
                        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        status,
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

    FakeModelsServer {
        api_base: format!("http://{addr}/v1"),
        requests: request_rx,
        request_count,
    }
}

fn assert_models_request(request: &str) {
    assert!(
        request.starts_with("GET /v1/models "),
        "expected GET /v1/models request, got: {request}"
    );
}

fn lower_headers(request: &str) -> String {
    request.to_ascii_lowercase()
}

fn run_current_thread<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(future)
}

#[test]
fn every_login_provider_has_auth_status_and_tui_copy_metadata() -> Result<()> {
    let _env = TestEnv::new()?;
    let status = AuthStatus::default();

    for provider in login_providers() {
        assert!(!provider.id.trim().is_empty(), "provider id must be set");
        assert!(
            !provider.display_name.trim().is_empty(),
            "{} display name must be set",
            provider.id
        );
        assert!(
            !provider.menu_detail.trim().is_empty(),
            "{} setup copy must be set",
            provider.id
        );
        assert!(
            !provider.auth_kind.label().trim().is_empty(),
            "{} auth kind label must be set",
            provider.id
        );
        assert!(
            !status
                .method_detail_for_provider(*provider)
                .trim()
                .is_empty(),
            "{} detected setup copy must be renderable",
            provider.id
        );
    }

    Ok(())
}

#[test]
fn live_models_contract_uses_models_endpoint_and_bearer_auth() -> Result<()> {
    let env = TestEnv::new()?;
    let server = spawn_models_server(
        1,
        r#"{"data":[{"id":"bearer-contract-model","object":"model"}]}"#,
        Duration::ZERO,
    );
    env.configure_openai_compatible_runtime(
        &server.api_base,
        "auth-flow-bearer-contract",
        Some("sk-bearer-contract"),
        false,
    );

    let provider = OpenRouterProvider::new()?;
    let models = run_current_thread(provider.fetch_models())?;
    assert_eq!(models[0].id, "bearer-contract-model");

    let request = server.requests.recv_timeout(Duration::from_secs(2))?;
    assert_models_request(&request);
    let headers = lower_headers(&request);
    assert!(
        headers.contains("authorization: bearer sk-bearer-contract"),
        "missing bearer auth header: {request}"
    );
    assert!(
        provider
            .available_models_display()
            .iter()
            .any(|model| model == "bearer-contract-model"),
        "fetched model should be visible immediately after refresh"
    );

    Ok(())
}

#[test]
fn live_models_contract_supports_api_key_header_mode() -> Result<()> {
    let env = TestEnv::new()?;
    let server = spawn_models_server(
        1,
        r#"{"data":[{"id":"header-contract-model","object":"model"}]}"#,
        Duration::ZERO,
    );
    env.configure_openai_compatible_runtime(
        &server.api_base,
        "auth-flow-header-contract",
        Some("sk-header-contract"),
        false,
    );
    jcode::env::set_var("JCODE_OPENROUTER_AUTH_HEADER", "api-key");
    jcode::env::set_var("JCODE_OPENROUTER_AUTH_HEADER_NAME", "x-api-key");

    let provider = OpenRouterProvider::new()?;
    let models = run_current_thread(provider.fetch_models())?;
    assert_eq!(models[0].id, "header-contract-model");

    let request = server.requests.recv_timeout(Duration::from_secs(2))?;
    assert_models_request(&request);
    let headers = lower_headers(&request);
    assert!(
        headers.contains("x-api-key: sk-header-contract"),
        "missing x-api-key auth header: {request}"
    );
    assert!(
        !headers.contains("authorization: bearer"),
        "api-key mode must not also send bearer auth: {request}"
    );

    Ok(())
}

#[test]
fn local_no_auth_models_contract_sends_no_auth_header() -> Result<()> {
    let env = TestEnv::new()?;
    let server = spawn_models_server(
        1,
        r#"{"data":[{"id":"local-no-auth-model","object":"model"}]}"#,
        Duration::ZERO,
    );
    env.configure_openai_compatible_runtime(
        &server.api_base,
        "auth-flow-local-no-auth-contract",
        None,
        true,
    );

    let provider = OpenRouterProvider::new()?;
    let models = run_current_thread(provider.fetch_models())?;
    assert_eq!(models[0].id, "local-no-auth-model");

    let request = server.requests.recv_timeout(Duration::from_secs(2))?;
    assert_models_request(&request);
    let headers = lower_headers(&request);
    assert!(
        !headers.contains("authorization:"),
        "local no-auth endpoint should not send Authorization: {request}"
    );
    assert!(
        !headers.contains("api-key:") && !headers.contains("x-api-key:"),
        "local no-auth endpoint should not send API-key headers: {request}"
    );

    Ok(())
}

#[test]
fn model_picker_cache_miss_schedules_single_background_refresh_and_updates_routes() -> Result<()> {
    let env = TestEnv::new()?;
    let server = spawn_models_server(
        2,
        r#"{"data":[{"id":"background-race-live-model","object":"model"}]}"#,
        Duration::from_millis(25),
    );
    env.configure_openai_compatible_runtime(
        &server.api_base,
        "auth-flow-background-race",
        None,
        true,
    );
    jcode::env::set_var("JCODE_OPENROUTER_MODEL", "background-race-selected-model");

    let provider = OpenRouterProvider::new()?;
    run_current_thread(async {
        let first = provider.available_models_display();
        let second = provider.available_models_display();
        assert!(
            first
                .iter()
                .any(|model| model == "background-race-selected-model")
        );
        assert!(
            second
                .iter()
                .any(|model| model == "background-race-selected-model")
        );

        for _ in 0..100 {
            if provider
                .available_models_display()
                .iter()
                .any(|model| model == "background-race-live-model")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let display = provider.available_models_display();
        assert!(
            display
                .iter()
                .any(|model| model == "background-race-live-model"),
            "background refresh should update picker models; display={display:?}"
        );
        let routes = provider.model_routes();
        assert!(
            routes.iter().any(|route| {
                route.model == "background-race-live-model"
                    && route.api_method == "openai-compatible"
                    && route.available
            }),
            "background refresh should update model routes; routes={routes:?}"
        );
    });

    let request = server.requests.recv_timeout(Duration::from_secs(2))?;
    assert_models_request(&request);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        server.request_count.load(Ordering::SeqCst),
        1,
        "concurrent picker renders should coalesce into one background /models request"
    );

    Ok(())
}

#[test]
fn live_model_catalog_failure_keeps_static_and_selected_model_picker_fallbacks() -> Result<()> {
    let env = TestEnv::new()?;
    let server = spawn_raw_models_server(
        1,
        "500 Internal Server Error",
        r#"{"error":"catalog temporarily unavailable"}"#,
        Duration::ZERO,
    );
    env.configure_openai_compatible_runtime(
        &server.api_base,
        "auth-flow-catalog-failure",
        Some("sk-catalog-failure"),
        false,
    );
    jcode::env::set_var("JCODE_OPENROUTER_MODEL", "selected-fallback-model");
    jcode::env::set_var(
        "JCODE_OPENROUTER_STATIC_MODELS",
        "selected-fallback-model\nstatic-fallback-model",
    );

    let provider = OpenRouterProvider::new()?;
    let before = provider.available_models_display();
    assert!(
        before
            .iter()
            .any(|model| model == "selected-fallback-model")
    );
    assert!(before.iter().any(|model| model == "static-fallback-model"));

    let err = run_current_thread(provider.fetch_models()).expect_err("catalog fetch should fail");
    let message = err.to_string();
    assert!(
        message.contains("500") && message.contains("catalog temporarily unavailable"),
        "unexpected catalog failure: {message}"
    );

    let after = provider.available_models_display();
    assert!(
        after.iter().any(|model| model == "selected-fallback-model")
            && after.iter().any(|model| model == "static-fallback-model"),
        "failed live catalog should not remove picker fallbacks; before={before:?} after={after:?}"
    );
    let request = server.requests.recv_timeout(Duration::from_secs(2))?;
    assert_models_request(&request);

    Ok(())
}

fn buffer_to_text(buffer: &Buffer) -> String {
    let area = buffer.area;
    let mut out = String::new();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

#[test]
fn login_picker_renders_openai_compatible_setup_copy() -> Result<()> {
    let mut picker = LoginPicker::with_summary(
        " Login ",
        vec![LoginPickerItem::new(
            1,
            OPENAI_COMPAT_LOGIN_PROVIDER,
            AuthState::Available,
            "http://localhost:11434/v1 configured; no API key required",
        )],
        LoginPickerSummary {
            ready_count: 1,
            recommended_count: 0,
            ..LoginPickerSummary::default()
        },
    );

    let backend = TestBackend::new(120, 44);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| picker.render(frame))?;
    let text = buffer_to_text(terminal.backend().buffer());

    for expected in [
        "OpenAI-compatible",
        "/login openai-compatible",
        "API key / CLI",
        "Detected setup",
        "http://localhost:11434/v1 configured; no API key required",
        "custom endpoint setup: base URL first, then API key",
        "Press Enter to begin login.",
    ] {
        assert!(
            text.contains(expected),
            "login picker should render expected copy {expected:?}; rendered:\n{text}"
        );
    }

    Ok(())
}

// --- Neuralwatt preset: endpoint override, catalog, listing, requests ---

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = stream.read(&mut chunk).unwrap_or(0);
        if read == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..read]);
        let Some(end) = buf.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&buf[..end]).into_owned();
        let mut content_length = 0usize;
        for line in headers.lines() {
            if let Some((key, value)) = line.split_once(':')
                && key.eq_ignore_ascii_case("content-length")
            {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
        if buf.len() >= end + 4 + content_length {
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Fake Neuralwatt endpoint: every `/v1/models` request gets the catalog, and
/// each `/v1/chat/completions` request consumes the next scripted stream. This
/// keeps startup's background catalog refreshes from eating a scripted chat
/// response.
fn spawn_neuralwatt_server(chat_script: Vec<String>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fake Neuralwatt server");
    let addr = listener.local_addr().expect("fake Neuralwatt addr");
    let (request_tx, request_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut chat_index = 0usize;
        for _ in 0..64 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");
            let request = read_http_request(&mut stream);
            let is_models = request.starts_with("GET /v1/models");
            let (content_type, body) = if is_models {
                ("application/json", neuralwatt_models_body())
            } else {
                let body = chat_script
                    .get(chat_index)
                    .cloned()
                    .unwrap_or_else(|| neuralwatt_text_stream("done"));
                chat_index += 1;
                ("text/event-stream", body)
            };
            let _ = request_tx.send(request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{addr}/v1"), request_rx)
}

/// Serve exactly one response, for surfaces that make a single request.
fn spawn_single_response_server(
    base_path: &str,
    content_type: &'static str,
    body: String,
) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind single-response server");
    let addr = listener.local_addr().expect("single-response addr");
    let (request_tx, request_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");
        let request = read_http_request(&mut stream);
        let _ = request_tx.send(request);
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
    });
    (format!("http://{addr}{base_path}"), request_rx)
}

fn neuralwatt_models_body() -> String {
    r#"{"object":"list","data":[{"id":"nw-e2e-model","object":"model"}]}"#.to_string()
}

fn neuralwatt_tool_call_stream() -> String {
    concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"bash\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"command\\\":\\\"echo hi\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    )
    .to_string()
}

fn neuralwatt_text_stream(text: &str) -> String {
    format!(
        "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{text}\"}},\"finish_reason\":null}}]}}\n\ndata: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\ndata: [DONE]\n\n"
    )
}

fn configure_neuralwatt(api_base: &str) {
    jcode::env::set_var("NEURALWATT_API_KEY", "sk-nw-e2e");
    jcode::env::set_var("JCODE_NEURALWATT_API_BASE", api_base);
    jcode::provider_catalog::apply_openai_compatible_profile_env(Some(
        jcode::provider_catalog::NEURALWATT_PROFILE,
    ));
    AuthStatus::invalidate_cache();
}

fn collect_stream(
    runtime: &dyn Provider,
    messages: &[Message],
    tools: &[ToolDefinition],
) -> Vec<StreamEvent> {
    run_current_thread(async {
        let mut stream = runtime
            .complete(messages, tools, "", None)
            .await
            .expect("completion should start");
        let mut events = Vec::new();
        while let Some(event) = futures::StreamExt::next(&mut stream).await {
            events.push(event.expect("stream event should parse"));
        }
        events
    })
}

fn wait_for_request(
    requests: &mpsc::Receiver<String>,
    predicate: impl Fn(&str) -> bool,
) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let request = requests
            .recv_timeout(remaining)
            .expect("expected a request to the fake Neuralwatt endpoint");
        if predicate(&request) {
            return request;
        }
    }
}

/// First `/v1/models` succeeds, later ones fail with 500: a refresh failure
/// must not empty or corrupt the catalog that was already served.
fn spawn_flaky_models_server() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind flaky models server");
    let addr = listener.local_addr().expect("flaky models addr");
    let (request_tx, request_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for attempt in 0..4 {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set read timeout");
            let request = read_http_request(&mut stream);
            let _ = request_tx.send(request);
            let response = if attempt == 0 {
                let body = neuralwatt_models_body();
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            } else {
                "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: 27\r\nConnection: close\r\n\r\n{\"error\":\"catalog down\"}".to_string()
            };
            let _ = stream.write_all(response.as_bytes());
        }
    });
    (format!("http://{addr}/v1"), request_rx)
}

#[test]
fn neuralwatt_failed_refresh_keeps_the_previous_catalog() -> Result<()> {
    let _env = TestEnv::new()?;
    let (api_base, _requests) = spawn_flaky_models_server();
    configure_neuralwatt(&api_base);
    let runtime = OpenRouterProvider::new_openai_compatible_profile_runtime(
        jcode::provider_catalog::NEURALWATT_PROFILE,
    )?;
    let models = run_current_thread(runtime.fetch_models())?;
    assert!(models.iter().any(|model| model.id == "nw-e2e-model"));

    let cache_path = std::path::PathBuf::from(
        std::env::var("JCODE_HOME").expect("test JCODE_HOME should be set"),
    )
    .join("cache")
    .join("neuralwatt_models.json");
    let before = std::fs::read_to_string(&cache_path)?;

    let failed = run_current_thread(runtime.refresh_models());
    assert!(
        failed.is_err(),
        "the second fetch must fail against the broken endpoint"
    );
    let after = std::fs::read_to_string(&cache_path)?;
    assert_eq!(
        before, after,
        "a failed refresh must not empty or rewrite the served catalog"
    );
    let cached: serde_json::Value = serde_json::from_str(&after)?;
    assert!(
        cached["models"]
            .as_array()
            .is_some_and(|models| models.iter().any(|model| model["id"] == "nw-e2e-model")),
        "the previous catalog must remain intact: {after}"
    );

    Ok(())
}

#[test]
fn neuralwatt_catalog_fetch_uses_the_override_endpoint_and_its_own_namespace() -> Result<()> {
    let env = TestEnv::new()?;
    let (api_base, requests) = spawn_neuralwatt_server(Vec::new());
    configure_neuralwatt(&api_base);

    let resolved = jcode::provider_catalog::resolve_openai_compatible_profile(
        jcode::provider_catalog::NEURALWATT_PROFILE,
    );
    assert_eq!(
        resolved.api_base, api_base,
        "the endpoint override must apply to the catalog as well"
    );

    let runtime = OpenRouterProvider::new_openai_compatible_profile_runtime(
        jcode::provider_catalog::NEURALWATT_PROFILE,
    )?;
    let models = run_current_thread(runtime.fetch_models())?;
    assert!(
        models.iter().any(|model| model.id == "nw-e2e-model"),
        "the live catalog must be what the endpoint reports: {models:?}"
    );

    let request = wait_for_request(&requests, |request| request.starts_with("GET /v1/models"));
    assert_models_request(&request);
    assert!(
        lower_headers(&request).contains("authorization: bearer sk-nw-e2e"),
        "Neuralwatt's own key must authenticate the catalog request: {request}"
    );
    let cache_path = std::path::PathBuf::from(
        std::env::var("JCODE_HOME").expect("test JCODE_HOME should be set"),
    )
    .join("cache")
    .join("neuralwatt_models.json");
    assert!(
        cache_path.is_file(),
        "the catalog must be cached under Neuralwatt's own namespace: {cache_path:?}"
    );

    let _ = env;
    Ok(())
}

#[test]
fn neuralwatt_models_are_listed_and_a_request_round_trips_a_tool_result() -> Result<()> {
    let _env = TestEnv::new()?;
    let (api_base, requests) = spawn_neuralwatt_server(vec![
        neuralwatt_tool_call_stream(),
        neuralwatt_text_stream("tool done"),
    ]);
    configure_neuralwatt(&api_base);

    // Pull the live catalog first: a session constructed later loads it from
    // Neuralwatt's namespace instead of starting from an empty catalog.
    let runtime = OpenRouterProvider::new_openai_compatible_profile_runtime(
        jcode::provider_catalog::NEURALWATT_PROFILE,
    )?;
    let models = run_current_thread(runtime.fetch_models())?;
    assert!(models.iter().any(|model| model.id == "nw-e2e-model"));
    let catalog_request = wait_for_request(&requests, |request| request.starts_with("GET /v1/models"));
    assert_models_request(&catalog_request);

    // Configured by name: the session runs on Neuralwatt's identity.
    jcode::cli::startup::register_external_provider_runtimes();
    let provider = {
        // Startup spawns background refreshes, so construction needs a runtime
        // context even though the test drives requests separately.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let _guard = rt.enter();
        MultiProvider::from_auth_status(AuthStatus::default())
    };
    assert_eq!(Provider::display_name(&provider), "Neuralwatt");

    // The picker lists the id under Neuralwatt rather than a generic provider.
    let routes = provider.model_routes();
    let route = routes
        .iter()
        .find(|route| route.model == "nw-e2e-model")
        .unwrap_or_else(|| panic!("Neuralwatt model missing from the picker: {routes:?}"));
    assert_eq!(route.provider, "Neuralwatt");
    assert_eq!(route.api_method, "openai-compatible:neuralwatt");

    // Selecting it starts a request to Neuralwatt's endpoint.
    provider.set_model("nw-e2e-model")?;
    assert_eq!(provider.model(), "nw-e2e-model");

    let tools = [ToolDefinition::new(
        "bash",
        "run a shell command",
        serde_json::json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"],
        }),
    )];
    let first_turn = [Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: "list the directory".to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }];
    let events = collect_stream(&provider, &first_turn, &tools);
    assert!(
        events.iter().any(|event| matches!(
            event,
            StreamEvent::ToolUseStart { name, .. } if name == "bash"
        )),
        "expected a streamed tool call, got {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::ToolUseEndFor { .. })),
        "the tool call must complete before the turn ends: {events:?}"
    );

    let chat_request = wait_for_request(&requests, |request| {
        request.starts_with("POST /v1/chat/completions")
    });
    assert!(
        chat_request.starts_with("POST /v1/chat/completions "),
        "the request must reach Neuralwatt's chat endpoint: {chat_request}"
    );
    assert!(
        chat_request.contains(r#""model":"nw-e2e-model""#),
        "the selected model must carry the request: {chat_request}"
    );
    assert!(
        lower_headers(&chat_request).contains("authorization: bearer sk-nw-e2e"),
        "Neuralwatt's own key must authenticate the request: {chat_request}"
    );

    // The tool result goes back in the follow-up request.
    let second_turn = [
        first_turn[0].clone(),
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::ToolUse {
                id: "call_1".to_string(),
                name: "bash".to_string(),
                input: serde_json::json!({ "command": "echo hi" }),
                thought_signature: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
        Message {
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: "call_1".to_string(),
                content: "hi".to_string(),
                is_error: None,
            }],
            timestamp: None,
            tool_duration_ms: None,
        },
    ];
    let final_events = collect_stream(&provider, &second_turn, &tools);
    assert!(
        final_events.iter().any(
            |event| matches!(event, StreamEvent::TextDelta(text) if text == "tool done")
        ),
        "the final turn should render the model's text: {final_events:?}"
    );
    let follow_up = wait_for_request(&requests, |request| {
        request.starts_with("POST /v1/chat/completions")
    });
    assert!(
        follow_up.contains("call_1"),
        "the tool call id must round-trip: {follow_up}"
    );
    assert!(
        follow_up.contains(r#""content":"hi""#) || follow_up.contains(r#""content": "hi""#),
        "the tool result content must round-trip: {follow_up}"
    );

    Ok(())
}

#[test]
fn neuralwatt_anthropic_compatible_named_profile_reaches_messages() -> Result<()> {
    let env = TestEnv::new()?;
    let anthropic_stream = concat!(
        "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"glm-5.3\",\"content\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
        "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello from messages\"}}\n\n",
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
        "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    )
    .to_string();
    let (api_base, requests) =
        spawn_single_response_server("/v1", "text/event-stream", anthropic_stream);
    let config_dir = std::path::PathBuf::from(
        std::env::var("JCODE_HOME").expect("test JCODE_HOME should be set"),
    );
    std::fs::create_dir_all(&config_dir)?;
    std::fs::write(
        config_dir.join("config.toml"),
        format!(
            r#"
[providers.neuralwatt]
type = "anthropic-compatible"
base_url = "{api_base}"
auth = "header"
auth_header = "x-api-key"
api_key_env = "NEURALWATT_API_KEY"
default_model = "glm-5.3"
"#
        ),
    )?;
    jcode::config::invalidate_config_cache();
    jcode::env::set_var("NEURALWATT_API_KEY", "sk-nw-anthropic");
    let profile_name = jcode::provider_catalog::apply_named_provider_profile_env_from_config(
        "neuralwatt",
        &jcode::config::config(),
    )?;
    assert_eq!(profile_name, "neuralwatt");

    let runtime = jcode_provider_anthropic_runtime::AnthropicProvider::new();
    runtime.set_model("glm-5.3")?;
    let messages = [Message {
        role: Role::User,
        content: vec![ContentBlock::Text {
            text: "hello".to_string(),
            cache_control: None,
        }],
        timestamp: None,
        tool_duration_ms: None,
    }];
    let events = collect_stream(&runtime, &messages, &[]);
    assert!(
        events
            .iter()
            .any(|event| matches!(event, StreamEvent::TextDelta(text) if text == "hello from messages")),
        "the Anthropic surface must complete: {events:?}"
    );

    let request = requests.recv_timeout(Duration::from_secs(2))?;
    assert!(
        request.starts_with("POST /v1/messages "),
        "the Anthropic-compatible named profile must reach /v1/messages: {request}"
    );
    let headers = lower_headers(&request);
    assert!(
        headers.contains("x-api-key: sk-nw-anthropic"),
        "auth_header = x-api-key must be honored: {request}"
    );
    assert!(
        !headers.contains("authorization: bearer"),
        "header auth must not also send bearer auth: {request}"
    );

    // `-p neuralwatt` is the preset (OpenAI surface); the Messages-shaped
    // alternative lives behind `--provider-profile`, not `-p`.
    let _ = env;
    Ok(())
}

#[tokio::test]
async fn neuralwatt_provider_doctor_offline_walks_its_checkpoints() -> Result<()> {
    let _env = TestEnv::new()?;
    let report = jcode_provider_doctor::run_provider_e2e(
        jcode::provider_catalog::NEURALWATT_PROFILE,
        None,
        None,
        jcode_provider_doctor::DoctorTier::Offline,
    )
    .await?;
    assert!(
        report.tier_passed,
        "provider-doctor must walk Neuralwatt's offline checkpoints: {report:?}"
    );
    assert!(
        !report.checks.is_empty(),
        "offline doctor must still report checkpoints"
    );
    Ok(())
}
