//! Actual binary and local, scripted provider qualification for ordinary memory.
#![cfg(feature = "memory-prototype")]

#[test]
fn ordinary_memory_cli_exposes_explicit_run_and_host_commands() {
    for (args, expected) in [
        (&["run", "--help"][..], "--episodic-memory"),
        (&["memory-session", "--help"][..], "--request"),
        (&["memory-session", "--help"][..], "init"),
        (&["memory-session", "--help"][..], "retry"),
        (&["memory-session", "--help"][..], "forget"),
    ] {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_mecha"))
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(expected),
            "{args:?} must expose {expected}"
        );
    }
}

use axum::{
    body::{Body, Bytes},
    extract::State,
    http::{Method, StatusCode, Uri},
    response::Response,
};
use mecha_core::memory_session::SessionMemoryRequest;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Output, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{process::Command, sync::Notify};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(35);
const OBSERVATION: &str = "marigold 7NCQ41";
const CLAIM: &str = "The fictional telescope alignment was independently verified.";

type Respond = Box<dyn FnOnce(&Value) -> Value + Send>;

enum Step {
    Reply(Value),
    Inspect(Respond),
    Wait(Arc<Notify>, Arc<Notify>, Value),
    Fatal,
}

#[derive(Clone, Default)]
struct Wire {
    inner: Arc<Mutex<WireState>>,
}

#[derive(Default)]
struct WireState {
    pending: VecDeque<Step>,
    requests: Vec<Value>,
    responses: Vec<Value>,
}

impl Wire {
    fn script(&self, steps: impl IntoIterator<Item = Step>) {
        let mut wire = self.inner.lock().unwrap();
        assert!(wire.pending.is_empty());
        wire.pending.extend(steps);
    }

    fn requests(&self) -> Vec<Value> {
        self.inner.lock().unwrap().requests.clone()
    }

    async fn serve(State(wire): State<Self>, method: Method, uri: Uri, body: Bytes) -> Response {
        let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let step = {
            let mut state = wire.inner.lock().unwrap();
            state
                .requests
                .push(json!({"method": method.as_str(), "uri": uri.to_string(), "body": value}));
            if method == Method::POST && uri.path() == "/v1/chat/completions" {
                state.pending.pop_front()
            } else {
                None
            }
        };
        let reply = match step {
            Some(Step::Reply(reply)) => reply,
            Some(Step::Inspect(reply)) => reply(&value),
            Some(Step::Wait(entered, release, reply)) => {
                entered.notify_one();
                release.notified().await;
                reply
            }
            Some(Step::Fatal) => {
                return Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Body::from("scripted invalid request"))
                    .unwrap();
            }
            None if method == Method::GET && uri.path() == "/props" => json!({"role":"router"}),
            None if method == Method::GET && uri.path() == "/models" => {
                json!({"data":[{"id":"fixture","status":{"value":"loaded"}}]})
            }
            None => {
                return Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Body::empty())
                    .unwrap();
            }
        };
        let (kind, raw) = if value["stream"] == true {
            let message = &reply["choices"][0]["message"];
            let mut delta = message.clone();
            if let Some(calls) = delta["tool_calls"].as_array_mut() {
                for (index, call) in calls.iter_mut().enumerate() {
                    call["index"] = json!(index);
                }
            }
            let chunk = json!({
                "model": "fixture",
                "choices": [{"index": 0, "delta": delta, "finish_reason": reply["choices"][0]["finish_reason"]}],
                "usage": reply["usage"]
            });
            (
                "text/event-stream",
                format!("data: {chunk}\n\ndata: [DONE]\n\n"),
            )
        } else {
            ("application/json", reply.to_string())
        };
        wire.inner
            .lock()
            .unwrap()
            .responses
            .push(json!({"content_type": kind, "body": raw}));
        Response::builder()
            .header("content-type", kind)
            .body(Body::from(raw))
            .unwrap()
    }
}

struct Fixture {
    root: PathBuf,
    request: SessionMemoryRequest,
    wire: Wire,
    endpoint: String,
    server: tokio::task::JoinHandle<()>,
    label: &'static str,
}

impl Fixture {
    async fn new(label: &'static str) -> Self {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("mecha-ordinary-cli-{}", uuid::Uuid::new_v4()));
        mecha_core::create_private_dir(&root).unwrap();
        for dir in ["home", "mecha", "db", "keys", "work-a", "work-b"] {
            mecha_core::create_private_dir(&root.join(dir)).unwrap();
        }
        let request = SessionMemoryRequest {
            protocol: "ordinary-session-memory-v1".into(),
            db: root.join("db/ordinary.memory-prototype.sqlite"),
            key_file: root.join("keys/ordinary.key"),
            authority: root.join("authority"),
            domain: "ordinary-fixture".into(),
            account: "ordinary-fixture".into(),
            scopes: ["test-scope".into()].into(),
        };
        let path = root.join("request.json");
        fs::write(&path, serde_json::to_vec(&request).unwrap()).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();

        let wire = Wire::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = axum::Router::new()
            .fallback(Wire::serve)
            .with_state(wire.clone());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let fixture = Self {
            root,
            request,
            wire,
            endpoint,
            server,
            label,
        };
        fixture.config("openai-compatible", "", "");
        fixture
    }

    fn config(&self, kind: &str, provider: &str, extra: &str) {
        fs::write(
            self.root.join("mecha/config.toml"),
            format!(
                r#"
default_provider = "fixture"
[providers.fixture]
kind = "{kind}"
base_url = "{}"
model = "fixture"
max_retries = 0
{provider}
[tools]
permission_mode = "allow"
enabled = ["fs_read", "fs_write", "fs_list", "goal_context"]
{extra}
"#,
                self.endpoint
            ),
        )
        .unwrap();
    }

    fn manifest(&self) -> PathBuf {
        self.root.join("request.json")
    }

    fn command(&self, work: &str) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_mecha"));
        cmd.env_clear()
            .env("HOME", self.root.join("home"))
            .env("MECHA_HOME", self.root.join("mecha"))
            .current_dir(self.root.join(work))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        cmd
    }

    fn run(&self, work: &str, goal: &str, prompt: &str) -> Command {
        let mut cmd = self.command(work);
        cmd.args(["run", "--json", "--episodic-memory"])
            .arg(self.manifest())
            .args([
                "--goal",
                goal,
                "--no-mcp",
                "--no-code-tool",
                "--no-grants",
                "--no-learned-rules",
                "--no-past-appraisals",
                "--no-success-examples",
                "--tool",
                "fs_read",
                "--tool",
                "fs_write",
                "--tool",
                "goal_context",
                prompt,
            ]);
        cmd
    }

    async fn host(&self, args: &[&str]) -> Output {
        let mut cmd = self.command("home");
        cmd.args(["memory-session", "--request"])
            .arg(self.manifest())
            .args(args);
        let label = if matches!(args.first().copied(), Some("sources" | "learn")) {
            static HOST_CALL: std::sync::atomic::AtomicUsize =
                std::sync::atomic::AtomicUsize::new(0);
            format!(
                "{}-{}",
                args[0],
                HOST_CALL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )
        } else {
            args.join("-")
        };
        self.output(cmd, &label).await
    }

    async fn init(&self) {
        assert_eq!(
            success(&self.host(&["init"]).await),
            json!({"protocol": "ordinary-session-memory-v1", "status": "initialized"})
        );
    }

    async fn output(&self, mut cmd: Command, label: &str) -> Output {
        let output = tokio::time::timeout(REQUEST_TIMEOUT, cmd.output())
            .await
            .expect("fixture CLI did not exit")
            .unwrap();
        self.save_output(label, &output);
        output
    }

    fn save_output(&self, label: &str, output: &Output) {
        self.save(&format!("{label}.stdout"), &output.stdout);
        self.save(&format!("{label}.stderr"), &output.stderr);
    }

    fn transcript(&self, report: &Value) -> (PathBuf, Vec<Value>) {
        let id = report["session"].as_str().unwrap();
        let path = self.root.join("mecha/sessions").join(format!("{id}.jsonl"));
        let text = fs::read_to_string(&path).unwrap();
        self.save(&format!("{id}.jsonl"), text.as_bytes());
        (
            path,
            text.lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect(),
        )
    }

    fn seal(&self, report: &Value) -> Value {
        let run = run_id(report);
        let raw = fs::read(self.request.authority.join(format!("{run}.seal"))).unwrap();
        self.save(&format!("{run}.seal.json"), &raw);
        serde_json::from_slice(&raw).unwrap()
    }

    fn sqlite(&self) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(&self.request.db).unwrap();
        let key = fs::read_to_string(&self.request.key_file).unwrap();
        db.execute_batch(&format!("PRAGMA key=\"x'{}'\"", key.trim()))
            .unwrap();
        db
    }

    fn active_receipts(&self) -> i64 {
        self.sqlite()
            .query_row(
                "SELECT count(*) FROM host_capture_receipt WHERE state = 'active'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn save(&self, name: &str, bytes: &[u8]) {
        if let Some(dir) = std::env::var_os("MECHA_ORDINARY_CLI_EVIDENCE_DIR") {
            let dest = PathBuf::from(dir).join(self.label);
            fs::create_dir_all(&dest).unwrap();
            fs::write(dest.join(name), bytes).unwrap();
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        let state = self.wire.inner.lock().unwrap();
        self.save(
            "wire.json",
            &serde_json::to_vec_pretty(&json!({
                "requests": state.requests,
                "responses": state.responses,
            }))
            .unwrap(),
        );
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn success(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "status {}\nstdout {}\nstderr {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn failure(out: &Output, expected: &str) {
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(expected),
        "expected {expected:?}, stderr {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn answer(text: &str) -> Value {
    completion(json!({"role": "assistant", "content": text}), "stop")
}

fn tool(id: &str, name: &str, args: Value) -> Value {
    completion(
        json!({
            "role": "assistant", "content": null,
            "tool_calls": [{
                "id": id, "type": "function",
                "function": {"name": name, "arguments": args.to_string()}
            }]
        }),
        "tool_calls",
    )
}

fn completion(message: Value, finish_reason: &str) -> Value {
    json!({
        "model": "fixture",
        "choices": [{"index": 0, "message": message, "finish_reason": finish_reason}],
        "usage": {"prompt_tokens": 14, "completion_tokens": 7, "total_tokens": 21}
    })
}

fn write(id: &str, path: &str, content: &str) -> Value {
    tool(id, "fs_write", json!({"path": path, "content": content}))
}

fn recall() -> Value {
    tool(
        "recall",
        "goal_context",
        json!({"memory": {"query": "marigold"}}),
    )
}

fn run_id(report: &Value) -> &str {
    report["memory"]["run_id"].as_str().unwrap()
}

fn assert_report(report: &Value, state: &str) {
    assert_eq!(report["memory"]["status"], state, "{report:#}");
    assert_eq!(report["memory"]["protocol"], "ordinary-session-memory-v1");
    assert_eq!(report["memory"]["semantic_assessment"], "not_evaluated");
}

fn last_tool(body: &Value, id: &str) -> String {
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|message| message["role"] == "tool" && message["tool_call_id"] == id)
        .and_then(|message| message["content"].as_str())
        .unwrap_or("")
        .into()
}

fn prior_tool_result(delivered: &str) -> Option<Value> {
    // A recall whose every record is local is delivered bare; one holding
    // anything from outside, or of unknown origin, arrives wrapped.
    let payload = match delivered.split_once("\n---\n") {
        Some((_, wrapped)) => wrapped.strip_suffix("\n</untrusted-content>")?,
        None => delivered,
    };
    let body: Value = serde_json::from_str(payload).ok()?;
    body["memory"]["records"]
        .as_array()?
        .iter()
        .map(|entry| &entry["record"]["data"])
        .find(|data| {
            data["source"]["connector"] == "host-session-field-v1"
                && data["source"]["id"]
                    .as_str()
                    .is_some_and(|id| id.ends_with("/content/0/content"))
                && data["text"]
                    .as_str()
                    .is_some_and(|text| text.starts_with("marigold "))
        })
        .cloned()
}

fn assert_artifact(f: &Fixture, work: &str, name: &str, expected: &str) {
    let path = f.root.join(work).join(name);
    let bytes = fs::read(&path).unwrap();
    assert_eq!(bytes, expected.as_bytes());
    f.save(&format!("{work}-{name}"), &bytes);
}

fn record<'a>(transcript: &'a [Value], kind: &str) -> &'a Value {
    transcript.iter().find(|row| row["record"] == kind).unwrap()
}

fn wire_specs(body: &Value) -> Vec<&str> {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect()
}

fn initial() -> [Step; 3] {
    [
        Step::Reply(write("first-write", "observed.txt", OBSERVATION)),
        Step::Reply(tool(
            "first-read",
            "fs_read",
            json!({"path": "observed.txt"}),
        )),
        Step::Reply(answer(CLAIM)),
    ]
}

#[tokio::test]
async fn separate_ordinary_processes_recall_prior_attributed_tool_result() {
    let f = Fixture::new("two-process").await;
    // Host operations don't consult either config layer.
    fs::write(f.root.join("mecha/config.toml"), "invalid = [").unwrap();
    fs::write(f.root.join("home/mecha.toml"), "invalid = [").unwrap();
    f.init().await;
    let mut reinit = f.command("home");
    reinit
        .args(["memory-session", "--request"])
        .arg(f.manifest())
        .arg("init");
    failure(
        &f.output(reinit, "init-existing").await,
        "requires new paths",
    );
    f.config("openai-compatible", "", "");

    f.wire.script(initial());
    let mut cmd = f.run("work-a", "project:source", "Record a marigold fixture.");
    cmd.arg("--no-thinking");
    let a = success(&f.output(cmd, "run-a").await);
    assert_report(&a, "retained");
    assert_eq!(a["text"], CLAIM);
    assert_artifact(&f, "work-a", "observed.txt", OBSERVATION);
    let (_, transcript) = f.transcript(&a);
    assert_eq!(record(&transcript, "meta")["kind"], "run");
    assert_eq!(
        record(&transcript, "config")["tools"],
        json!(["fs_read", "fs_write", "goal_context"])
    );
    assert_eq!(record(&transcript, "outcome")["stop_cause"], "completed");
    let payload = f.seal(&a)["payload"].clone();
    assert_eq!(payload["criterion"]["goal"], "project:source");
    assert_eq!(
        payload["artifact"],
        json!({"artifact_digest": null, "verdict": "Unavailable"})
    );
    assert!(payload["originals"].as_array().unwrap().iter().any(|item| {
        item["kind"] == "ToolResult" && item["text"].as_str().unwrap().contains(OBSERVATION)
    }));
    assert!(payload["originals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["kind"] == "AssistantText" && item["text"] == CLAIM }));

    f.wire.script([
        Step::Reply(recall()),
        Step::Inspect(Box::new(|body| {
            // The scripted response depends on the delivered tool data. The
            // answer is neither in the initial prompt nor a shared workspace.
            let delivered = last_tool(body, "recall");
            let result = prior_tool_result(&delivered)
                .map(|item| {
                    format!(
                        "Copied the recorded reading {}",
                        item["text"].as_str().unwrap()
                    )
                })
                .unwrap_or_else(|| "MISSING PRIOR TOOL OBSERVATION".into());
            write("second-write", "copied.txt", &result)
        })),
        Step::Reply(answer("Recorded the prior reading separately.")),
    ]);
    let mut cmd = f.run(
        "work-b",
        "project:followup",
        "Look up the recorded reading and make a new note.",
    );
    cmd.arg("--no-thinking");
    let b = success(&f.output(cmd, "run-b").await);
    assert_report(&b, "retained");
    assert_ne!(a["session"], b["session"]);
    assert_ne!(run_id(&a), run_id(&b));
    assert_artifact(
        &f,
        "work-b",
        "copied.txt",
        &format!("Copied the recorded reading {OBSERVATION}"),
    );
    assert!(!f.root.join("work-b/observed.txt").exists());
    let (_, transcript) = f.transcript(&b);
    // Everything the first run kept was the owner's prompt, the model's own
    // words and local file results, and the store recorded each as such. So
    // recalling it brings private data into the second run and nothing from
    // outside: the interlock is not armed by memory alone.
    assert_eq!(record(&transcript, "taint")["private"], true);
    assert_eq!(record(&transcript, "taint")["untrusted"], false);
    assert_eq!(record(&transcript, "outcome")["stop_cause"], "completed");
    assert!(payload["originals"]
        .as_array()
        .unwrap()
        .iter()
        .all(|item| item["external"] == false));

    let wire = f.wire.requests();
    assert_eq!(wire.len(), 6);
    assert_eq!(
        wire_specs(&wire[3]["body"]),
        ["fs_read", "fs_write", "goal_context"]
    );
    assert!(!wire[3]["body"].to_string().contains(OBSERVATION));
    let delivered = last_tool(&wire[4]["body"], "recall");
    assert!(!delivered.contains("<untrusted-content"));
    assert!(delivered.contains(OBSERVATION));
    assert!(delivered.contains(a["session"].as_str().unwrap()));
    let reading = prior_tool_result(&delivered).expect("no attributed tool result on the wire");
    let delivered: Value = serde_json::from_str(&delivered).unwrap();
    assert!(delivered["memory"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .all(|hit| hit["untrusted_input"] == false && hit["private_data"] == true));
    assert!(reading["source"]["id"]
        .as_str()
        .unwrap()
        .starts_with(a["session"].as_str().unwrap()));
    assert_eq!(reading["scopes"], json!(["test-scope"]));

    let b_payload = f.seal(&b)["payload"].clone();
    let consulted = b_payload["consulted"].as_array().unwrap();
    assert!(!consulted.is_empty());
    assert_eq!(b_payload["criterion"]["goal"], "project:followup");
    assert!(!b_payload["originals"]
        .to_string()
        .contains("native_capture_reader_v1"));
    let db = f.sqlite();
    assert_eq!(f.active_receipts(), 2);
    for original in b["memory"]["receipt"]["originals"].as_array().unwrap() {
        for source in consulted {
            let exists: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM dependency WHERE output=?1 AND input=?2)",
                    [
                        original["revision"].as_str().unwrap(),
                        source["revision"].as_str().unwrap(),
                    ],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(
                exists,
                "missing consulted dependency for {original}: {source}"
            );
        }
    }
    let projection = &a["memory"]["receipt"]["projection"]["revision"];
    let stored: String = db
        .query_row(
            "SELECT payload FROM revision WHERE id = ?1",
            [projection.as_str().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    f.save("projection-a.json", stored.as_bytes());
    let evidence: Value = serde_json::from_str(&stored).unwrap();
    let projection: Value =
        serde_json::from_str(evidence["data"]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        projection["identity"]["check_version"],
        "ordinary-session-observation-v1"
    );
    assert_eq!(
        projection["identity"]["evaluator_version"],
        "ordinary-session-recorder-v1"
    );
    assert_eq!(projection["identity"]["session_id"], a["session"]);
    assert_eq!(projection["artifact"]["verdict"], "Unavailable");
    assert_eq!(projection["receipts"][0]["status"], "Succeeded");

    // Forget must purge the consulted descendants even from a host-only
    // invocation after the original workspace disappeared.
    fs::remove_dir_all(f.root.join("work-a")).unwrap();
    fs::write(f.root.join("mecha/config.toml"), "invalid = [").unwrap();
    let forgotten = success(&f.host(&["forget", run_id(&a)]).await);
    assert!(forgotten["objects"].as_u64().unwrap() > 2);
    assert_eq!(f.active_receipts(), 0);
    let dependent = success(&f.host(&["retry", run_id(&b)]).await);
    assert_ne!(dependent["status"], "retained");
    assert_eq!(f.wire.requests().len(), 6);
}

#[tokio::test]
async fn reasoning_wire_retains_action_without_importing_reasoning() {
    const SENTINEL: &str = "PROVIDER_REASONING_SENTINEL_2474";
    for (label, no_thinking) in [
        ("thinking", false),
        ("thinking-with-no-thinking-flag", true),
    ] {
        let f = Fixture::new(label).await;
        f.init().await;
        let mut reasoning = write(
            "write-with-reasoning",
            "reasoned.txt",
            "completed fixture write",
        );
        reasoning["choices"][0]["message"]["reasoning_content"] = json!(SENTINEL);
        f.wire.script([
            Step::Reply(reasoning),
            Step::Reply(answer("The fixture write is complete.")),
        ]);
        let mut cmd = f.run("work-a", "task:reasoning", "Write the fixture.");
        if no_thinking {
            cmd.arg("--no-thinking");
        }
        let out = success(&f.output(cmd, "run").await);
        assert_report(&out, "retained");
        assert_eq!(out["text"], "The fixture write is complete.");
        assert_artifact(&f, "work-a", "reasoned.txt", "completed fixture write");
        let (_, transcript) = f.transcript(&out);
        assert_eq!(record(&transcript, "config")["thinking"], !no_thinking);
        assert_eq!(record(&transcript, "outcome")["stop_cause"], "completed");
        assert!(transcript.iter().any(|row| {
            row["content"].as_array().is_some_and(|blocks| {
                blocks
                    .iter()
                    .any(|block| block["type"] == "thinking" && block["text"] == SENTINEL)
            })
        }));
        let wire = f.wire.requests();
        assert_eq!(wire.len(), 2);
        assert_ne!(
            wire[0]["body"]["chat_template_kwargs"]["enable_thinking"],
            false
        );
        assert!(wire[1]["body"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| {
                message["role"] == "assistant" && message["reasoning_content"] == SENTINEL
            }));
        assert!(last_tool(&wire[1]["body"], "write-with-reasoning").contains("reasoned.txt"));
        let seal = f.seal(&out);
        let originals = seal["payload"]["originals"].as_array().unwrap();
        assert!(originals.iter().any(|item| {
            item["kind"] == "ActionInvocation" && item["pointer"] == "/content/1/input"
        }));
        assert!(!serde_json::to_string(&seal["payload"])
            .unwrap()
            .contains(SENTINEL));
        assert_eq!(f.active_receipts(), 1);
        let leaks: i64 = f
            .sqlite()
            .query_row(
                "SELECT count(*) FROM revision WHERE instr(payload, ?1) > 0",
                [SENTINEL],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(leaks, 0);
    }
}

#[tokio::test]
async fn sqlite_admission_failure_can_retry_capture_without_repeating_action() {
    let f = Fixture::new("retry").await;
    f.init().await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    f.wire.script([
        Step::Reply(write("write-once", "once.txt", "one completed action")),
        Step::Wait(
            entered.clone(),
            release.clone(),
            answer("The action completed."),
        ),
    ]);
    let mut cmd = f.run("work-a", "task:retry", "Write once.");
    cmd.arg("--no-thinking");
    let child = cmd.spawn().unwrap();
    tokio::time::timeout(REQUEST_TIMEOUT, entered.notified())
        .await
        .unwrap();
    assert_artifact(&f, "work-a", "once.txt", "one completed action");
    let lock = f.sqlite();
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    release.notify_one();
    let out = tokio::time::timeout(REQUEST_TIMEOUT, child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    f.save_output("run", &out);
    let pending = success(&out);
    assert_report(&pending, "pending");
    assert_eq!(pending["memory"]["reason"], "admission_unavailable");
    assert_eq!(pending["text"], "The action completed.");
    assert_eq!(f.active_receipts(), 0);
    let (_, transcript) = f.transcript(&pending);
    assert_eq!(record(&transcript, "outcome")["stop_cause"], "completed");
    f.seal(&pending);
    lock.execute_batch("ROLLBACK").unwrap();
    fs::write(f.root.join("work-a/once.txt"), "changed after action").unwrap();
    fs::write(f.root.join("mecha/config.toml"), "invalid = [").unwrap();
    let retained = success(&f.host(&["retry", run_id(&pending)]).await);
    assert_eq!(retained["status"], "retained");
    assert_eq!(retained["semantic_assessment"], "not_evaluated");
    assert_artifact(&f, "work-a", "once.txt", "changed after action");
    fs::remove_dir_all(f.root.join("work-a")).unwrap();
    assert_eq!(
        success(&f.host(&["retry", run_id(&pending)]).await)["receipt"],
        retained["receipt"]
    );
    assert_eq!(f.active_receipts(), 1);
    assert_eq!(f.wire.requests().len(), 2);
}

#[tokio::test]
async fn withdrawal_after_wire_delivery_withholds_dependent_retention() {
    let f = Fixture::new("withdrawal").await;
    f.init().await;
    f.wire.script(initial());
    let mut cmd = f.run("work-a", "task:source", "Record the marigold reading.");
    cmd.arg("--no-thinking");
    let a = success(&f.output(cmd, "run-a").await);
    assert_report(&a, "retained");
    f.transcript(&a);

    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    f.wire.script([
        Step::Reply(recall()),
        Step::Wait(
            entered.clone(),
            release.clone(),
            write(
                "dependent-write",
                "dependent.txt",
                "action after prior delivery",
            ),
        ),
        Step::Reply(answer("The dependent action completed.")),
    ]);
    let mut cmd = f.run(
        "work-b",
        "task:dependent",
        "Check the prior reading and record that you did.",
    );
    cmd.arg("--no-thinking");
    let child = cmd.spawn().unwrap();
    tokio::time::timeout(REQUEST_TIMEOUT, entered.notified())
        .await
        .unwrap();
    assert!(last_tool(&f.wire.requests()[4]["body"], "recall").contains(OBSERVATION));
    assert!(
        success(&f.host(&["forget", run_id(&a)]).await)["objects"]
            .as_u64()
            .unwrap()
            > 0
    );
    release.notify_one();
    let out = tokio::time::timeout(REQUEST_TIMEOUT, child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    f.save_output("run-b", &out);
    let b = success(&out);
    assert_report(&b, "unavailable");
    assert_eq!(b["memory"]["reason"], "recording_unavailable");
    assert!(!f
        .request
        .authority
        .join(format!("{}.seal", run_id(&b)))
        .exists());
    assert_eq!(b["text"], "The dependent action completed.");
    assert_artifact(&f, "work-b", "dependent.txt", "action after prior delivery");
    let (_, transcript) = f.transcript(&b);
    assert_eq!(record(&transcript, "outcome")["stop_cause"], "completed");
    assert_eq!(f.active_receipts(), 0);
    assert_eq!(
        success(&f.host(&["retry", run_id(&b)]).await)["status"],
        "unavailable"
    );
    assert_eq!(f.wire.requests().len(), 6);
}

#[tokio::test]
async fn invalid_manifest_and_session_kind_fail_before_any_provider_startup() {
    let f = Fixture::new("preflight").await;
    f.init().await;
    f.config("local", "", "");
    let work = "work-a";
    let pinned = || {
        let mut cmd = f.run(work, "task:guard", "No dispatch.");
        cmd.args(["--provider", "fixture"]);
        cmd
    };
    // The experiment mark set by hand is refused like the others
    // (`memory_profile::open_run_profile`): this home is not a trial's.
    for kind in ["test", "experiment", "driven"] {
        let mut cmd = pinned();
        cmd.env("MECHA_SESSION_KIND", kind);
        failure(&f.output(cmd, kind).await, "session-kind override");
    }
    // Nor does a reference beside the mark admit it: both are environment,
    // and whoever set one can set the other.
    {
        let mut cmd = pinned();
        cmd.env("MECHA_SESSION_KIND", "experiment").env(
            "MECHA_EXPERIMENT_REF",
            r#"{"exp_id":"x","trial_id":"x","arm":"x","actor":"x","task":"x","repetition":1,"condition_hash":"x"}"#,
        );
        failure(&f.output(cmd, "forged-ref").await, "session-kind override");
    }

    // What the run is configured with (a project file, a hook, a fallback,
    // inbound messages, other tools) is no longer a condition of
    // remembering it; `an_ordinary_configuration_is_remembered` runs one.
    let path = f.manifest();
    let raw = fs::read(&path).unwrap();
    fs::write(
        &path,
        b"{\"protocol\":\"ordinary-session-memory-v1\",\"protocol\":\"duplicate\"}",
    )
    .unwrap();
    failure(&f.output(pinned(), "malformed").await, "duplicate");
    fs::write(&path, &raw).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    failure(
        &f.output(pinned(), "public-request").await,
        "must be private",
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let hardlink = f.root.join("hardlink.json");
    fs::hard_link(&path, &hardlink).unwrap();
    failure(
        &f.output(pinned(), "aliased-request").await,
        "aliases refused",
    );
    fs::remove_file(hardlink).unwrap();

    let inside = f.root.join(work).join("request.json");
    fs::rename(&path, &inside).unwrap();
    let mut cmd = f.command(work);
    cmd.args(["run", "--json", "--episodic-memory"])
        .arg(&inside)
        .args([
            "--provider",
            "fixture",
            "--goal",
            "task:unsafe",
            "--no-mcp",
            "--no-code-tool",
            "--no-grants",
            "--tool",
            "goal_context",
            "Do not dispatch.",
        ]);
    failure(
        &f.output(cmd, "workspace-request").await,
        "overlaps workspace",
    );
    fs::rename(inside, &path).unwrap();
    fs::rename(&path, f.root.join("moved-request.json")).unwrap();
    std::os::unix::fs::symlink(f.root.join("moved-request.json"), &path).unwrap();
    assert!(!f.output(pinned(), "symlink-request").await.status.success());
    fs::remove_file(&path).unwrap();
    fs::rename(f.root.join("moved-request.json"), &path).unwrap();

    fs::set_permissions(&f.request.key_file, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!f.output(pinned(), "public-key").await.status.success());
    fs::set_permissions(&f.request.key_file, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_file(&f.request.key_file).unwrap();
    assert!(!f.output(pinned(), "missing-key").await.status.success());
    assert!(!f.host(&["retry", &"a".repeat(64)]).await.status.success());
    let plain = rusqlite::Connection::open(&f.request.db).unwrap();
    assert!(plain
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row
            .get::<_, i64>(0))
        .is_err());
    assert!(!f.root.join("mecha/sessions").exists());
    assert!(f.wire.requests().is_empty());
}

#[tokio::test]
async fn read_only_and_no_grants_are_enforced_by_native_dispatch() {
    let f = Fixture::new("permissions").await;
    f.init().await;
    f.wire.script([
        Step::Reply(write("read-only-write", "blocked.txt", "must not write")),
        Step::Reply(answer("The write was blocked.")),
    ]);
    let mut cmd = f.run("work-a", "task:read-only", "Do a read-only run.");
    cmd.args(["--no-thinking", "--read-only"]);
    let denied = success(&f.output(cmd, "read-only").await);
    assert_report(&denied, "retained");
    assert!(!f.root.join("work-a/blocked.txt").exists());
    let result = last_tool(&f.wire.requests()[1]["body"], "read-only-write");
    assert!(result.contains("read-only"), "{result:?}");
    let seal = f.seal(&denied);
    assert_eq!(seal["payload"]["receipts"][0]["status"], "Failed");
    let (_, transcript) = f.transcript(&denied);
    assert_eq!(record(&transcript, "outcome")["stop_cause"], "completed");

    let outside = f.root.join("work-b/never.txt");
    f.config(
        "openai-compatible",
        "",
        &format!(
            "[[grant]]\npath='{}'\naccess='write'",
            outside.parent().unwrap().display()
        ),
    );
    f.wire.script([
        Step::Reply(write(
            "granted-write",
            outside.to_str().unwrap(),
            "must not escape",
        )),
        Step::Reply(answer("The external write was blocked.")),
    ]);
    let mut cmd = f.run("work-a", "task:grants", "Stay in the workspace.");
    cmd.args(["--no-thinking", "--yes"]);
    let out = success(&f.output(cmd, "no-grants").await);
    assert_report(&out, "retained");
    assert!(!outside.exists());
    let (_, transcript) = f.transcript(&out);
    assert_eq!(record(&transcript, "grants")["grants"], json!([]));
    assert_eq!(f.seal(&out)["payload"]["receipts"][0]["status"], "Failed");
    assert!(last_tool(&f.wire.requests()[3]["body"], "granted-write").contains("workspace"));
}

#[tokio::test]
async fn cancellation_and_budget_keep_native_outcomes_and_remember_the_partial_run() {
    let f = Fixture::new("limits").await;
    f.init().await;
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    f.wire.script([
        Step::Reply(write(
            "before-cancel",
            "before-cancel.txt",
            "kept before interrupt",
        )),
        Step::Wait(entered.clone(), release, answer("unreleased fixture")),
    ]);
    let mut cmd = f.run("work-a", "task:cancel", "Write and wait.");
    cmd.arg("--no-thinking");
    let child = cmd.spawn().unwrap();
    tokio::time::timeout(REQUEST_TIMEOUT, entered.notified())
        .await
        .unwrap();
    // SAFETY: SIGINT to the specific fixture child while its own run waits on HTTP.
    assert_eq!(
        unsafe { libc::kill(child.id().unwrap() as i32, libc::SIGINT) },
        0
    );
    let out = tokio::time::timeout(REQUEST_TIMEOUT, child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    f.save_output("cancel", &out);
    let cancelled = success(&out);
    // A run that was stopped is remembered as far as it got.
    assert_report(&cancelled, "retained");
    assert_eq!(cancelled["stop_cause"], "stopped");
    assert_artifact(&f, "work-a", "before-cancel.txt", "kept before interrupt");
    let (_, transcript) = f.transcript(&cancelled);
    assert_eq!(record(&transcript, "outcome")["stop_cause"], "stopped");

    f.wire.script([
        Step::Reply(write(
            "before-budget",
            "before-budget.txt",
            "kept before ceiling",
        )),
        Step::Reply(answer("The budget ended the fixture.")),
    ]);
    let mut cmd = f.run("work-a", "task:budget", "Write with a turn ceiling.");
    cmd.args(["--no-thinking", "--max-turns", "1"]);
    let limited = success(&f.output(cmd, "budget").await);
    assert_report(&limited, "retained");
    assert_eq!(limited["stop_cause"], "max_turns");
    assert_artifact(&f, "work-a", "before-budget.txt", "kept before ceiling");
    let (_, transcript) = f.transcript(&limited);
    assert_eq!(record(&transcript, "outcome")["stop_cause"], "max_turns");
    assert_eq!(f.active_receipts(), 2);
}

#[tokio::test]
async fn fatal_provider_error_keeps_action_and_no_fabricated_outcome() {
    let f = Fixture::new("fatal").await;
    f.init().await;
    f.wire.script([
        Step::Reply(write(
            "before-error",
            "kept.txt",
            "kept before provider failure",
        )),
        Step::Fatal,
    ]);
    let out = f
        .output(
            f.run("work-a", "task:fatal", "Write and then respond."),
            "fatal",
        )
        .await;
    failure(&out, "scripted invalid request");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let id = stderr
        .lines()
        .find_map(|line| {
            line.strip_prefix("mecha: episodic memory unavailable (run ")
                .and_then(|rest| rest.strip_suffix(')'))
        })
        .unwrap();
    assert_eq!(id.len(), 64);
    assert_artifact(&f, "work-a", "kept.txt", "kept before provider failure");
    let path = fs::read_dir(f.root.join("mecha/sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let transcript = fs::read_to_string(path).unwrap();
    f.save("transcript.jsonl", transcript.as_bytes());
    assert!(transcript.contains("before-error"));
    assert!(transcript.contains("\"record\":\"taint\""));
    assert!(!transcript.contains("\"record\":\"outcome\""));
    let retry = success(&f.host(&["retry", id]).await);
    assert_eq!(retry["status"], "unavailable");
    assert_eq!(retry["reason"], "source_unavailable");
    assert_eq!(f.active_receipts(), 0);
    assert_eq!(f.wire.requests().len(), 2);
}

#[tokio::test]
async fn opt_out_and_synthetic_context_keep_their_existing_json_and_tool_surfaces() {
    let f = Fixture::new("opt-out").await;
    let ordinary_options = [
        "run",
        "--json",
        "--goal",
        "task:optout",
        "--no-mcp",
        "--no-code-tool",
        "--no-grants",
        "--no-learned-rules",
        "--no-past-appraisals",
        "--no-success-examples",
        "--tool",
        "goal_context",
        "Fixture question.",
    ];
    f.wire.script([Step::Reply(answer("Fixture answer."))]);
    let mut cmd = f.command("work-a");
    cmd.args(ordinary_options);
    let plain = success(&f.output(cmd, "plain").await);
    assert!(plain.get("memory").is_none());
    assert_eq!(plain["text"], "Fixture answer.");
    let (_, transcript) = f.transcript(&plain);
    assert_eq!(record(&transcript, "outcome")["stop_cause"], "completed");
    assert!(
        f.wire.requests()[0]["body"]["tools"][0]["function"]["parameters"]["properties"]
            .get("memory")
            .is_none()
    );

    use mecha_core::memory::{Capture, Memory, SourceKey};
    let path = f.root.join("db/synthetic.memory-prototype.sqlite");
    let mut memory = Memory::create(&path, "synthetic-fixture").unwrap();
    memory
        .capture(Capture {
            source: SourceKey {
                connector: "synthetic".into(),
                account: "fixture".into(),
                id: "record".into(),
            },
            text: "Marigold synthetic fixture result".into(),
            occurred_at: None,
            scopes: ["fixture-scope".into()].into(),
            transcript: None,
        })
        .unwrap();
    f.wire.script([
        Step::Reply(recall()),
        Step::Reply(answer("Fixture answer.")),
    ]);
    let mut cmd = f.command("work-b");
    cmd.args(ordinary_options)
        .args(["--memory-context"])
        .arg(path)
        .args([
            "--memory-synthetic",
            "--memory-domain",
            "synthetic-fixture",
            "--memory-account",
            "fixture",
            "--memory-scope",
            "fixture-scope",
        ]);
    let synthetic = success(&f.output(cmd, "synthetic").await);
    assert!(synthetic.get("memory").is_none());
    assert!(plain
        .as_object()
        .unwrap()
        .keys()
        .eq(synthetic.as_object().unwrap().keys()));
    assert!(last_tool(&f.wire.requests()[2]["body"], "recall")
        .contains("Marigold synthetic fixture result"));
    assert!(!f.request.authority.exists());
}

#[tokio::test]
async fn ordinary_capture_flags_cannot_be_omitted_or_combined_with_other_recorders() {
    let f = Fixture::new("flags").await;
    // A missing goal must be refused before even reading config.
    fs::write(f.root.join("mecha/config.toml"), "invalid = [").unwrap();
    // A confirmed goal is the one thing a remembered run must name.
    let required = [&["--goal", "task:flags"][..]];
    let command = || {
        let mut cmd = f.command("work-a");
        cmd.args(["run", "--episodic-memory"])
            .arg(f.manifest())
            .arg("Fixture prompt.");
        cmd
    };
    for missing in required {
        let mut cmd = command();
        for args in required.into_iter().filter(|args| *args != missing) {
            cmd.args(args);
        }
        let output = f.output(cmd, missing[0]).await;
        if missing[0] == "--goal" {
            assert_eq!(output.status.code(), Some(2));
        } else {
            assert_eq!(output.status.code(), Some(1));
            assert!(String::from_utf8_lossy(&output.stderr).contains("ordinary memory requires"));
        }
        assert!(String::from_utf8_lossy(&output.stderr).contains(missing[0]));
    }
    for conflicting in [
        &["--resume", "an-old-id"][..],
        &["--no-session"][..],
        &["--image", "a.png"][..],
        &["--mismatch-case", "case.json"][..],
        &["--memory-context", "synthetic.memory-prototype.sqlite"][..],
    ] {
        let mut cmd = command();
        for args in required {
            cmd.args(args);
        }
        cmd.args(conflicting);
        let output = f.output(cmd, conflicting[0]).await;
        assert_eq!(output.status.code(), Some(2));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("cannot be used with"), "{stderr}");
        assert!(stderr.contains(conflicting[0]), "{stderr}");
    }
    assert!(f.wire.requests().is_empty());
    assert!(!f.root.join("mecha/sessions").exists());
}

#[tokio::test]
async fn an_ordinary_configuration_is_remembered_and_its_outbox_route_still_holds() {
    let f = Fixture::new("ordinary-config").await;
    f.init().await;
    // What the closed profile refused: an outbox route over a tool the run
    // has, inbound messages on, a project file, and no explicit tool list.
    f.config(
        "local",
        "",
        "[outbox]\ntools=['fs_write']\n[messages]\nenabled=true",
    );
    fs::write(
        f.root.join("work-a").join("mecha.toml"),
        "[tools]\npermission_mode = 'allow'\n",
    )
    .unwrap();
    f.wire.script([
        Step::Reply(write("staged", "staged.txt", "held for review")),
        Step::Reply(answer("The note is staged for review.")),
    ]);
    let mut cmd = f.command("work-a");
    cmd.args(["run", "--json", "--episodic-memory"])
        .arg(f.manifest())
        .args([
            "--goal",
            "task:ordinary",
            "--no-code-tool",
            "--no-learned-rules",
            "--no-past-appraisals",
            "--no-success-examples",
            "--provider",
            "fixture",
            "--no-thinking",
            "Stage a note.",
        ]);
    let out = success(&f.output(cmd, "ordinary-config").await);
    assert_report(&out, "retained");
    // Remembering the run did not route around the outbox.
    assert!(!f.root.join("work-a").join("staged.txt").exists());
    let seal = f.seal(&out);
    let invoked = seal["payload"]["originals"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"] == "ActionInvocation")
        .map(|p| serde_json::from_str::<Value>(p["text"].as_str().unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(invoked.len(), 1);
    assert_eq!(invoked[0]["tool_name"], "fs_write");
}

#[tokio::test]
async fn ordinary_capture_accepts_required_globals_on_either_side_of_run() {
    let f = Fixture::new("global-placement").await;
    f.config("local", "", "");
    let mut invalid = f.request.clone();
    invalid.protocol = "invalid-fixture-protocol".into();
    fs::write(f.manifest(), serde_json::to_vec(&invalid).unwrap()).unwrap();
    let globals = [
        "--no-mcp",
        "--no-code-tool",
        "--no-grants",
        "--tool",
        "goal_context",
    ];

    for before_run in [false, true] {
        let mut cmd = f.command("work-a");
        if before_run {
            cmd.args(globals);
        }
        cmd.args(["run", "--episodic-memory"])
            .arg(f.manifest())
            .args(["--goal", "task:placement", "Fixture prompt."]);
        if !before_run {
            cmd.args(globals);
        }
        // Both placements must reach manifest validation before any network.
        let label = if before_run {
            "before-run"
        } else {
            "after-run"
        };
        failure(&f.output(cmd, label).await, "ordinary protocol unavailable");
    }
    assert!(f.wire.requests().is_empty());
    assert!(!f.root.join("mecha/sessions").exists());
}

#[tokio::test]
async fn ordinary_learning_cli_preserves_single_call_wire_and_failed_raw_reply() {
    learning_wire_cases(true).await;
}

#[tokio::test]
async fn ordinary_learning_cli_keeps_partial_usage_unknown_per_counter() {
    learning_wire_cases(false).await;
}

async fn learning_wire_cases(identity_cases: bool) {
    for (served_model, incomplete, usage, expected_input, expected_output) in [
        (
            Some("other-model"),
            false,
            json!({"prompt_tokens":14,"completion_tokens":7}),
            Some(14),
            Some(7),
        ),
        (
            None,
            false,
            json!({"prompt_tokens":14,"completion_tokens":7}),
            Some(14),
            Some(7),
        ),
        (
            Some("other-model"),
            true,
            json!({"prompt_tokens":14,"completion_tokens":7}),
            Some(14),
            Some(7),
        ),
        (
            Some("fixture"),
            false,
            json!({"prompt_tokens":14}),
            Some(14),
            None,
        ),
        (
            Some("fixture"),
            false,
            json!({"completion_tokens":7}),
            None,
            Some(7),
        ),
        (
            Some("fixture"),
            false,
            json!({"prompt_tokens":0,"completion_tokens":0}),
            None,
            None,
        ),
        (Some("fixture"), false, Value::Null, None, None),
        (Some("fixture"), true, Value::Null, None, None),
    ] {
        if (served_model != Some("fixture")) != identity_cases {
            continue;
        }
        let failed = incomplete || served_model != Some("fixture");
        let f = Fixture::new("ordinary-learning").await;
        f.init().await;
        let mut selectors = Vec::new();
        for (workspace, text) in [
            ("work-a", "Fixture queue alpha needs approval."),
            ("work-b", "Fixture queue beta needs approval."),
        ] {
            f.wire.script([Step::Reply(answer(text))]);
            let observed = success(
                &f.output(
                    f.run(workspace, "charter:1", "Observe the fixture queue."),
                    workspace,
                )
                .await,
            );
            assert_report(&observed, "retained");
            selectors.push(json!({"run_id":run_id(&observed),"original_index":1}));
        }
        let input = f.root.join("sources.json");
        fs::write(&input, serde_json::to_vec(&selectors).unwrap()).unwrap();
        fs::set_permissions(&input, fs::Permissions::from_mode(0o600)).unwrap();
        let workspace = f.root.join("work-a");
        let descriptors = success(
            &f.host(&[
                "sources",
                "--workspace",
                workspace.to_str().unwrap(),
                "--input",
                input.to_str().unwrap(),
            ])
            .await,
        );
        let inventory = success(
            &f.host(&[
                "sources",
                "--workspace",
                workspace.to_str().unwrap(),
                "--run-id",
                selectors[0]["run_id"].as_str().unwrap(),
            ])
            .await,
        );
        assert_eq!(inventory["sources"][1]["source"], selectors[0]);
        assert_eq!(inventory["sources"][1]["kind"], "AssistantText");
        let draft = json!({"name":"Fixture queue approval","principle":"The observed queues need approval.","applicability":"Observed queues only.","support":descriptors["sources"].as_array().unwrap().iter().map(|s|json!({"source_id":s["reference"]["revision"],"field":s["field"],"quote":s["text"]})).collect::<Vec<_>>(),"counterexamples":[]});
        fs::write(&input,serde_json::to_vec(&json!({"protocol":"ordinary-session-learning-v1","operation":{"kind":"induce","sources":selectors}})).unwrap()).unwrap();
        let mut response = completion(
            json!({"role":"assistant","content":draft.to_string()}),
            if incomplete { "length" } else { "stop" },
        );
        if let Some(model) = served_model {
            response["model"] = json!(model);
        } else {
            response.as_object_mut().unwrap().remove("model");
        }
        if usage.is_null() {
            response.as_object_mut().unwrap().remove("usage");
        } else {
            response["usage"] = usage;
        }
        f.wire.script([Step::Reply(response)]);
        let before = f.wire.requests().len();
        // Learning must not consult generic/default provider configuration.
        fs::write(
            f.root.join("mecha/config.toml"),
            "deliberately invalid TOML [",
        )
        .unwrap();
        let output = f
            .host(&[
                "learn",
                "--workspace",
                workspace.to_str().unwrap(),
                "--input",
                input.to_str().unwrap(),
                "--endpoint",
                &f.endpoint,
                "--model",
                "fixture",
                "--holds-dir",
                f.root.join("holds").to_str().unwrap(),
                "--seed",
                "17",
            ])
            .await;
        assert_eq!(
            output.status.success(),
            !failed,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["protocol"], "ordinary-session-learning-v1");
        assert_eq!(report["status"], if failed { "failed" } else { "retained" });
        assert_eq!(report["reply"]["text"], draft.to_string());
        assert_eq!(report["reply"]["model"], served_model.unwrap_or(""));
        assert_eq!(report["reply"]["input_tokens"], json!(expected_input));
        assert_eq!(report["reply"]["output_tokens"], json!(expected_output));
        if served_model != Some("fixture") {
            assert!(report["reply"]["error"]
                .as_str()
                .unwrap()
                .contains("model identity mismatch"));
        }
        if incomplete {
            assert!(report["reply"]["error"]
                .as_str()
                .unwrap()
                .contains("incomplete model response"));
        }
        let concepts: i64 = f
            .sqlite()
            .query_row(
                "SELECT count(*) FROM object WHERE kind='concept'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(concepts, if failed { 0 } else { 1 });
        assert_eq!(report["retained"].is_null(), failed);
        let requests = f.wire.requests();
        let wire = &requests[before..];
        assert_eq!(wire.len(), 3, "{wire:?}");
        assert_eq!(wire[2]["uri"], "/v1/chat/completions?autoload=false");
        assert!(wire[2]["body"]["tools"].is_null());
        assert_eq!(wire[2]["body"]["seed"], 17);
        assert_eq!(wire[2]["body"]["model"], "fixture");
        assert!(report["generation"]["prompt"]
            .as_str()
            .unwrap()
            .contains("Fixture queue alpha"));
    }
}
