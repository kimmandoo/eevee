use super::*;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

/// One-shot loopback HTTP server returning `body` with `content_type`.
fn serve(responses: Vec<(&'static str, String, String)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for (status, content_type, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0usize;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap_or(0);
                }
            }
            let mut discard = vec![0; length];
            std::io::Read::read_exact(&mut reader, &mut discard).unwrap();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
    });
    format!("http://{address}")
}

fn local() -> Client {
    Client {
        allow_local: true,
        ..Client::default()
    }
}

#[test]
fn definitions_are_strict_compatible() {
    for definition in definitions() {
        let parameters = &definition["parameters"];
        let mut required: Vec<_> = parameters["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
        let mut properties: Vec<_> = parameters["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        required.sort();
        properties.sort();
        assert_eq!(required, properties, "strict mode requires every property");
        assert_eq!(parameters["additionalProperties"], false);
    }
}

#[test]
fn validation_refuses_unsafe_requests() {
    assert!(Call::search("rust ureq resolver", 5, "week").is_ok());
    assert!(Call::search("  ", 5, "any").is_err());
    assert!(Call::search("x", 5, "decade").is_err());
    assert_eq!(
        Call::search("x", 99, "any").unwrap(),
        Call::Search {
            query: "x".into(),
            limit: MAX_RESULTS,
            recency: Recency::Any
        }
    );
    assert!(Call::fetch("https://example.com/a?b=c", 0).is_ok());
    assert!(Call::fetch("file:///etc/passwd", 0).is_err());
    assert!(Call::fetch("ftp://example.com", 0).is_err());
    assert!(Call::fetch("https://user:pw@example.com/", 0).is_err());
    assert!(Call::fetch("https://exa mple.com/", 0).is_err());
    assert!(Call::fetch("https://", 0).is_err());
}

#[test]
fn private_addresses_are_not_public() {
    for blocked in [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "::1",
        "fe80::1",
        "fd00::1",
        "::ffff:127.0.0.1",
    ] {
        assert!(!is_public(blocked.parse().unwrap()), "{blocked}");
    }
    for allowed in ["93.184.216.34", "1.1.1.1", "2606:4700::1111"] {
        assert!(is_public(allowed.parse().unwrap()), "{allowed}");
    }
}

#[test]
fn loopback_is_blocked_without_opt_in() {
    let base = serve(vec![]);
    let client = Client {
        allow_local: false,
        ..Client::default()
    };
    let result = client.run(&Call::fetch(&format!("{base}/"), 0).unwrap());
    assert_eq!(result["status"], "error");
    assert_eq!(result["correctable"], true);
    assert!(
        result["message"].as_str().unwrap().contains("blocked"),
        "{result}"
    );
}

#[test]
fn fetch_converts_html_and_pages_long_text() {
    let long = "word ".repeat(4000);
    let html = format!(
        "<!doctype html><html><head><title>Guide &amp; Notes</title><style>.x{{}}</style></head>\
         <body><nav>Menu</nav><h1>Install</h1><p>Run <code>cargo add ureq</code>.</p>\
         <ul><li>First</li><li>Second <a href=\"/docs/next\">next page</a></li></ul>\
         <script>alert(1)</script><pre>let x = 1;\n  let y = 2;</pre><p>{long}</p></body></html>"
    );
    let base = serve(vec![("200 OK", "text/html; charset=utf-8".into(), html)]);
    let result = local().run(&Call::fetch(&format!("{base}/guide/intro"), 0).unwrap());
    assert_eq!(result["status"], "success", "{result}");
    assert_eq!(result["title"], "Guide & Notes");
    let content = result["content"].as_str().unwrap();
    assert!(content.starts_with("# Install"), "{content}");
    assert!(content.contains("Run `cargo add ureq`."));
    assert!(content.contains("- First\n- Second next page ("));
    assert!(content.contains(&format!("({base}/docs/next)")));
    assert!(content.contains("```\nlet x = 1;\n  let y = 2;\n```"));
    assert!(!content.contains("Menu") && !content.contains("alert"));
    assert_eq!(content.chars().count(), PAGE_CHARS);
    let next = result["next_offset"].as_u64().unwrap() as usize;
    assert_eq!(next, PAGE_CHARS);
    assert!(result["total_chars"].as_u64().unwrap() as usize > PAGE_CHARS);
}

#[test]
fn fetch_returns_json_and_refuses_binary() {
    let base = serve(vec![
        ("200 OK", "application/json".into(), "{\"ok\":true}".into()),
        (
            "200 OK",
            "application/octet-stream".into(),
            "\u{0}\u{1}".into(),
        ),
        ("404 Not Found", "text/plain".into(), "missing".into()),
    ]);
    let client = local();
    let json = client.run(&Call::fetch(&format!("{base}/a.json"), 0).unwrap());
    assert_eq!(json["content"], "{\"ok\":true}");
    assert!(json["next_offset"].is_null());
    let binary = client.run(&Call::fetch(&format!("{base}/a.bin"), 0).unwrap());
    assert_eq!(binary["status"], "refused");
    assert_eq!(binary["correctable"], true);
    let missing = client.run(&Call::fetch(&format!("{base}/none"), 0).unwrap());
    assert_eq!(missing["status"], "error");
    assert!(missing["message"].as_str().unwrap().contains("404"));
}

const DUCKDUCKGO_PAGE: &str = r#"
<div class="result results_links results_links_deep web-result result--ad">
  <a rel="nofollow" class="result__a" href="https://duckduckgo.com/y.js?ad_domain=x">Ad title</a>
</div>
<div class="result results_links results_links_deep web-result">
  <h2 class="result__title"><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.rs%2Fureq%2Flatest%2F&amp;rut=abc">ureq &mdash; <b>Rust</b></a></h2>
  <a class="result__snippet" href="//duckduckgo.com/l/?uddg=x">Simple, safe <b>HTTP</b> client.</a>
</div>
<div class="result results_links results_links_deep web-result">
  <h2 class="result__title"><a rel="nofollow" class="result__a" href="https://github.com/algesten/ureq">algesten/ureq</a></h2>
</div>
"#;

#[test]
fn duckduckgo_results_skip_ads_and_unwrap_links() {
    let results = parse_duckduckgo(DUCKDUCKGO_PAGE, 10);
    assert_eq!(results.len(), 2, "{results:?}");
    assert_eq!(results[0]["title"], "ureq — Rust");
    assert_eq!(results[0]["url"], "https://docs.rs/ureq/latest/");
    assert_eq!(results[0]["snippet"], "Simple, safe HTTP client.");
    assert_eq!(results[1]["url"], "https://github.com/algesten/ureq");
    assert_eq!(results[1]["snippet"], "");
    assert_eq!(parse_duckduckgo(DUCKDUCKGO_PAGE, 1).len(), 1);
}

#[test]
fn search_reports_bot_checks_as_correctable() {
    let base = serve(vec![
        ("200 OK", "text/html".into(), DUCKDUCKGO_PAGE.into()),
        (
            "202 Accepted",
            "text/html".into(),
            "<div class=\"anomaly-modal\"></div>".into(),
        ),
    ]);
    let client = Client {
        duckduckgo: format!("{base}/html/"),
        ..local()
    };
    // Brave is used only when its key is set; this test exercises DuckDuckGo.
    if brave_key().is_some() {
        return;
    }
    let call = Call::search("ureq", 5, "month").unwrap();
    let found = client.run(&call);
    assert_eq!(found["status"], "success", "{found}");
    assert_eq!(found["results"].as_array().unwrap().len(), 2);
    let blocked = client.run(&call);
    assert_eq!(blocked["status"], "error");
    assert_eq!(blocked["correctable"], true);
}

#[test]
fn relative_links_resolve_against_the_page() {
    assert_eq!(
        absolute("https://a.dev/x/y.html?q=1", "z.html").as_deref(),
        Some("https://a.dev/x/z.html")
    );
    assert_eq!(
        absolute("https://a.dev/x/y", "/root").as_deref(),
        Some("https://a.dev/root")
    );
    assert_eq!(
        absolute("https://a.dev/", "//cdn.dev/f").as_deref(),
        Some("https://cdn.dev/f")
    );
    assert_eq!(
        absolute("https://docs.rs/u/2/u/", "../src/lib.rs.html#1").as_deref(),
        Some("https://docs.rs/u/2/src/lib.rs.html#1")
    );
    assert_eq!(absolute("https://a.dev/", "mailto:x@y"), None);
    assert_eq!(absolute("https://a.dev/", "#top"), None);
}

/// Live network check; run explicitly with `cargo test live_web -- --ignored`.
#[test]
#[ignore]
fn live_web_search_and_fetch() {
    let client = Client::default();
    let search = client.run(&Call::search("ureq rust http client", 5, "any").unwrap());
    eprintln!("{}", serde_json::to_string_pretty(&search).unwrap());
    let page = client.run(&Call::fetch("https://docs.rs/ureq/2.12.1/ureq/", 0).unwrap());
    eprintln!(
        "{} {} {}\n{}",
        page["status"],
        page["title"],
        page["total_chars"],
        page["content"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(1500)
            .collect::<String>()
    );
    assert_eq!(page["status"], "success");
}
