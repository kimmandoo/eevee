use super::*;

struct Fixture {
    root: PathBuf,
    registration: Registration,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "eevee-mcp-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let program = if runner::resolve_executable("python3", &root).is_ok() {
            "python3"
        } else {
            "python"
        };
        let registration = Registration {
            name: "fixture".into(),
            program: program.into(),
            args: vec![
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("scripts/mcp_stdio_fixture.py")
                    .to_string_lossy()
                    .into_owned(),
                "--root".into(),
                root.to_string_lossy().into_owned(),
            ],
            cwd: PathBuf::from("."),
            env: Vec::new(),
            critical: Vec::new(),
            extra: forward::Extra::default(),
        };
        Self { root, registration }
    }
    fn catalogue(pages: Vec<Value>) -> Self {
        let mut fixture = Self::new();
        std::fs::write(
            fixture.root.join("catalogue.json"),
            serde_json::to_vec(&pages).unwrap(),
        )
        .unwrap();
        let script = fixture.root.join("catalogue.py");
        std::fs::write(
            &script,
            r#"import json, pathlib, sys, time
root = pathlib.Path(sys.argv[1])
pages = json.loads((root / "catalogue.json").read_text(encoding="utf-8"))
index = 0
for line in sys.stdin:
    request = json.loads(line)
    with (root / "discovery-events.jsonl").open("a", encoding="utf-8") as events:
        events.write(json.dumps(request) + "\n")
    if "id" not in request:
        continue
    method = request["method"]
    if method == "initialize":
        result = {"protocolVersion": "2024-11-05"}
    elif method == "tools/list":
        result = pages[min(index, len(pages) - 1)]
        index += 1
        if result.get("fixture_wait"):
            time.sleep(60)
        if result.get("fixture_error"):
            print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32603}}), flush=True)
            continue
    else:
        raise RuntimeError("unexpected effect request")
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
"#,
        )
        .unwrap();
        fixture.registration.args = vec![
            script.to_string_lossy().into_owned(),
            fixture.root.to_string_lossy().into_owned(),
        ];
        fixture
    }
    fn try_connect(&self, cancelled: &dyn Fn() -> bool) -> Result<Session, String> {
        let prepared = self.registration.prepare(&self.root)?;
        Session::connect(
            &self.registration,
            &prepared,
            self.root.to_str().unwrap(),
            cancelled,
        )
    }
    fn discovery_requests(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.join("discovery-events.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    fn connect(&self) -> Session {
        let prepared = self.registration.prepare(&self.root).unwrap();
        Session::connect(
            &self.registration,
            &prepared,
            self.root.to_str().unwrap(),
            &|| false,
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn registration_is_inert_and_persistence_preserves_unknown_fields() {
    let fixture = Fixture::new();
    let prepared = fixture.registration.prepare(&fixture.root).unwrap();
    assert!(!fixture.root.join("mcp.pid").exists());
    let review = fixture.registration.review(&prepared);
    assert!(
        review.contains("UNMANAGED")
            && review.contains("NOT sandboxed")
            && review.contains("Exact argv")
    );
    let mut document = Document::default();
    let mut registration = fixture.registration.clone();
    registration
        .extra
        .0
        .insert("future_hint".into(), json!({"nested":[1,2]}));
    document
        .extra
        .0
        .insert("future_document".into(), json!("preserved"));
    document.projects.insert(
        fixture.root.to_string_lossy().into_owned(),
        vec![registration.clone()],
    );
    document.save(&fixture.root).unwrap();
    let restored = Document::load(&fixture.root).unwrap();
    assert_eq!(restored.extra, document.extra);
    assert_eq!(restored.projects.values().next().unwrap()[0], registration);
    assert!(!fixture.root.join("mcp.pid").exists());
    registration.critical.push("future_hint".into());
    assert!(
        registration
            .prepare(&fixture.root)
            .unwrap_err()
            .contains("future_hint")
    );
}

#[test]
fn exact_launch_identity_and_project_cwd_are_enforced() {
    let fixture = Fixture::new();
    let mut registration = fixture.registration.clone();
    registration.cwd = fixture.root.parent().unwrap().to_path_buf();
    assert!(
        registration
            .prepare(&fixture.root)
            .unwrap_err()
            .contains("inside this project")
    );
    let mut prepared = fixture.registration.prepare(&fixture.root).unwrap();
    prepared.executable.sha256 = "0".repeat(64);
    assert!(
        Session::connect(
            &fixture.registration,
            &prepared,
            fixture.root.to_str().unwrap(),
            &|| false
        )
        .err()
        .unwrap()
        .contains("identity")
    );
    assert!(!fixture.root.join("mcp.pid").exists());
}

#[test]
fn initialize_and_list_once_calls_are_bounded_sanitized_and_unmanaged() {
    let fixture = Fixture::new();
    let mut session = fixture.connect();
    assert_eq!(session.tools.len(), 3);
    let result = session.call(
        "echo",
        &json!({"text":"before\u{1b}[31mred\u{1b}[0m\u{1b}]0;hidden\u{7}after"}),
        &|| false,
    );
    assert_eq!(result["status"], "success");
    assert_eq!(result["result"]["content"][0]["text"], "beforeredafter");
    let result = session.call(
        "write_note",
        &json!({"text":"reviewed fixture write 한글"}),
        &|| false,
    );
    assert_eq!(result["status"], "success");
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("note.txt")).unwrap(),
        "reviewed fixture write 한글"
    );
    let events = std::fs::read_to_string(fixture.root.join("mcp-events.jsonl")).unwrap();
    assert_eq!(events.matches("\"event\": \"initialize\"").count(), 1);
    assert_eq!(events.matches("\"event\": \"tools/list\"").count(), 1);
    assert!(!events.contains("OPENAI_API_KEY"));
    let result = session.call("echo", &json!({"text":"", "mode":"oversize"}), &|| false);
    assert_eq!(result["status"], "error");
    assert!(result.to_string().len() <= LIMIT);
    assert!(session.process.is_none());
}

#[test]
fn roots_only_and_server_initiated_effect_features_are_refused() {
    let fixture = Fixture::new();
    let mut session = fixture.connect();
    let roots = session.call("echo", &json!({"text":"", "mode":"roots"}), &|| false);
    let reply: Value =
        serde_json::from_str(roots["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(reply["result"]["roots"].as_array().unwrap().len(), 1);
    assert_eq!(
        reply["result"]["roots"][0]["uri"],
        reqwest::Url::from_directory_path(&fixture.root)
            .unwrap()
            .as_str()
    );
    for mode in ["sampling", "elicitation", "resources", "prompts"] {
        let result = session.call("echo", &json!({"text":"", "mode":mode}), &|| false);
        let reply: Value =
            serde_json::from_str(result["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(reply["error"]["code"], -32601);
    }
}

#[test]
fn crash_and_cancel_are_tool_errors_and_kill_owned_server() {
    let fixture = Fixture::new();
    let mut session = fixture.connect();
    let result = session.call("echo", &json!({"text":"", "mode":"crash"}), &|| false);
    assert_eq!(result["status"], "error");
    assert!(session.process.is_none());
    let mut session = fixture.connect();
    let started = Instant::now();
    let result = session.call("echo", &json!({"text":"", "mode":"hang"}), &|| {
        started.elapsed() > Duration::from_millis(300)
    });
    assert_eq!(result["status"], "error");
    assert!(result["message"].as_str().unwrap().contains("terminated"));
    assert!(session.process.is_none());
    assert!(fixture.root.join("mcp-child.pid").exists());
}

#[test]
fn server_annotation_never_marks_user_read_or_grants_selection() {
    let fixture = Fixture::new();
    let session = Arc::new(Mutex::new(fixture.connect()));
    let tool = {
        let connected = session.lock();
        SelectedTool {
            server: "fixture".into(),
            name: "echo".into(),
            description: "untrusted".into(),
            schema: connected.tools[0]["inputSchema"].clone(),
            annotations: json!({"readOnlyHint":true}),
            user_read: false,
            project: fixture.root.to_string_lossy().into_owned(),
            session_id: connected.id,
            session: session.clone(),
            effects: Effects::load(&fixture.root, fixture.root.to_str().unwrap()).unwrap(),
        }
    };
    assert!(tool.validate(&tool.project, false).is_err());
    assert!(tool.validate(&tool.project, true).is_ok());
    assert_eq!(tool.declaration()["name"], "mcp__fixture__echo");
    let mut explicitly_read = tool.clone();
    explicitly_read.user_read = true;
    assert!(
        explicitly_read
            .validate(&explicitly_read.project, false)
            .is_ok()
    );
    assert!(explicitly_read.validate("another-project", true).is_err());
    session.lock().stop();
    assert!(tool.validate(&tool.project, true).is_err());
}

fn catalogue_tool(name: &str) -> Value {
    json!({"name":name,"description":"untrusted","inputSchema":{"type":"object","properties":{}}})
}

#[test]
fn paginated_discovery_publishes_every_tool_without_selecting_or_calling() {
    let fixture = Fixture::catalogue(vec![
        json!({"tools":[catalogue_tool("first")],"nextCursor":"opaque / page\n2"}),
        json!({"tools":[],"nextCursor":""}),
        json!({"tools":[catalogue_tool("last")]}),
    ]);
    assert!(!fixture.root.join("discovery-events.jsonl").exists());
    let session = fixture.try_connect(&|| false).unwrap();
    assert_eq!(
        session
            .tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["first", "last"]
    );
    let requests = fixture.discovery_requests();
    let lists = requests
        .iter()
        .filter(|request| request["method"] == "tools/list")
        .collect::<Vec<_>>();
    assert_eq!(lists.len(), 3);
    assert_eq!(lists[0]["params"], json!({}));
    assert_eq!(lists[1]["params"], json!({"cursor":"opaque / page\n2"}));
    assert_eq!(lists[2]["params"], json!({"cursor":""}));
    assert!(
        requests
            .iter()
            .all(|request| request["method"] != "tools/call")
    );
}

#[test]
fn duplicate_names_and_cursor_cycles_refuse_the_complete_catalogue() {
    for pages in [
        vec![
            json!({"tools":[catalogue_tool("same")],"nextCursor":"next"}),
            json!({"tools":[catalogue_tool("same")]}),
        ],
        vec![json!({"tools":[catalogue_tool("same"),catalogue_tool("same")]})],
        vec![
            json!({"tools":[catalogue_tool("first")],"nextCursor":"a"}),
            json!({"tools":[],"nextCursor":"b"}),
            json!({"tools":[],"nextCursor":"a"}),
        ],
    ] {
        let fixture = Fixture::catalogue(pages);
        let error = fixture.try_connect(&|| false).err().unwrap();
        assert!(
            error.contains("duplicate") || error.contains("cursor cycle"),
            "{error}"
        );
        assert!(
            fixture
                .discovery_requests()
                .iter()
                .all(|request| request["method"] != "tools/call")
        );
    }
}

#[test]
fn malformed_or_failed_later_pages_never_publish_a_partial_catalogue() {
    for last in [
        json!({"tools":[],"nextCursor":null}),
        json!({"tools":[],"nextCursor":3}),
        json!({"not_tools":[]}),
        json!({"tools":[{"name":"broken","inputSchema":{"type":"string"}}]}),
        json!({"fixture_error":true}),
    ] {
        let fixture = Fixture::catalogue(vec![
            json!({"tools":[catalogue_tool("valid")],"nextCursor":"next"}),
            last,
        ]);
        assert!(fixture.try_connect(&|| false).is_err());
        assert_eq!(
            fixture
                .discovery_requests()
                .iter()
                .filter(|request| request["method"] == "tools/list")
                .count(),
            2
        );
    }
}

#[test]
fn tool_and_response_byte_limits_apply_across_all_pages() {
    let first = (0..32)
        .map(|index| catalogue_tool(&format!("tool_{index}")))
        .collect::<Vec<_>>();
    let second = (32..64)
        .map(|index| catalogue_tool(&format!("tool_{index}")))
        .collect::<Vec<_>>();
    let fixture = Fixture::catalogue(vec![
        json!({"tools":first,"nextCursor":"next"}),
        json!({"tools":second}),
    ]);
    assert_eq!(fixture.try_connect(&|| false).unwrap().tools.len(), 64);
    let fixture = Fixture::catalogue(vec![
        json!({"tools":first,"nextCursor":"next"}),
        json!({"tools":(32..65).map(|index| catalogue_tool(&format!("tool_{index}"))).collect::<Vec<_>>()}),
    ]);
    assert!(
        fixture
            .try_connect(&|| false)
            .err()
            .unwrap()
            .contains("64 tools")
    );
    let mut large = catalogue_tool("large");
    large["description"] = json!("x".repeat(33 * 1024));
    let mut other = large.clone();
    other["name"] = json!("other");
    let fixture = Fixture::catalogue(vec![
        json!({"tools":[large],"nextCursor":"next"}),
        json!({"tools":[other]}),
    ]);
    assert!(
        fixture
            .try_connect(&|| false)
            .err()
            .unwrap()
            .contains("64 KiB")
    );
}

#[test]
fn discovery_cancellation_and_shared_deadline_stop_without_retry() {
    let fixture = Fixture::catalogue(vec![
        json!({"tools":[catalogue_tool("first")],"nextCursor":"next"}),
        json!({"tools":[],"fixture_wait":true}),
    ]);
    let started = Instant::now();
    assert!(
        fixture
            .try_connect(&|| started.elapsed() > Duration::from_millis(500))
            .is_err()
    );
    let requests = fixture.discovery_requests();
    assert!(
        requests
            .iter()
            .filter(|request| request["method"] == "initialize")
            .count()
            == 1
    );
    assert!(
        requests
            .iter()
            .filter(|request| request["method"] == "tools/list")
            .count()
            <= 2
    );
    assert!(
        requests
            .iter()
            .all(|request| request["method"] != "tools/call")
    );

    let fixture = Fixture::catalogue(vec![json!({"tools":[]})]);
    let mut session = fixture.try_connect(&|| false).unwrap();
    let mut received = 0;
    let deadline = Instant::now() + Duration::from_secs(1);
    session
        .rpc_until("tools/list", json!({}), &|| false, deadline, &mut received)
        .unwrap();
    assert!(received > 0);
    // A subsequent page cannot renew an already exhausted discovery deadline.
    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
    let error = session
        .rpc_until(
            "tools/list",
            json!({"cursor":"next"}),
            &|| false,
            deadline,
            &mut received,
        )
        .unwrap_err();
    assert!(error.contains("timed out"));
    assert!(session.process.is_none());
    assert_eq!(
        fixture
            .discovery_requests()
            .iter()
            .filter(|request| request["method"] == "tools/list")
            .count(),
        2
    );
}

#[test]
fn schema_json_is_exact_while_display_metadata_is_sanitized() {
    let schema = json!({
        "type":"object",
        "properties":{
            "literal\nkey":{
                "type":"string",
                "enum":["line\nbreak","carriage\rreturn","tab\tvalue","literal\\u001b"],
                "default":"carriage\rreturn",
                "pattern":"^[a-z]+\\s*$"
            }
        },
        "required":["literal\nkey"],
        "additionalProperties":false,
        "$defs":{"nested":{"const":{"literal\tkey":["a\r\nb"]}}}
    });
    let fixture = Fixture::catalogue(vec![json!({"tools":[{
        "name":"exact",
        "description":"before\u{1b}[31mred\u{1b}[0m ghp_secret12345",
        "inputSchema":schema,
        "annotations":{"title":"unsafe\u{202e}display"}
    }]})]);
    let session = Arc::new(Mutex::new(fixture.try_connect(&|| false).unwrap()));
    let connected = session.lock();
    assert_eq!(connected.tools[0]["inputSchema"], schema);
    assert_eq!(connected.tools[0]["description"], "beforered [REDACTED]");
    assert_eq!(connected.tools[0]["annotations"]["title"], "unsafedisplay");
    let tool = SelectedTool {
        server: fixture.registration.name.clone(),
        name: "exact".into(),
        description: connected.tools[0]["description"].as_str().unwrap().into(),
        schema: connected.tools[0]["inputSchema"].clone(),
        annotations: json!({}),
        user_read: false,
        project: fixture.root.to_string_lossy().into_owned(),
        session_id: connected.id,
        session: session.clone(),
        effects: Effects::load(&fixture.root, fixture.root.to_str().unwrap()).unwrap(),
    };
    assert_eq!(tool.declaration()["parameters"], schema);
}

#[test]
fn unsafe_schema_strings_and_keys_are_refused_not_rewritten() {
    for schema in [
        json!({"type":"object","properties":{"unsafe\u{1b}[31m":{"type":"string"}}}),
        json!({"type":"object","properties":{"ghp_secret12345":{"type":"string"}}}),
        json!({"type":"object","properties":{"value":{"enum":["bad\u{0}value"]}}}),
        json!({"type":"object","default":{"value":"bad\u{202e}value"}}),
        json!({"type":"object","description":"Bearer secret12345"}),
        json!({"type":"object","properties":{"value":{"default":"sk-secret12345"}}}),
        json!({"type":"object","$defs":{"nested":{"const":["bad\u{9b}31m"]}}}),
    ] {
        let fixture = Fixture::catalogue(vec![json!({"tools":[{
            "name":"unsafe","inputSchema":schema
        }]})]);
        let error = fixture.try_connect(&|| false).err().unwrap();
        assert!(
            error.contains("inputSchema") && error.contains("never redacted"),
            "{error}"
        );
    }
}

#[test]
fn composed_wire_name_accepts_64_characters_and_refuses_65_without_aliases() {
    let mut fixture = Fixture::catalogue(vec![json!({"tools":[catalogue_tool(&"t".repeat(25))]})]);
    fixture.registration.name = "s".repeat(32);
    let session = fixture.try_connect(&|| false).unwrap();
    let name = session.tools[0]["name"].as_str().unwrap();
    assert_eq!(
        format!("mcp__{}__{name}", fixture.registration.name).len(),
        64
    );
    assert_eq!(name, "t".repeat(25));
    let mut fixture = Fixture::catalogue(vec![json!({"tools":[catalogue_tool(&"t".repeat(26))]})]);
    fixture.registration.name = "s".repeat(32);
    let error = fixture.try_connect(&|| false).err().unwrap();
    assert!(
        error.contains("64-character") && error.contains("shorten"),
        "{error}"
    );
    assert!(validate_wire_name(&"s".repeat(25), &"t".repeat(32)).is_ok());
    assert!(validate_wire_name(&"s".repeat(26), &"t".repeat(32)).is_err());
    let fixture = Fixture::catalogue(vec![json!({"tools":[catalogue_tool("ghp_secret12345")]})]);
    assert!(fixture.try_connect(&|| false).is_err());
}
