//! The image library's web door, end to end: a real `mecha serve` against a
//! throwaway home, driving every write that runs as a `mecha imagelib` child
//! — the half the in-process route tests cannot reach, because inside a unit
//! test the child would be the test binary (review of #385).
use std::process::Command;
use std::time::Duration;

fn png(shade: u8) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(4, 4, image::Rgb([shade, 30, 30]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn mecha(home: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args(args)
        .env("MECHA_HOME", home)
        .env("MECHA_SESSION_KIND", "test")
        .output()
        .unwrap()
}

struct Cleanup(std::path::PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
async fn every_library_write_the_page_can_make_lands_in_the_store_it_reads() {
    let root = std::env::temp_dir().join(format!("mecha-library-web-{}", std::process::id()));
    let _cleanup = Cleanup(root.clone());
    let home = root.join("home");
    let work = root.join("work");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    // A provider that is never reached: the chat only has to exist for the
    // upload door, and nothing here sends a turn.
    std::fs::write(
        home.join("config.toml"),
        r#"
default_provider = "fixture"
[providers.fixture]
kind = "openai-compatible"
base_url = "http://127.0.0.1:9"
model = "fixture"
max_retries = 0
[tools]
enabled = ["fs_read"]
[sandbox]
kind = "none"
"#,
    )
    .unwrap();

    // The library, made the owner's way.
    for (name, shade) in [("maya", 10u8), ("john", 20), ("sam", 30)] {
        let portrait = root.join(format!("{name}.png"));
        std::fs::write(&portrait, png(shade)).unwrap();
        let out = mecha(
            &home,
            &[
                "imagelib",
                "add-character",
                name,
                "--portrait",
                portrait.to_str().unwrap(),
                "--description",
                &format!("{name}, a memorable face"),
            ],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // sam as a model's proposal: a candidate, as `image_library_propose` makes.
    let sam = home.join("imagelib/characters/sam/entry.toml");
    let raw = std::fs::read_to_string(&sam)
        .unwrap()
        .replace("status = \"approved\"", "status = \"candidate\"")
        .replace("origin = \"owner\"", "origin = \"model_clean\"");
    std::fs::write(&sam, raw).unwrap();
    let blobs = || {
        std::fs::read_dir(home.join("imagelib/blobs"))
            .unwrap()
            .count()
    };
    assert_eq!(blobs(), 3);

    let reserve = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reserve.local_addr().unwrap().port();
    drop(reserve);
    let reserve = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let voice_port = reserve.local_addr().unwrap().port();
    drop(reserve);
    let log = std::fs::File::create(root.join("serve.log")).unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_mecha"))
        .args([
            "serve",
            "--port",
            &port.to_string(),
            "--voice-port",
            &voice_port.to_string(),
            "--owner-login",
            "test@example.com",
            "--no-mcp",
        ])
        .env("MECHA_HOME", &home)
        .env("MECHA_SESSION_KIND", "test")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .current_dir(&work)
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let client = reqwest::Client::builder()
        .default_headers(
            [
                (
                    "Tailscale-User-Login".parse().unwrap(),
                    "test@example.com".parse().unwrap(),
                ),
                ("X-Mecha-Request".parse().unwrap(), "1".parse().unwrap()),
            ]
            .into_iter()
            .collect(),
        )
        .build()
        .unwrap();
    let base = format!("http://127.0.0.1:{port}");
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if client.get(format!("{base}/api/ping")).send().await.is_ok() {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "{}",
                std::fs::read_to_string(root.join("serve.log")).unwrap()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();

    let post = |path: &str, body: serde_json::Value| {
        let (client, url) = (client.clone(), format!("{base}{path}"));
        async move { client.post(url).json(&body).send().await.unwrap() }
    };
    let list = |token: Option<String>| {
        let (client, base) = (client.clone(), base.clone());
        async move {
            let url = match token {
                Some(t) => format!("{base}/api/library?unlock={t}"),
                None => format!("{base}/api/library"),
            };
            client
                .get(url)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()
        }
    };
    let names = |v: &serde_json::Value| -> Vec<String> {
        v["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect()
    };

    // Lock: the child writes the store the page reads.
    let r = post("/api/library/character/maya/lock", serde_json::json!({})).await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let v = list(None).await;
    assert_eq!(names(&v), ["john", "sam"]);
    assert!(
        v.get("hidden_locked").is_none(),
        "no count of what is hidden"
    );

    // No password set: the toggle opens for the asking, and a locked entry
    // is unlocked only with it.
    let r = post("/api/library/unlock", serde_json::json!({})).await;
    let token = r.json::<serde_json::Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let r = post(
        "/api/library/character/maya/unlock",
        serde_json::json!({"unlock": token}),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let v = list(None).await;
    assert!(names(&v).contains(&"maya".to_string()));

    // Remove: moved aside, its portrait kept (a manifest may name it).
    let r = post("/api/library/character/john/remove", serde_json::json!({})).await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    assert!(!names(&list(None).await).contains(&"john".to_string()));
    assert!(home
        .join("imagelib/removed")
        .read_dir()
        .unwrap()
        .next()
        .is_some());
    assert_eq!(blobs(), 3);

    // Reject: the candidate and its unshared portrait are gone.
    let r = post("/api/library/character/sam/reject", serde_json::json!({})).await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    assert!(!names(&list(None).await).contains(&"sam".to_string()));
    assert_eq!(blobs(), 2);

    // Save from a chat picture, with a description that opens with a dash —
    // a description, not a flag.
    let r = client
        .post(format!("{base}/api/chat/libtest/upload?name=face.png"))
        .body(png(90))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let r = post(
        "/api/library/save",
        serde_json::json!({"key": "libtest", "path": "inbox/face.png", "name": "theo",
                           "description": "- a lanky man with red hair", "locked": false}),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let shown = mecha(&home, &["imagelib", "show", "theo"]);
    let shown = String::from_utf8_lossy(&shown.stdout);
    assert!(
        shown.contains("text:    - a lanky man with red hair"),
        "{shown}"
    );
    assert!(shown.contains("origin:  yours"), "{shown}");

    // Add a character from an uploaded portrait, and a style from its text —
    // each opening with a dash, which is text, not a flag.
    let b64 = |bytes: Vec<u8>| {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    };
    let r = post(
        "/api/library/add",
        serde_json::json!({"kind": "character", "name": "ada", "portrait": b64(png(120)),
                           "text": "- a tall woman with a silver bob", "locked": true}),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let shown = mecha(&home, &["imagelib", "show", "ada"]);
    let shown = String::from_utf8_lossy(&shown.stdout);
    assert!(
        shown.contains("text:    - a tall woman with a silver bob"),
        "{shown}"
    );
    assert!(shown.contains("status:  approved, locked"), "{shown}");
    let r = post(
        "/api/library/add",
        serde_json::json!({"kind": "style", "name": "ink", "text": "--dramatic ink wash"}),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let shown = mecha(&home, &["imagelib", "show", "ink", "--kind", "style"]);
    assert!(String::from_utf8_lossy(&shown.stdout).contains("text:    --dramatic ink wash"));

    // Edit: a new description is a new version, the old one kept; a new
    // portrait alone leaves the text as it was.
    let r = post(
        "/api/library/edit",
        serde_json::json!({"kind": "character", "name": "maya", "text": "maya, now with a scar"}),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let r = post(
        "/api/library/edit",
        serde_json::json!({"kind": "character", "name": "maya", "portrait": b64(png(200))}),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let shown = mecha(&home, &["imagelib", "show", "maya"]);
    let shown = String::from_utf8_lossy(&shown.stdout);
    assert!(shown.contains("character maya (v3)"), "{shown}");
    assert!(shown.contains("text:    maya, now with a scar"), "{shown}");
    for v in ["v1", "v2"] {
        assert!(home
            .join(format!("imagelib/characters/maya/history/{v}.toml"))
            .is_file());
    }
    // A style's text through the same door.
    let r = post(
        "/api/library/edit",
        serde_json::json!({"kind": "style", "name": "ink", "text": "soft ink wash, grey paper"}),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
    let shown = mecha(&home, &["imagelib", "show", "ink", "--kind", "style"]);
    let shown = String::from_utf8_lossy(&shown.stdout);
    assert!(shown.contains("style ink (v2)"), "{shown}");
    assert!(
        shown.contains("text:    soft ink wash, grey paper"),
        "{shown}"
    );

    // A locked entry is edited only while unlocked.
    let r = post(
        "/api/library/edit",
        serde_json::json!({"kind": "character", "name": "ada", "text": "changed"}),
    )
    .await;
    assert_eq!(r.status(), 404);
    let r = post(
        "/api/library/edit",
        serde_json::json!({"kind": "character", "name": "ada", "text": "changed", "unlock": token}),
    )
    .await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());

    let _ = child.kill().await;
}
