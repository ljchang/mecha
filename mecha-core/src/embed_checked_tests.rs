use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn server(
    status: u16,
    extra: String,
    body: String,
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            seen.fetch_add(1, Ordering::SeqCst);
            let mut buf = [0; 65536];
            let _ = socket.read(&mut buf).await;
            let response = format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}", body.len());
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    (url, count, task)
}

#[tokio::test]
async fn redirects_never_forward_embedding_text() {
    for status in [307, 308] {
        let (spy, seen, spy_task) = server(
            200,
            String::new(),
            r#"{"data":[{"embedding":[1,2]}]}"#.into(),
        )
        .await;
        let (url, _, task) =
            server(status, format!("Location: {spy}/stolen\r\n"), String::new()).await;
        let result = Embedder::new(&url)
            .unwrap()
            .query("synthetic private sentinel")
            .await;
        task.abort();
        spy_task.abort();
        assert_eq!(
            seen.load(Ordering::SeqCst),
            0,
            "redirect leaked the request"
        );
        assert!(result.is_err());
    }
}

fn spec() -> EncodingProfileSpec {
    EncodingProfileSpec {
        expected_alias: "synthetic-model".into(),
        artifact_revision: "host-revision-1".into(),
        dimensions: 2,
        pooling: "last".into(),
        normalization: "server-l2".into(),
        query_instruction: "find passages".into(),
        query_template: "Instruct: {instruction}\nQuery: {text}".into(),
        preprocessing_version: "identity-v1".into(),
        representation_version: "original-fields-v1".into(),
    }
}
fn profile() -> EncodingProfile {
    EncodingProfile::new(spec()).unwrap()
}
fn inputs(n: usize) -> Vec<EmbeddingInput> {
    (0..n)
        .map(|i| EmbeddingInput {
            text: format!("synthetic {i}"),
            task: Task::Passage,
        })
        .collect()
}
fn reply(data: serde_json::Value) -> String {
    serde_json::json!({"model":"synthetic-model", "data":data}).to_string()
}

#[test]
fn profile_identity_covers_every_component() {
    let baseline = profile().id().to_string();
    for field in 0..9 {
        let mut s = spec();
        match field {
            0 => s.expected_alias.push('x'),
            1 => s.artifact_revision.push('x'),
            2 => s.dimensions += 1,
            3 => s.pooling.push('x'),
            4 => s.normalization.push('x'),
            5 => s.query_instruction.push('x'),
            6 => s.query_template.push('x'),
            7 => s.preprocessing_version.push('x'),
            8 => s.representation_version.push('x'),
            _ => unreachable!(),
        }
        assert_ne!(EncodingProfile::new(s).unwrap().id(), baseline);
    }
    assert_eq!(profile().id(), baseline);
    for dimensions in [0, 8193] {
        let mut s = spec();
        s.dimensions = dimensions;
        assert!(EncodingProfile::new(s).is_err());
    }
    for template in ["{text}", "{instruction}{text}{text}", "{instruction}"] {
        let mut s = spec();
        s.query_template = template.into();
        assert!(EncodingProfile::new(s).is_err());
    }
}

#[tokio::test]
async fn checked_reorders_and_refuses_malformed_vectors() {
    let valid =
        reply(serde_json::json!([{"index":1,"embedding":[3,4]},{"index":0,"embedding":[1,2]}]));
    let (url, _, task) = server(200, String::new(), valid.clone()).await;
    let batch = Embedder::new(&url)
        .unwrap()
        .embed_checked(&inputs(2), &profile())
        .await
        .unwrap();
    assert_eq!(batch.vectors, vec![vec![1., 2.], vec![3., 4.]]);
    assert_eq!(batch.profile_id, profile().id());
    assert_eq!(batch.server_model.as_deref(), Some("synthetic-model"));
    assert_eq!(batch.attempts, 1);
    task.abort();
    let valid: serde_json::Value = serde_json::from_str(&valid).unwrap();
    for case in 0..10 {
        let mut bad = valid.clone();
        match case {
            0 => bad["data"][0]["index"] = 0.into(),
            1 => {
                bad["data"][0].as_object_mut().unwrap().remove("index");
            }
            2 => bad["data"][0]["index"] = 0.5.into(),
            3 => bad["model"] = "wrong".into(),
            4 => {
                bad.as_object_mut().unwrap().remove("model");
            }
            5 => bad["data"][0]["embedding"] = serde_json::json!([1, 2, 3]),
            6 => bad["data"][0]["embedding"] = serde_json::json!([0, 0]),
            7 => bad["data"][0]["embedding"] = serde_json::json!([1e40, 1]),
            8 => bad["data"][0]["embedding"] = serde_json::json!(["1", 2]),
            9 => bad["data"][0]["index"] = 2.into(),
            _ => unreachable!(),
        }
        let (url, _, task) = server(200, String::new(), bad.to_string()).await;
        assert!(
            Embedder::new(&url)
                .unwrap()
                .embed_checked(&inputs(2), &profile())
                .await
                .is_err(),
            "case {case}"
        );
        task.abort();
    }
}

#[tokio::test]
async fn checked_validates_every_input_before_sending() {
    let (url, seen, task) = server(200, String::new(), String::new()).await;
    let client = Embedder::new(&url).unwrap();
    let mut batch = inputs(17);
    batch[16].text = "é".repeat(8001);
    assert!(client.embed_checked(&batch, &profile()).await.is_err());
    batch = inputs(1);
    batch[0].task = Task::Query;
    batch[0].text = "a".repeat(8000);
    assert!(client.embed_checked(&batch, &profile()).await.is_err());
    batch[0].task = Task::Recall;
    batch[0].text = "short".into();
    assert!(client.embed_checked(&batch, &profile()).await.is_err());
    batch = (0..16)
        .map(|_| EmbeddingInput {
            text: "\u{0001}".repeat(8000),
            task: Task::Passage,
        })
        .collect();
    assert!(client.embed_checked(&batch, &profile()).await.is_err());
    assert!(client
        .embed_checked(&inputs(1025), &profile())
        .await
        .is_err());
    assert_eq!(seen.load(Ordering::SeqCst), 0);
    task.abort();
    for url in [
        "http://localhost:1",
        "https://127.0.0.1:1",
        "http://user@127.0.0.1:1",
        "http://127.0.0.1:1?q=x",
        "http://127.0.0.1:1#x",
    ] {
        if let Ok(client) = Embedder::new(url) {
            assert!(client.embed_checked(&inputs(1), &profile()).await.is_err());
        }
    }
}

#[tokio::test]
async fn checked_errors_never_echo_server_content() {
    for (status, body) in [
        (500, "synthetic-secret-echo".into()),
        (200, "synthetic-secret-echo".into()),
        (200, "x".repeat(8_388_609)),
    ] {
        let (url, _, task) = server(status, String::new(), body).await;
        let err = Embedder::new(&url)
            .unwrap()
            .embed_checked(&inputs(1), &profile())
            .await
            .unwrap_err();
        assert!(!format!("{err:#}").contains("synthetic-secret-echo"));
        assert!(err.to_string().len() < 200);
        task.abort();
    }
}

#[tokio::test]
async fn environment_proxy_is_ignored() {
    const CHILD: &str = "MECHA_EMBED_PROXY_TEST_CHILD";
    if let Ok(url) = std::env::var(CHILD) {
        Embedder::new(&url)
            .unwrap()
            .query("synthetic proxy text")
            .await
            .unwrap();
        return;
    }
    let body = r#"{"data":[{"embedding":[1,2]}]}"#.to_string();
    let (url, direct, task) = server(200, String::new(), body.clone()).await;
    let (proxy, proxied, spy) = server(200, String::new(), body).await;
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "embed::checked_tests::environment_proxy_is_ignored",
            "--nocapture",
        ])
        .env(CHILD, url)
        .env("NO_PROXY", "")
        .env("no_proxy", "");
    for key in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        command.env(key, &proxy);
    }
    let output = command.output().await.unwrap();
    task.abort();
    spy.abort();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(proxied.load(Ordering::SeqCst), 0);
    assert_eq!(direct.load(Ordering::SeqCst), 1);
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> serde_json::Value {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
            let length: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
            }
        }
    }
}

#[tokio::test]
async fn mixed_tasks_template_is_nonrecursive_and_batches_are_bounded() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let worker = tokio::spawn(async move {
        let mut requests = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let input = read_request(&mut socket).await;
            let entries = input["input"].as_array().unwrap();
            assert!(entries.len() <= 16);
            let body = reply(serde_json::Value::Array(
                entries
                    .iter()
                    .enumerate()
                    .map(|(i, _)| serde_json::json!({"index":i,"embedding":[1,2]}))
                    .collect(),
            ));
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            requests.push(input);
        }
        requests
    });
    let mut s = spec();
    s.query_instruction = "find {text}".into();
    let profile = EncodingProfile::new(s).unwrap();
    let mut batch = inputs(17);
    batch[0] = EmbeddingInput {
        text: "literal {instruction} {text}".into(),
        task: Task::Query,
    };
    let result = Embedder::new(&url)
        .unwrap()
        .embed_checked(&batch, &profile)
        .await
        .unwrap();
    assert_eq!(result.vectors.len(), 17);
    assert_eq!(result.attempts, 2);
    assert!(result.request_bytes > 0 && result.response_bytes > 0);
    let requests = worker.await.unwrap();
    assert_eq!(
        requests[0]["input"][0],
        "Instruct: find {text}\nQuery: literal {instruction} {text}"
    );
    assert_eq!(requests[0]["input"][1], "synthetic 1");
    assert_eq!(requests[1]["input"][0], "synthetic 16");
}

#[tokio::test]
async fn split_failure_returns_no_partial_batch() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let worker = tokio::spawn(async move {
        let mut seen = Vec::new();
        for (status, body) in [
            (500, r#"{"error":{"message":"input is too large"}}"#.into()),
            (
                200,
                reply(serde_json::json!([{"index":0,"embedding":[1,2]}])),
            ),
            (
                200,
                r#"{"model":"wrong","data":[{"index":0,"embedding":[3,4]}]}"#.into(),
            ),
        ] {
            let (mut socket, _) = listener.accept().await.unwrap();
            seen.push(read_request(&mut socket).await["input"].clone());
            socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        seen
    });
    assert!(Embedder::new(&url)
        .unwrap()
        .embed_checked(&inputs(2), &profile())
        .await
        .is_err());
    assert_eq!(
        worker.await.unwrap(),
        vec![
            serde_json::json!(["synthetic 0", "synthetic 1"]),
            serde_json::json!(["synthetic 0"]),
            serde_json::json!(["synthetic 1"])
        ]
    );
}

#[tokio::test]
async fn checked_streamed_response_cap_without_content_length() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let worker = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let block = vec![b' '; 65536];
        for _ in 0..129 {
            if socket.write_all(&block).await.is_err() {
                break;
            }
        }
    });
    let error = Embedder::new(&url)
        .unwrap()
        .embed_checked(&inputs(1), &profile())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("response exceeds byte limit"));
    worker.await.unwrap();
}

#[tokio::test]
async fn attempt_and_deadline_budgets_cover_the_whole_operation() {
    let (url, count, task) = server(500, String::new(), "input is too large".into()).await;
    let client = Embedder::new(&url).unwrap();
    let error = client
        .checked_with_budget(&inputs(16), &profile(), CHECKED_DURATION, 2)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("attempt budget exhausted"));
    assert_eq!(count.load(Ordering::SeqCst), 2);
    task.abort();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let worker = tokio::spawn(async move {
        for (status, body) in [
            (500, "input is too large".to_string()),
            (
                200,
                reply(serde_json::json!([{"index":0,"embedding":[1,2]}])),
            ),
        ] {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            tokio::time::sleep(Duration::from_millis(80)).await;
            let response = format!(
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    let error = Embedder::new(&url)
        .unwrap()
        .checked_with_budget(
            &inputs(2),
            &profile(),
            Duration::from_millis(140),
            CHECKED_ATTEMPTS,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("operation deadline exceeded"));
    worker.abort();
}
