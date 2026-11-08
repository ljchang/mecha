use super::*;
use crate::provider::Provider;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Server {
    endpoint: String,
    seen: Arc<Mutex<Vec<(String, Value)>>>,
    resident: Arc<Mutex<String>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(status: u16, response_model: &str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let resident = Arc::new(Mutex::new("trial-model".to_owned()));
        let (records, model, response_model) =
            (seen.clone(), resident.clone(), response_model.to_owned());
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break None;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(end) = bytes.windows(4).position(|x| x == b"\r\n\r\n") {
                        break Some(end + 4);
                    }
                };
                let Some(end) = header_end else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&bytes[..end]).to_string();
                let size = headers
                    .lines()
                    .find_map(|s| {
                        s.to_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|x| x.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                while bytes.len() < end + size {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                }
                let path = headers.split_whitespace().nth(1).unwrap().to_string();
                let body = serde_json::from_slice(&bytes[end..]).unwrap_or(Value::Null);
                records.lock().unwrap().push((path.clone(), body));
                let (code, body) = match path.as_str() {
                    "/props" => (200, json!({"role":"router"})),
                    "/models" => (
                        200,
                        json!({"data":[{"id":model.lock().unwrap().clone(),"status":{"value":"loaded"}}]}),
                    ),
                    _ => (
                        status,
                        json!({"model":response_model,"choices":[{"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":7,"completion_tokens":2}}),
                    ),
                };
                if code == 999 {
                    std::future::pending::<()>().await;
                }
                let body = body.to_string();
                let response = format!(
                    "HTTP/1.1 {code} Test\r\nLocation: /redirected\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        });
        Self {
            endpoint,
            seen,
            resident,
            task,
        }
    }
    fn inference_count(&self) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|(p, _)| p.starts_with("/v1/chat/completions"))
            .count()
    }
}
struct Temp(std::path::PathBuf);
impl Temp {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("mecha-trial-provider-{}", uuid::Uuid::new_v4())))
    }
    fn config(&self, endpoint: &str) -> LocalTrialConfig {
        LocalTrialConfig {
            endpoint: endpoint.into(),
            model: "trial-model".into(),
            holds_dir: self.0.clone(),
            seed: 42,
        }
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn provider_mode(
    structured: bool,
    config: LocalTrialConfig,
    cancel: CancellationToken,
) -> Result<LocalTrialProvider> {
    if structured {
        LocalTrialProvider::new_structured(config, cancel).await
    } else {
        LocalTrialProvider::new(config, cancel).await
    }
}

#[tokio::test]
async fn rejects_non_numeric_remote_or_decorated_endpoints_before_holding() {
    for structured in [false, true] {
        let dir = Temp::new();
        for endpoint in [
            "http://localhost:8080",
            "https://127.0.0.1:8080",
            "http://192.0.2.1",
            "http://user@127.0.0.1",
            "http://127.0.0.1?x=1",
            "http://127.0.0.1/#x",
            "http://127.0.0.1/arbitrary",
        ] {
            assert!(
                provider_mode(structured, dir.config(endpoint), CancellationToken::new())
                    .await
                    .is_err()
            );
            assert!(!dir.0.exists());
        }
    }
}

#[tokio::test]
async fn accepts_resident_and_holds_until_drop() {
    for structured in [false, true] {
        let server = Server::start(200, "trial-model").await;
        let dir = Temp::new();
        let provider = provider_mode(
            structured,
            dir.config(&server.endpoint),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(
            crate::hold::Holds::new(&dir.0).live(&server.endpoint).len(),
            1
        );
        drop(provider);
        assert!(crate::hold::Holds::new(&dir.0)
            .live(&server.endpoint)
            .is_empty());
    }
}

fn request() -> crate::message::CompletionRequest {
    let mut request = crate::quarantine::QuarantinedPass::new("trial-model", 4096)
        .local_only()
        .ask("synthetic question");
    request.think = Some(false);
    request
}

#[tokio::test]
async fn explicit_structured_constructor_sends_exact_llama_schema_and_audited_body() {
    let server = Server::start(200, "trial-model").await;
    let dir = Temp::new();
    let provider =
        LocalTrialProvider::new_structured(dir.config(&server.endpoint), CancellationToken::new())
            .await
            .unwrap();
    assert!(provider.structured_output());
    let schema = json!({"type":"object","properties":{"answer":{"type":"string","enum":["done"]}},"required":["answer"],"additionalProperties":false});
    let mut req = request();
    req.response_schema = Some(schema.clone());
    let audit = provider.wire_body(&req, false).unwrap();
    let response = provider.complete(&req, None).await.unwrap();
    assert_eq!(response.model, "trial-model");
    let records = server.seen.lock().unwrap();
    let (path, sent) = records
        .iter()
        .find(|(p, _)| p.starts_with("/v1/chat/completions"))
        .unwrap();
    assert_eq!(path, "/v1/chat/completions?autoload=false");
    assert_eq!(sent, &audit);
    assert_eq!(
        sent["response_format"],
        json!({"type":"json_object","schema":schema})
    );
    assert_eq!(sent["seed"], 42);
    assert_eq!(sent["temperature"], 0.6);
    assert_eq!(sent["chat_template_kwargs"]["enable_thinking"], false);
    assert_eq!(
        crate::hold::Holds::new(&dir.0).live(&server.endpoint).len(),
        1
    );
}

#[tokio::test]
async fn legacy_constructor_refuses_schema_without_inference() {
    let server = Server::start(200, "trial-model").await;
    let dir = Temp::new();
    let provider = LocalTrialProvider::new(dir.config(&server.endpoint), CancellationToken::new())
        .await
        .unwrap();
    assert!(!provider.structured_output());
    let mut req = request();
    req.response_schema = Some(json!({"type":"object"}));
    assert!(provider
        .complete(&req, None)
        .await
        .unwrap_err()
        .to_string()
        .contains("structured output"));
    assert_eq!(server.inference_count(), 0);
}

#[tokio::test]
async fn sends_audited_seed_temperature_and_fast_settings_without_mutation() {
    let server = Server::start(200, "trial-model").await;
    let dir = Temp::new();
    let provider = LocalTrialProvider::new(dir.config(&server.endpoint), CancellationToken::new())
        .await
        .unwrap();
    let request = request();
    let audit = provider.wire_body(&request, false).unwrap();
    let response = provider.complete(&request, None).await.unwrap();
    assert_eq!(response.model, "trial-model");
    let records = server.seen.lock().unwrap();
    let (_, sent) = records
        .iter()
        .find(|(p, _)| p.starts_with("/v1/chat/completions"))
        .unwrap();
    assert_eq!(
        records.last().unwrap().0,
        "/v1/chat/completions?autoload=false"
    );
    assert_eq!(sent, &audit);
    assert_eq!(sent["seed"], 42);
    assert_eq!(sent["temperature"], 0.6);
    assert_eq!(sent["chat_template_kwargs"]["enable_thinking"], false);
    assert_eq!(sent["max_tokens"], 4096);
}

#[tokio::test]
async fn resident_change_refuses_before_inference_and_request_cannot_switch_model() {
    for structured in [false, true] {
        let server = Server::start(200, "trial-model").await;
        let dir = Temp::new();
        let provider = provider_mode(
            structured,
            dir.config(&server.endpoint),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let mut changed = request();
        changed.model = "other".into();
        assert!(provider.complete(&changed, None).await.is_err());
        *server.resident.lock().unwrap() = "other".into();
        assert!(provider.complete(&request(), None).await.is_err());
        assert_eq!(server.inference_count(), 0);
    }
}

#[tokio::test]
async fn returned_model_must_match_and_transport_never_retries_or_redirects() {
    for structured in [false, true] {
        for (status, model) in [(200, "other"), (503, "trial-model"), (307, "trial-model")] {
            let server = Server::start(status, model).await;
            let dir = Temp::new();
            let provider = provider_mode(
                structured,
                dir.config(&server.endpoint),
                CancellationToken::new(),
            )
            .await
            .unwrap();
            assert!(provider.complete(&request(), None).await.is_err());
            assert_eq!(server.inference_count(), 1);
            assert!(!server
                .seen
                .lock()
                .unwrap()
                .iter()
                .any(|(p, _)| p == "/redirected"));
        }
    }
}

#[tokio::test]
async fn cancellation_stops_inflight_request_and_prevents_later_calls() {
    for structured in [false, true] {
        let server = Server::start(999, "trial-model").await;
        let dir = Temp::new();
        let cancel = CancellationToken::new();
        let provider = provider_mode(structured, dir.config(&server.endpoint), cancel.clone())
            .await
            .unwrap();
        let request = request();
        let completion = provider.complete(&request, None);
        let canceller = async {
            while server.inference_count() == 0 {
                tokio::task::yield_now().await;
            }
            cancel.cancel();
        };
        let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            tokio::join!(completion, canceller)
        })
        .await
        .unwrap();
        assert!(result.unwrap_err().to_string().contains("cancel"));
        assert!(provider.complete(&request, None).await.is_err());
        assert_eq!(server.inference_count(), 1);
    }
}

#[tokio::test]
async fn startup_mismatch_releases_hold_and_cancellation_never_waits_for_switch() {
    for structured in [false, true] {
        let server = Server::start(200, "trial-model").await;
        let dir = Temp::new();
        *server.resident.lock().unwrap() = "other".into();
        assert!(provider_mode(
            structured,
            dir.config(&server.endpoint),
            CancellationToken::new()
        )
        .await
        .is_err());
        assert!(crate::hold::Holds::new(&dir.0)
            .live(&server.endpoint)
            .is_empty());
        assert_eq!(server.inference_count(), 0);
        let switch = crate::hold::Holds::new(&dir.0)
            .begin_switch(&server.endpoint, Some("other"), "trial-model")
            .unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(tokio::time::timeout(
            std::time::Duration::from_secs(1),
            provider_mode(structured, dir.config(&server.endpoint), cancel)
        )
        .await
        .unwrap()
        .is_err());
        drop(switch);
    }
}

#[tokio::test]
async fn mismatched_response_is_retained_for_host_audit() {
    for structured in [false, true] {
        let server = Server::start(200, "unrequested-model").await;
        let dir = Temp::new();
        let provider = provider_mode(
            structured,
            dir.config(&server.endpoint),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        let error = provider.complete(&request(), None).await.unwrap_err();
        let rejected = error.downcast_ref::<ReturnedModelMismatch>().unwrap();
        assert_eq!(rejected.response.model, "unrequested-model");
        assert_eq!(rejected.response.message.text(), "done");
        assert_eq!(rejected.response.usage.output_tokens, 2);
    }
}
