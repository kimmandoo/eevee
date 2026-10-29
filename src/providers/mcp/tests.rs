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
