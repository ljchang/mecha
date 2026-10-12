use super::*;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mecha-render-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The render's `Egress::None` rests on these: every namespace unshared, the
/// network's included, and of the home exactly the four binds the module
/// names — the browser's directory, the bundle, the output directory and this
/// binary — with home-rooted paths as they really are (Playwright's browser
/// under `~/.cache`, the scratch under `~/.mecha`), so the check is not of
/// its own literals (review of #662).
#[test]
fn the_sandbox_has_no_network_and_binds_only_its_four_paths() {
    let home = PathBuf::from("/home/someone");
    let browser = home.join(".cache/ms-playwright/chromium_headless_shell-1/chrome");
    let bundle = home.join(".mecha/hud/.scratch/1-0/bundle");
    let out = home.join(".mecha/hud/.scratch/1-0/out");
    let mecha = home.join(".cargo/bin/mecha");
    let a = sandbox_args(&browser, &bundle, &out, &mecha);
    assert_eq!(a[0], "--unshare-all", "{a:?}");
    assert!(!a.iter().any(|x| x == "--share-net"), "{a:?}");
    // The ELF interpreter's directory as this machine has it: /lib64 on
    // x86-64, /lib on aarch64 (review of #662).
    for dir in ["/lib", "/lib64"] {
        if std::fs::symlink_metadata(dir).is_ok() {
            assert!(a.iter().any(|x| x == dir), "{dir} is reachable: {a:?}");
        }
    }
    let allowed = [&browser, &bundle, &out, &mecha].map(|p| p.to_string_lossy().into_owned());
    let mut from_home = Vec::new();
    for (i, x) in a.iter().enumerate() {
        if x == "--bind" || x == "--ro-bind" {
            let src = &a[i + 1];
            if src.starts_with("/home") {
                from_home.push(src.clone());
            }
        }
    }
    from_home.sort();
    let mut expected = allowed.to_vec();
    expected.sort();
    assert_eq!(from_home, expected, "nothing else of the home is bound");
    let writable: Vec<&String> = a
        .iter()
        .enumerate()
        .filter(|(_, x)| *x == "--bind")
        .map(|(i, _)| &a[i + 2])
        .collect();
    assert_eq!(writable, [&"/out".to_string()], "one writable directory");
    assert_eq!(a.last().map(String::as_str), Some("--"));
}

/// The blank below the last panel is cut, with a margin; content is kept.
#[test]
fn a_screenshot_is_cropped_to_its_content() {
    let mut img = image::RgbaImage::from_pixel(40, 200, image::Rgba([240, 240, 240, 255]));
    for x in 5..35 {
        img.put_pixel(x, 50, image::Rgba([10, 10, 10, 255]));
    }
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let shot = crop(&png).unwrap();
    assert!(!shot.cut);
    let cropped = image::load_from_memory(&shot.png).unwrap();
    assert_eq!(cropped.height(), 50 + 24);
    assert_eq!(cropped.width(), 40);
    // Content in the last row: the page ran past the viewport, and says so.
    let mut tall = image::RgbaImage::from_pixel(40, 200, image::Rgba([240, 240, 240, 255]));
    tall.put_pixel(20, 199, image::Rgba([10, 10, 10, 255]));
    tall.put_pixel(0, 199, image::Rgba([240, 240, 240, 255]));
    let mut png = Vec::new();
    tall.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    assert!(crop(&png).unwrap().cut, "a page past the viewport is said");
}

fn get(port: u16, path: &str) -> String {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

/// The loopback server serves the bundle and nothing beside it, every answer
/// under the interactive CSP.
#[test]
fn the_server_answers_only_inside_the_bundle_under_the_csp() {
    let root = scratch("serve");
    let bundle = root.join("bundle");
    std::fs::create_dir_all(bundle.join("data")).unwrap();
    std::fs::write(bundle.join("index.html"), "<p>page</p>").unwrap();
    std::fs::write(bundle.join("data").join("a.json"), "{}").unwrap();
    std::fs::write(root.join("secret.txt"), "not for the page").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let served = bundle.canonicalize().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let _ = answer(stream, &served);
        }
    });
    let page = get(port, "/index.html");
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(
        page.contains("Content-Security-Policy: default-src 'self'"),
        "{page}"
    );
    assert!(page.contains("frame-ancestors 'none'"), "{page}");
    assert!(get(port, "/data/a.json").contains("application/json"));
    for escape in [
        "/../secret.txt",
        "/%2e%2e/secret.txt",
        "/data/../../secret.txt",
    ] {
        let r = get(port, escape);
        assert!(r.starts_with("HTTP/1.1 404"), "{escape}: {r}");
        assert!(!r.contains("not for the page"), "{escape}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A real render, where this machine can make one: a page drawn in the
/// sandbox comes back as a picture. Skipped without a browser, `bwrap` or a
/// built renderer, unless `MECHA_TEST_REQUIRE_BACKENDS=1`, which makes the
/// skip a failure.
#[test]
fn a_page_is_drawn_inside_the_sandbox() {
    let require = std::env::var("MECHA_TEST_REQUIRE_BACKENDS").as_deref() == Ok("1");
    let renderer = match Renderer::find() {
        Ok(r) => r,
        Err(why) => {
            assert!(!require, "no renderer: {why}");
            eprintln!("skipped: {why}");
            return;
        }
    };
    // The test binary is not `mecha`: point at the built CLI beside it.
    let mecha = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent()?.parent().map(|d| d.join("mecha")))
        .filter(|p| p.is_file());
    let Some(mecha) = mecha else {
        assert!(!require, "no built `mecha` beside the test binary");
        eprintln!("skipped: no built `mecha` beside the test binary");
        return;
    };
    let renderer = Renderer { mecha, ..renderer };
    let root = scratch("draw");
    let bundle = root.join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    // A stylesheet, not `style=`: the interactive CSP refuses inline style,
    // and the render runs under it.
    std::fs::write(
        bundle.join("index.html"),
        "<!doctype html><html><head><link rel=\"stylesheet\" href=\"s.css\"></head>\
         <body><div></div></body></html>",
    )
    .unwrap();
    std::fs::write(
        bundle.join("s.css"),
        "body{margin:0;background:#fff}div{width:300px;height:120px;background:#000}",
    )
    .unwrap();
    let png = match renderer.render(&bundle, &root) {
        Ok(shot) => shot.png,
        Err(e) if !require => {
            eprintln!("skipped: the sandbox could not render here: {e:#}");
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        Err(e) => panic!("{e:#}"),
    };
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!(img.width(), WIDTH);
    assert!(img.height() < HEIGHT, "cropped to the content");
    assert_eq!(img.get_pixel(10, 10).0[..3], [0, 0, 0], "the box was drawn");
    let _ = std::fs::remove_dir_all(&root);
}
