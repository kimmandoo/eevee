use super::*;
use optimizer::{Facts, POLICY, Plan, TOKENIZER, token_count};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::process::Command;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Real files are read through the same no-link reader and Git visibility check as
// ordinary captures. No provider, model, network client or replacement optimizer.
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "ev-optimizer-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        let path = fs::canonicalize(path).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .arg(&path)
                .status()
                .unwrap()
                .success()
        );
        Self(path)
    }

    fn report(&self, files: &[(&str, &str)]) -> Arc<repo::Report> {
        let mut identities = BTreeMap::new();
        for (name, text) in files {
            let path = PathBuf::from(name);
            fs::create_dir_all(self.0.join(&path).parent().unwrap()).unwrap();
            fs::write(self.0.join(&path), text.as_bytes()).unwrap();
            identities.insert(
                path.clone(),
                repo::FileIdentity {
                    path,
                    tracked: false,
                    head: None,
                    index: Vec::new(),
                    dirty: Some(true),
                    state: FileState::Regular {
                        sha256: Sha256::digest(text.as_bytes()).into(),
                        bytes: text.len() as u64,
                        binary: false,
                        executable: false,
                    },
                },
            );
        }
        let snapshot = Arc::new(repo::Snapshot {
            head: None,
            files: identities,
            working_diff: repo::WorkingDiff::default(),
            complete: true,
            stable: true,
            warnings: Vec::new(),
        });
        Arc::new(repo::Report {
            repository: repo::RepositoryIdentity {
                root: self.0.clone(),
                git_root: Some(self.0.clone()),
                git_dir: Some(self.0.join(".git")),
                storage_root: None,
            },
            baseline: Arc::clone(&snapshot),
            current: snapshot,
            changes: Vec::new(),
            initial: true,
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn facts(task_id: i64) -> Facts {
    Facts {
        task_id,
        basis: [19; 32],
        goal: "Repair subtotal underflow".into(),
        instructions: "Preserve the public API.".into(),
        ..Facts::default()
    }
}

fn optimize(
    report: &Arc<repo::Report>,
    pins: &[Excerpt],
    facts: Facts,
    budget: usize,
    cache: &[CacheRevision],
) -> Plan {
    match execute(
        facts.task_id,
        Operation::Optimize {
            report: Arc::clone(report),
            excerpts: pins.to_vec(),
            facts,
            budget,
            cache: cache.to_vec(),
        },
        &[],
        &AtomicBool::new(false),
    )
    .unwrap()
    {
        Response::Optimized(plan) => plan,
        other => panic!("expected actual optimized response, got {other:?}"),
    }
}

fn pin(report: &Arc<repo::Report>, path: &str, start: usize, end: usize, id: i64) -> Excerpt {
    let mut excerpt = capture(
        7,
        report,
        Path::new(path),
        start,
        end,
        &[],
        &AtomicBool::new(false),
    )
    .unwrap();
    excerpt.id = id;
    excerpt
}

fn padded_source(relevant: &str) -> String {
    let mut source = relevant.to_owned();
    // Valid, intentionally irrelevant Rust; materially larger than JSON overhead.
    for index in 0..180 {
        source.push_str(&format!("\n// archival display material retained independently of current calculations\npub fn archival_{index}() -> &'static str {{\n    \"independent historical display metadata with no bearing on calculations\"\n}}\n"));
    }
    source
}

fn multi_file_sources() -> Vec<(&'static str, String)> {
    vec![
        (
            "src/accounting.rs",
            padded_source(
                "pub fn normalize_total(amount: u32) -> u32 {\n    // subtotal underflow occurs at zero\n    amount - 1\n}\n",
            ),
        ),
        (
            "src/submit.rs",
            padded_source(
                "pub fn submit(amount: u32) -> u32 {\n    crate::accounting::normalize_total(amount)\n}\n",
            ),
        ),
        (
            "tests/balance.rs",
            padded_source(
                "#[test]\nfn zero_balance() {\n    let observed = crate::accounting::normalize_total(0);\n    assert_eq!(observed, 0);\n}\n",
            ),
        ),
        (
            "src/unrelated.rs",
            padded_source("pub fn unrelated() -> bool {\n    true\n}\n"),
        ),
    ]
}

#[test]
fn rejected_duplicate_syntax_ranges_keep_unique_revisions_without_conflating_files() {
    let fixture = Fixture::new();
    let source = format!(
        "pub fn subtotal() -> usize {{ let _ = \"{}\"; subtotal() }}\n",
        "subtotal supporting public context ".repeat(160),
    );
    let report = fixture.report(&[("src/first.rs", &source), ("src/distinct.rs", &source)]);
    let ranges = symbols::ranges("src/first.rs", &source, &[]);
    assert!(
        ranges
            .iter()
            .filter(|range| range.start_line == 1 && range.end_line == 1)
            .count()
            > 1
    );
    let full = optimize(&report, &[], facts(7), 100_000, &[]);
    assert_eq!(full.included.len(), 2);
    let mut mandatory: Value = serde_json::from_str(&full.packet).unwrap();
    mandatory["ranked_excerpts"] = json!([]);
    let budget = token_count(&mandatory.to_string()).max(64);
    let plan = optimize(&report, &[], facts(7), budget, &[]);
    assert!(!plan.overflow);
    assert!(plan.included.is_empty());
    assert!(plan.tokens <= budget);
    assert!(
        plan.omissions
            .iter()
            .filter(|reason| reason.contains(": budget;"))
            .count()
            > 2
    );
    let keys: std::collections::BTreeSet<_> =
        plan.revisions.iter().map(|row| &row.source_key).collect();
    assert_eq!(
        keys.len(),
        plan.revisions.len(),
        "rejected roles must not duplicate one exact range"
    );
    let files: Vec<_> = plan
        .revisions
        .iter()
        .filter(|row| row.source_key.starts_with("file:"))
        .collect();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].source_sha256, files[1].source_sha256);
    assert_ne!(files[0].source_key, files[1].source_key);
    assert_eq!(plan.baseline_tokens, token_count(&source) * 2);
    let warm = optimize(&report, &[], facts(7), budget, &plan.revisions);
    assert_eq!(warm.packet, plan.packet);
    assert_eq!(warm.revisions, plan.revisions);
    assert_eq!(
        warm.cache_hits, 3,
        "two persisted file counts plus one assembly manifest"
    );
}

#[test]
fn optimize_selects_real_defect_caller_and_check_below_full_file_uax29_baseline() {
    let fixture = Fixture::new();
    let sources = multi_file_sources();
    let files: Vec<_> = sources
        .iter()
        .map(|(path, text)| (*path, text.as_str()))
        .collect();
    let report = fixture.report(&files);
    let task = facts(7);
    // Neither caller nor check is supplied as a changed/failure path or pin.
    assert!(task.changed_paths.is_empty() && task.failure_paths.is_empty());
    let plan = optimize(&report, &[], task.clone(), 4_000, &[]);
    assert!(!plan.overflow);
    assert_eq!(plan.policy, optimizer::policy_identity(&[]));
    assert_eq!(plan.tokenizer, TOKENIZER);
    assert_eq!(plan.tokens, token_count(&plan.packet));
    assert!(plan.tokens <= plan.budget);
    let full_file_tokens: usize = sources.iter().map(|(_, text)| token_count(text)).sum();
    assert_eq!(plan.baseline_tokens, full_file_tokens);
    assert!(plan.tokens < plan.baseline_tokens);
    let packet: Value = serde_json::from_str(&plan.packet).unwrap();
    for (path, evidence) in [
        ("src/accounting.rs", "amount - 1"),
        ("src/submit.rs", "normalize_total(amount)"),
        ("tests/balance.rs", "normalize_total(0)"),
    ] {
        assert!(
            plan.included
                .iter()
                .any(|item| item.path == path && !item.mandatory),
            "missing {path}: {plan:?}"
        );
        assert!(
            packet["ranked_excerpts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| {
                    item["path"] == path
                        && item["text"]
                            .as_str()
                            .is_some_and(|text| text.contains(evidence))
                }),
            "missing runtime evidence for {path}"
        );
    }
    assert!(
        !plan
            .included
            .iter()
            .any(|item| item.path == "src/unrelated.rs")
    );
    assert!(
        plan.omissions
            .iter()
            .any(|reason| reason.contains("src/unrelated.rs"))
    );
    assert_eq!(packet["mandatory_instructions"], task.instructions);
    let dispatch = optimizer::planning_payload(7, Some(&report), &[], Some(&plan), &[]).unwrap();
    assert!(dispatch.dispatch);
    assert_eq!(dispatch.text, plan.packet);
    assert_eq!(dispatch.bytes, plan.packet.len());
    let repeated = optimize(&report, &[], task, plan.budget, &[]);
    assert_eq!(repeated.packet, plan.packet);
    assert_eq!(repeated.included, plan.included);
}

#[test]
fn applied_optimizer_packet_stops_on_selected_or_omitted_source_mutation() {
    for path in ["src/accounting.rs", "src/unrelated.rs"] {
        let fixture = Fixture::new();
        let sources = multi_file_sources();
        let files: Vec<_> = sources
            .iter()
            .map(|(path, text)| (*path, text.as_str()))
            .collect();
        let report = fixture.report(&files);
        let plan = optimize(&report, &[], facts(7), 4_000, &[]);
        assert!(plan.guards.iter().any(|guard| guard.path == path));
        assert_eq!(
            plan.included.iter().any(|item| item.path == path),
            path == "src/accounting.rs"
        );
        optimizer::planning_payload(7, Some(&report), &[], Some(&plan), &[]).unwrap();
        let old = fs::read_to_string(fixture.0.join(path)).unwrap();
        fs::write(
            fixture.0.join(path),
            format!("{old}\n// revision changed after preview\n"),
        )
        .unwrap();
        let error =
            optimizer::planning_payload(7, Some(&report), &[], Some(&plan), &[]).unwrap_err();
        assert!(
            error.contains(path) && error.contains("changed after context preview"),
            "{error}"
        );
        let error = execute(
            7,
            Operation::Optimize {
                report,
                excerpts: Vec::new(),
                facts: facts(7),
                budget: 4_000,
                cache: Vec::new(),
            },
            &[],
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert!(error.contains("changed since catalogue capture"), "{error}");
    }
}

#[test]
fn mandatory_instructions_and_pins_overflow_without_truncation_or_dispatch() {
    let fixture = Fixture::new();
    let source = format!(
        "// {}\npub fn retained() {{}}\n",
        "mandatory captured evidence ".repeat(500)
    );
    let report = fixture.report(&[("src/retained.rs", &source)]);
    let captured = pin(&report, "src/retained.rs", 1, 2, 41);
    for overflowing_pin in [false, true] {
        let pins = if overflowing_pin {
            vec![captured.clone()]
        } else {
            Vec::new()
        };
        let mut task = facts(7);
        if !overflowing_pin {
            task.instructions = "mandatory user instruction must remain exact ".repeat(500);
        }
        let plan = optimize(&report, &pins, task.clone(), 64, &[]);
        assert!(plan.overflow);
        assert!(plan.tokens > plan.budget);
        assert_eq!(plan.tokens, token_count(&plan.packet));
        assert!(plan.included.iter().all(|included| included.mandatory));
        let packet: Value = serde_json::from_str(&plan.packet).unwrap();
        assert_eq!(packet["mandatory_instructions"], task.instructions);
        assert!(packet["ranked_excerpts"].as_array().unwrap().is_empty());
        if overflowing_pin {
            assert_eq!(
                packet["selected_sources"]["captures"][0]["text"],
                captured.text
            );
            assert_eq!(plan.included.len(), 1);
            assert_eq!(plan.included[0].path, captured.path);
        }
        assert!(plan.review().contains("REFUSED"));
        assert!(optimizer::planning_payload(7, Some(&report), &pins, Some(&plan), &[]).is_err());
    }
}

fn memory(id: i64, project: &str, text: &str, reference: &str) -> crate::workspace::memory::Record {
    use crate::workspace::memory::{Proposal, Provenance, Record, Scope, SourceKind};
    Record {
        id,
        revision: 1,
        project: project.into(),
        task_id: Some(7),
        proposal: Proposal {
            scope: Scope::Task,
            content: text.into(),
            provenance: Provenance {
                kind: SourceKind::VerifiedNote,
                reference: reference.into(),
                task_id: 7,
            },
        },
    }
}

#[test]
fn mandatory_note_count_uses_live_structural_capture_identity_not_instruction_json() {
    let fixture = Fixture::new();
    let source = format!(
        "// {}\npub fn retained() {{}}\n",
        "exact adopted source ".repeat(70)
    );
    let report = fixture.report(&[("src/retained.rs", &source), ("src/distinct.rs", &source)]);
    let narrow = pin(&report, "src/retained.rs", 1, 1, 41);
    let wide = pin(&report, "src/retained.rs", 1, 2, 42);
    let pins = vec![narrow.clone(), wide.clone()];
    let attachment = planning_payload(7, Some(&report), &pins, &[]).unwrap();
    let selected: Value = serde_json::from_str(&attachment.text).unwrap();
    let mut task = facts(7);
    task.instructions = json!({"approved_memories": [{"content": narrow.text}]}).to_string();
    assert_eq!(
        optimizer::mandatory_memory_tokens(&task, &selected).unwrap(),
        0
    );
    let mut bound = memory(
        1,
        fixture.0.to_str().unwrap(),
        &narrow.text,
        "task:7/source:41",
    );
    bound.proposal.provenance.kind = crate::workspace::memory::SourceKind::UserAuthored;
    task.memories = vec![bound.clone()];
    let bound_tokens = optimizer::mandatory_memory_tokens(&task, &selected).unwrap();
    let mut envelope = json!({
        "task": {"id": 7}, "selected_sources": selected, "approved_memories": task.memories,
    });
    optimizer::deduplicate_context(&mut envelope);
    assert_eq!(
        envelope["approved_memories"][0]["proposal"]["content_reference"]["same_bytes_as"],
        wide.id
    );
    assert_eq!(
        bound_tokens,
        token_count(&envelope["approved_memories"].to_string())
    );
    let plan = optimize(&report, &pins, task.clone(), 20_000, &[]);
    let packet: Value = serde_json::from_str(&plan.packet).unwrap();
    assert_eq!(packet["mandatory_instructions"], task.instructions);
    assert_eq!(plan.mandatory_note_tokens, bound_tokens);

    for mismatch in ["unbound", "task", "content", "distinct capture"] {
        let mut distinct = bound.clone();
        match mismatch {
            "unbound" => distinct.proposal.provenance.reference = "user-authored note".into(),
            "task" => distinct.proposal.provenance.task_id = 8,
            "content" => distinct.proposal.content.push_str(" edited"),
            "distinct capture" => {
                distinct.proposal.provenance.reference = "task:7/source:43".into()
            }
            _ => unreachable!(),
        }
        task.memories = vec![distinct.clone()];
        let tokens = optimizer::mandatory_memory_tokens(&task, &selected).unwrap();
        assert_eq!(
            tokens,
            token_count(&json!([distinct]).to_string()),
            "{mismatch}"
        );
        assert!(tokens > bound_tokens, "{mismatch}");
    }
}

#[test]
fn covered_capture_memory_reference_preserves_exact_child_range_and_source_identity() {
    let fixture = Fixture::new();
    let source =
        "// carrier prefix\r\npub fn child() {\r\n    retained();\r\n}\r\n// carrier suffix\r\n";
    let report = fixture.report(&[("src/first.rs", source), ("src/distinct.rs", source)]);
    let child = pin(&report, "src/first.rs", 2, 3, 41);
    let carrier = pin(&report, "src/first.rs", 1, 5, 42);
    let distinct = pin(&report, "src/distinct.rs", 1, 5, 43);
    let pins = vec![child.clone(), carrier.clone(), distinct.clone()];
    let attachment = planning_payload(7, Some(&report), &pins, &[]).unwrap();
    let selected: Value = serde_json::from_str(&attachment.text).unwrap();
    assert_eq!(selected["captures"][0]["same_bytes_as"], carrier.id);
    let mut adopted = memory(
        1,
        fixture.0.to_str().unwrap(),
        &child.text,
        "task:7/source:41",
    );
    adopted.proposal.provenance.kind = crate::workspace::memory::SourceKind::UserAuthored;
    assert!(crate::workspace::memory::validate_record(&adopted).is_ok());
    let input = json!({
        "task": {"id": 7}, "selected_sources": selected, "approved_memories": [adopted],
    });
    let encoded = optimizer::encode_context(input).unwrap();
    let output: Value = serde_json::from_str(&encoded).unwrap();
    let proposal = &output["approved_memories"][0]["proposal"];
    assert!(proposal.get("content").is_none());
    let reference = &proposal["content_reference"];
    assert_eq!(
        reference,
        &json!({
            "same_bytes_as": carrier.id,
            "start_line": child.start_line,
            "end_line": child.end_line,
        })
    );
    assert_eq!(
        proposal["provenance"]["reference"],
        format!("task:7/source:{}", child.id)
    );
    assert_eq!(proposal["provenance"]["task_id"], child.task_id);
    let captures = output["selected_sources"]["captures"].as_array().unwrap();
    let canonical = captures
        .iter()
        .find(|capture| capture["id"] == reference["same_bytes_as"])
        .unwrap();
    assert!(canonical.get("same_bytes_as").is_none());
    assert_eq!(canonical["path"], child.path);
    assert_eq!(canonical["source_sha256"], sha256_hex(&child.source_sha256));
    let outer_start = canonical["start_line"].as_u64().unwrap();
    let resolved = line_range(
        canonical["text"].as_str().unwrap(),
        (reference["start_line"].as_u64().unwrap() - outer_start + 1) as usize,
        (reference["end_line"].as_u64().unwrap() - outer_start + 1) as usize,
    )
    .unwrap();
    assert_eq!(resolved, child.text);
    assert_eq!(
        resolved.as_bytes(),
        b"pub fn child() {\r\n    retained();\r\n"
    );
    assert_ne!(resolved, carrier.text);
    assert_eq!(captures[2]["id"], distinct.id);
    assert_eq!(captures[2]["text"], distinct.text);
    assert!(captures[2].get("same_bytes_as").is_none());
    assert_ne!(reference["same_bytes_as"], distinct.id);
    assert_eq!(optimizer::encode_context(output).unwrap(), encoded);
}

#[test]
fn mandatory_notes_reserve_exact_budget_and_keep_cache_rows_packet_only() {
    let fixture = Fixture::new();
    let source = "pub fn retained() {}\n";
    let report = fixture.report(&[("src/retained.rs", source)]);
    let captured = pin(&report, "src/retained.rs", 1, 1, 41);
    let pins = vec![captured.clone()];
    let mut task = facts(7);
    task.memories = vec![memory(
        1,
        fixture.0.to_str().unwrap(),
        &"mandatory distinct note ".repeat(80),
        "user-authored note",
    )];
    let preview = optimize(&report, &pins, task.clone(), 20_000, &[]);
    assert!(preview.mandatory_note_tokens > 0);
    assert_eq!(preview.packet_tokens, token_count(&preview.packet));
    assert_eq!(
        preview.tokens,
        preview.packet_tokens + preview.mandatory_note_tokens
    );
    let exact = optimize(&report, &pins, task.clone(), preview.tokens, &[]);
    assert!(!exact.overflow);
    assert_eq!(exact.tokens, exact.budget);
    assert!(optimizer::planning_payload(7, Some(&report), &pins, Some(&exact), &[]).is_ok());
    let warm = optimize(&report, &pins, task.clone(), exact.budget, &exact.revisions);
    assert_eq!(warm.packet, exact.packet);
    assert_eq!(warm.packet_tokens, exact.packet_tokens);
    assert_eq!(warm.mandatory_note_tokens, exact.mandatory_note_tokens);
    assert_eq!(warm.tokens, exact.tokens);
    assert!(warm.cache_hits > 0);
    for row in warm
        .revisions
        .iter()
        .filter(|row| row.assembly.is_some() || row.source_key.starts_with("assembly:"))
    {
        assert_eq!(row.tokens, exact.packet_tokens);
        assert_eq!(
            row.source_sha256,
            sha256_hex(&Sha256::digest(exact.packet.as_bytes()).into())
        );
    }
    let refused = optimize(&report, &pins, task, exact.budget - 1, &[]);
    assert!(refused.overflow);
    assert_eq!(refused.tokens, exact.tokens);
    assert_eq!(refused.packet, exact.packet);
    assert!(optimizer::planning_payload(7, Some(&report), &pins, Some(&refused), &[]).is_err());
    for dimension in ["packet count", "total", "note count"] {
        let mut invalid = exact.clone();
        match dimension {
            "packet count" => invalid.packet_tokens += 1,
            "total" => invalid.tokens -= 1,
            "note count" => invalid.mandatory_note_tokens -= 1,
            _ => unreachable!(),
        }
        assert!(
            optimizer::planning_payload(7, Some(&report), &pins, Some(&invalid), &[]).is_err(),
            "{dimension}"
        );
    }
}

#[test]
fn optional_ranked_selection_cannot_spend_mandatory_note_reserve() {
    let fixture = Fixture::new();
    let source = format!(
        "pub fn normalize_total(amount: u32) -> u32 {{\n    // {}\n    amount - 1\n}}\n",
        "subtotal underflow evidence ".repeat(160),
    );
    let report = fixture.report(&[("src/accounting.rs", &source)]);
    let plain = optimize(&report, &[], facts(7), 20_000, &[]);
    assert!(plain.included.iter().any(|item| {
        !item.mandatory
            && item.path == "src/accounting.rs"
            && item.start_line == 1
            && item.end_line == 4
    }));
    let mut task = facts(7);
    task.memories = vec![memory(
        1,
        fixture.0.to_str().unwrap(),
        &"mandatory separate note ".repeat(80),
        "user-authored note",
    )];
    let reserve = optimizer::mandatory_memory_tokens(&task, &json!({})).unwrap();
    let budget = plain.tokens;
    let reserved = optimize(&report, &[], task.clone(), budget, &[]);
    assert!(!reserved.overflow);
    assert!(!reserved.included.iter().any(|item| {
        item.path == "src/accounting.rs" && item.start_line == 1 && item.end_line == 4
    }));
    assert_eq!(reserved.mandatory_note_tokens, reserve);
    assert!(reserved.packet_tokens + reserve <= budget);
    assert_eq!(reserved.tokens, reserved.packet_tokens + reserve);
    assert!(
        reserved
            .omissions
            .iter()
            .any(|omission| omission.starts_with("src/accounting.rs:1-4: budget"))
    );
    let warm = optimize(&report, &[], task, budget, &reserved.revisions);
    assert_eq!(warm.packet, reserved.packet);
    assert_eq!(warm.tokens, reserved.tokens);
    assert_eq!(warm.mandatory_note_tokens, reserve);
}

#[test]
fn selected_note_edits_revision_provenance_and_selection_invalidate_frozen_input_basis() {
    let fixture = Fixture::new();
    let report = fixture.report(&[("src/retained.rs", "pub fn retained() {}\n")]);
    let mut task = crate::store::Snapshot::empty(fixture.0.to_str().unwrap().into());
    task.id = 7;
    task.revision = 1;
    task.goal = "Retain source".into();
    task.approved_memories = vec![memory(
        1,
        &task.project,
        "selected approved note",
        "user-authored note",
    )];
    let original = Facts::from_task(&task);
    let instructions: Value = serde_json::from_str(&original.instructions).unwrap();
    assert!(instructions.get("approved_memories").is_none());
    assert_eq!(original.memories, task.approved_memories);
    let plan = optimize(&report, &[], original.clone(), 8_000, &[]);
    let input = crate::providers::Input {
        task: task.clone(),
        work: None,
        local_only: true,
        instruction_override: None,
        excerpts: Vec::new(),
        context_plan: Some(plan.clone()),
        screenshot: None,
        tool_catalog: None,
        report: Some(Arc::clone(&report)),
        resume: None,
        agent: crate::providers::Agent::Codex,
        operation: crate::providers::Operation::Plan,
        edit_paths: Vec::new(),
        handoff: None,
        inspect_sessions: false,
        inspect_models: false,
        selection: Default::default(),
        conversation: None,
        prior_plan: None,
        public_context: None,
        credential: None,
    };
    assert!(input.source_payload(&[]).is_ok());
    for dimension in ["content", "revision", "provenance", "selection"] {
        let mut changed = input.clone();
        match dimension {
            "content" => changed.task.approved_memories[0]
                .proposal
                .content
                .push_str(" edited"),
            "revision" => changed.task.approved_memories[0].revision += 1,
            "provenance" => {
                changed.task.approved_memories[0]
                    .proposal
                    .provenance
                    .reference = "edited user note".into()
            }
            "selection" => changed.task.approved_memories.clear(),
            _ => unreachable!(),
        }
        let updated = Facts::from_task(&changed.task);
        assert_eq!(updated.instructions, original.instructions);
        assert_ne!(updated.basis, original.basis, "{dimension}");
        assert!(
            changed
                .source_payload(&[])
                .unwrap_err()
                .contains("basis changed"),
            "{dimension}"
        );
        let rebuilt = optimize(&report, &[], updated, plan.budget, &plan.revisions);
        assert_eq!(rebuilt.packet, plan.packet);
        assert!(
            rebuilt
                .revisions
                .iter()
                .find(|row| row.assembly.is_some())
                .unwrap()
                .source_key
                != plan
                    .revisions
                    .iter()
                    .find(|row| row.assembly.is_some())
                    .unwrap()
                    .source_key
        );
    }
    assert_eq!(input.task.approved_memories, original.memories);
}

#[test]
fn compaction_reserves_actual_ledger_bound_notes_and_refuses_one_token_over_budget() {
    let fixture = Fixture::new();
    let source = format!(
        "// {}\npub fn retained() {{}}\n",
        "immutable source note ".repeat(60)
    );
    let report = fixture.report(&[("src/retained.rs", &source)]);
    let capture = pin(&report, "src/retained.rs", 1, 2, 41);
    let mut task = crate::store::Snapshot::empty(fixture.0.to_str().unwrap().into());
    task.id = 7;
    task.revision = 1;
    task.goal = "Retain historical source evidence".into();
    let mut adopted = memory(1, &task.project, &capture.text, "task:7/source:41");
    adopted.proposal.provenance.kind = crate::workspace::memory::SourceKind::UserAuthored;
    task.approved_memories = vec![adopted];
    let compact = |task: &crate::store::Snapshot, budget| match execute(
        7,
        Operation::Compact {
            report: Arc::clone(&report),
            task: Box::new(task.clone()),
            excerpts: vec![capture.clone()],
            runs: Vec::new(),
            budget,
        },
        &[],
        &AtomicBool::new(false),
    )
    .unwrap()
    {
        Response::Optimized(plan) => plan,
        other => panic!("expected compact plan, got {other:?}"),
    };
    let preview = compact(&task, 20_000);
    let ledger: Value = serde_json::from_str(&preview.packet).unwrap();
    let mut envelope = json!({
        "task": {"id": 7}, "selected_sources": ledger, "approved_memories": task.approved_memories,
    });
    optimizer::deduplicate_context(&mut envelope);
    assert!(
        envelope["approved_memories"][0]["proposal"]
            .get("content")
            .is_none()
    );
    assert_eq!(preview.packet_tokens, token_count(&preview.packet));
    assert_eq!(
        preview.mandatory_note_tokens,
        token_count(&envelope["approved_memories"].to_string())
    );
    assert_eq!(
        preview.tokens,
        preview.packet_tokens + preview.mandatory_note_tokens
    );
    let exact = compact(&task, preview.tokens);
    assert_eq!(exact.tokens, exact.budget);
    assert!(!exact.overflow);
    assert!(
        optimizer::planning_payload(
            7,
            Some(&report),
            std::slice::from_ref(&capture),
            Some(&exact),
            &[]
        )
        .is_ok()
    );
    let refused = compact(&task, preview.tokens - 1);
    assert!(refused.overflow);
    assert_eq!(refused.packet, exact.packet);
    assert!(
        optimizer::planning_payload(
            7,
            Some(&report),
            std::slice::from_ref(&capture),
            Some(&refused),
            &[]
        )
        .is_err()
    );
    task.approved_memories[0].proposal.provenance.reference = "edited user-authored note".into();
    let distinct = compact(&task, preview.tokens);
    assert_eq!(distinct.packet, exact.packet);
    assert!(distinct.mandatory_note_tokens > exact.mandatory_note_tokens);
    assert!(distinct.overflow);
    assert_ne!(distinct.basis, exact.basis);
}

#[test]
fn actual_compaction_and_accepted_handoff_share_one_capture_with_strict_digest_guards() {
    let fixture = Fixture::new();
    let source = "pub fn retained() -> u32 {\n    42 // exact retained source evidence, not approval or a current verification claim\n}\n";
    let report = fixture.report(&[("src/retained.rs", source)]);
    let capture = pin(&report, "src/retained.rs", 1, 3, 41);
    let mut task = crate::store::Snapshot::empty(fixture.0.to_str().unwrap().into());
    task.id = 7;
    task.revision = 1;
    task.goal = "Retain historical source evidence".into();
    let Response::Optimized(plan) = execute(
        7,
        Operation::Compact {
            report: Arc::clone(&report),
            task: Box::new(task.clone()),
            excerpts: vec![capture.clone()],
            runs: Vec::new(),
            budget: 8_000,
        },
        &[],
        &AtomicBool::new(false),
    )
    .unwrap() else {
        panic!("expected an actual compact plan");
    };
    let ledger: Value = serde_json::from_str(&plan.packet).unwrap();
    assert!(ledger["captures"][0]["source_sha256"].is_array());
    let accepted =
        crate::workspace::handoff::from_task(&task, std::slice::from_ref(&capture), &[]).unwrap();
    let input = json!({
        "task": {"id": task.id},
        "selected_sources": ledger,
        "accepted_handoff": accepted,
    });
    let encoded = optimizer::encode_context(input.clone()).unwrap();
    let output: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        output["accepted_handoff"]["sources"][0]["same_bytes_as"],
        capture.id
    );
    assert!(
        output["accepted_handoff"]["sources"][0]
            .get("text")
            .is_none()
    );
    assert_eq!(
        output["selected_sources"]["captures"][0]["text"],
        capture.text
    );
    assert_eq!(encoded.matches(&json!(capture.text).to_string()).count(), 1);
    assert_eq!(optimizer::encode_context(output).unwrap(), encoded);

    let mut malformed_extra = json!(capture.source_sha256);
    malformed_extra.as_array_mut().unwrap().push(Value::Null);
    let mut malformed_element = json!(capture.source_sha256);
    malformed_element[0] = json!("0");
    let mut malformed_byte = json!(capture.source_sha256);
    malformed_byte[0] = json!(256);
    let mut foreign = capture.source_sha256;
    foreign[0] ^= 1;
    for invalid_hash in [
        malformed_extra,
        malformed_element,
        malformed_byte,
        json!(&capture.source_sha256[..31]),
        json!(foreign),
        json!("g".repeat(64)),
        json!("0".repeat(63)),
        Value::Null,
    ] {
        for side in ["source", "capture"] {
            let mut changed = input.clone();
            if side == "source" {
                changed["accepted_handoff"]["sources"][0]["source_sha256"] = invalid_hash.clone();
            } else {
                changed["selected_sources"]["captures"][0]["source_sha256"] = invalid_hash.clone();
            }
            let expected = changed["accepted_handoff"]["sources"][0].clone();
            optimizer::deduplicate_context(&mut changed);
            assert_eq!(
                changed["accepted_handoff"]["sources"][0], expected,
                "{side}: {invalid_hash}"
            );
        }
    }
    for dimension in ["owner", "path", "range"] {
        let mut changed = input.clone();
        match dimension {
            "owner" => changed["accepted_handoff"]["task"]["id"] = json!(8),
            "path" => changed["accepted_handoff"]["sources"][0]["path"] = json!("src/elsewhere.rs"),
            "range" => changed["accepted_handoff"]["sources"][0]["start_line"] = json!(2),
            _ => unreachable!(),
        }
        let expected = changed["accepted_handoff"]["sources"][0].clone();
        optimizer::deduplicate_context(&mut changed);
        assert_eq!(
            changed["accepted_handoff"]["sources"][0], expected,
            "{dimension}"
        );
    }
}

fn assert_resolvable_aliases(value: &Value, captures: &BTreeMap<i64, Value>) -> usize {
    match value {
        Value::Object(map) => {
            let mut aliases = 0;
            if let Some(target) = map.get("same_bytes_as") {
                let id = target
                    .as_i64()
                    .expect("capture aliases use numeric capture IDs");
                let canonical = captures
                    .get(&id)
                    .expect("every alias resolves within the same outgoing packet");
                assert!(
                    canonical["text"].is_string(),
                    "aliases must resolve directly to retained bytes: {canonical}"
                );
                assert!(canonical.get("same_bytes_as").is_none(), "no alias chains");
                aliases += 1;
            }
            aliases
                + map
                    .values()
                    .map(|value| assert_resolvable_aliases(value, captures))
                    .sum::<usize>()
        }
        Value::Array(items) => items
            .iter()
            .map(|item| assert_resolvable_aliases(item, captures))
            .sum(),
        _ => 0,
    }
}

#[test]
fn explicit_capture_provenance_deduplicates_attachment_memory_and_handoff_without_path_conflation()
{
    let fixture = Fixture::new();
    let source = "pub fn shared_bytes() -> u32 {\n    42 // Immutable source payload shared by the explicit attachment, approved note and handoff; equal content from another path retains its own independent source identity.\n}\n";
    let report = fixture.report(&[("src/first.rs", source), ("src/distinct.rs", source)]);
    let first = pin(&report, "src/first.rs", 1, 3, 11);
    let mut duplicate = first.clone();
    duplicate.id = 12;
    let distinct = pin(&report, "src/distinct.rs", 1, 3, 13);
    let attachment = planning_payload(
        7,
        Some(&report),
        &[first.clone(), duplicate.clone(), distinct.clone()],
        &[],
    )
    .unwrap();
    let selected_sources: Value = serde_json::from_str(&attachment.text).unwrap();
    assert_eq!(selected_sources["captures"][1]["same_bytes_as"], first.id);
    assert_eq!(selected_sources["captures"][2]["text"], distinct.text);
    assert!(
        selected_sources["captures"][2]
            .get("same_bytes_as")
            .is_none()
    );
    let project = fixture.0.to_str().unwrap();
    let records = vec![
        memory(1, project, source, "task:7/source:11"),
        memory(2, project, source, "task:7/source:12"),
        memory(3, project, source, "task:8/source:11"),
        memory(
            4,
            project,
            source,
            "user-authored equal text without capture provenance",
        ),
    ];
    let approved = crate::workspace::memory::snapshot(&records, &[1, 2, 3, 4], project, 7).unwrap();
    let mut handoff = serde_json::to_value(&duplicate).unwrap();
    handoff["excerpt_id"] = json!(duplicate.id);
    let mut wrong_handoff = handoff.clone();
    wrong_handoff["path"] = json!("src/elsewhere.rs");
    let input = json!({
        "task": {"id": 7}, "selected_sources": selected_sources,
        "approved_memories": approved,
        "handoff": {"task": {"id": 7}, "sources": [handoff, wrong_handoff]},
        "ordinary_fact": source,
    });
    let encoded = optimizer::encode_context(input.clone()).unwrap();
    let output: Value = serde_json::from_str(&encoded).unwrap();
    let captures: BTreeMap<_, _> = output["selected_sources"]["captures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|capture| (capture["id"].as_i64().unwrap(), capture.clone()))
        .collect();
    assert_eq!(assert_resolvable_aliases(&output, &captures), 4);
    for index in [0, 1] {
        let proposal = &output["approved_memories"][index]["proposal"];
        assert!(proposal.get("content").is_none());
        assert_eq!(
            proposal["content_reference"],
            json!({"same_bytes_as": first.id})
        );
        assert_eq!(
            proposal["provenance"],
            input["approved_memories"][index]["proposal"]["provenance"]
        );
    }
    assert_eq!(output["handoff"]["sources"][0]["same_bytes_as"], first.id);
    assert_eq!(output["handoff"]["sources"][0]["excerpt_id"], duplicate.id);
    for index in [2, 3] {
        assert_eq!(
            output["approved_memories"][index],
            input["approved_memories"][index]
        );
    }
    assert_eq!(
        output["handoff"]["sources"][1],
        input["handoff"]["sources"][1]
    );
    assert_eq!(output["ordinary_fact"], source);
    assert_eq!(output["selected_sources"]["captures"][2]["text"], source);
    assert!(token_count(&encoded) < token_count(&serde_json::to_string(&input).unwrap()));
    // Re-encoding must not create dangling references or consume another copy.
    assert_eq!(optimizer::encode_context(output).unwrap(), encoded);
}

fn store_event(worker: &crate::store::Worker) -> crate::store::Event {
    let deadline = Instant::now() + Duration::from_secs(35);
    loop {
        if let Some(event) = worker.try_event() {
            return event;
        }
        assert!(
            Instant::now() < deadline,
            "context-cache storage observation timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn open_store(path: &Path, project: &str) -> (crate::store::Worker, crate::store::Project) {
    let worker = crate::store::Worker::start(
        path.to_path_buf(),
        project.into(),
        crate::store::Policy::default(),
    )
    .unwrap();
    let crate::store::Event::Loaded(loaded) = store_event(&worker) else {
        panic!("store did not load")
    };
    (worker, loaded)
}

fn cache_request(
    worker: &crate::store::Worker,
    task_id: i64,
    operation: crate::store::context_cache::Operation,
) -> Vec<CacheRevision> {
    let mut pending = Some(crate::store::context_cache::Request { task_id, operation });
    assert!(worker.try_context_cache(&mut pending));
    assert!(pending.is_none());
    match store_event(worker) {
        crate::store::Event::ContextCache { request, result } => {
            assert_eq!(request.task_id, task_id);
            result.unwrap().rows
        }
        other => panic!("expected actual persisted cache response: {other:?}"),
    }
}

fn sorted_rows(rows: &[CacheRevision]) -> Vec<CacheRevision> {
    let mut sorted = rows.to_vec();
    sorted.sort_by(|left, right| left.source_key.cmp(&right.source_key));
    sorted
}

#[test]
fn large_assembly_metadata_survives_real_store_restart_and_bypasses_ranking() {
    use crate::store::context_cache::Operation as CacheOperation;
    let fixture = Fixture::new();
    let sources: Vec<_> = (0..4)
        .map(|index| {
            (
                format!("src/archive_{index}.rs"),
                padded_source("pub fn current_calculation() -> u32 {\n    42\n}\n"),
            )
        })
        .collect();
    let files: Vec<_> = sources
        .iter()
        .map(|(path, source)| (path.as_str(), source.as_str()))
        .collect();
    let report = fixture.report(&files);
    let storage = fixture.0.join("local-data");
    let project = fixture.0.to_str().unwrap();
    let (worker, _) = open_store(&storage, project);
    let mut snapshot = crate::store::Snapshot::empty(project.into());
    snapshot.goal = "Review archival historical display metadata calculations".into();
    snapshot.revision = 1;
    let mut pending = Some(snapshot);
    assert!(worker.try_save(&mut pending));
    let crate::store::Event::Saved(task) = store_event(&worker) else {
        panic!("task was not saved")
    };
    let mut selection_facts = facts(task.id);
    selection_facts.goal = task.goal;
    selection_facts.instructions =
        "Retain independent historical display metadata and archival calculations.".into();
    let initial = optimize(&report, &[], selection_facts.clone(), 4_000, &[]);
    assert!(!initial.overflow);
    assert_eq!(initial.cache_hits, 0);
    assert!(!initial.included.is_empty());
    assert!(!initial.omissions.is_empty());
    let manifests: Vec<_> = initial
        .revisions
        .iter()
        .filter(|row| row.assembly.is_some())
        .collect();
    assert_eq!(manifests.len(), 1);
    let manifest = manifests[0];
    let identity = manifest.source_key.strip_prefix("manifest:").unwrap();
    assert_eq!(identity.len(), 64);
    assert!(identity.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(!manifest.source_key.contains(['\n', '{', '}']));
    assert_eq!(
        manifest.source_sha256,
        super::sha256_hex(&Sha256::digest(initial.packet.as_bytes()).into())
    );
    let metadata = manifest.assembly.as_ref().unwrap();
    assert!(
        metadata.len() > 65_536,
        "real ranked metadata must cross the old source-key persistence ceiling"
    );
    assert!(metadata.len() < optimizer::MAX_ASSEMBLY_METADATA_BYTES);
    optimizer::validate_assembly_metadata(metadata).unwrap();
    let decoded: Value = serde_json::from_str(metadata).unwrap();
    assert_eq!(decoded["included"], json!(initial.included));
    assert_eq!(decoded["omissions"], json!(initial.omissions));
    assert!(
        initial
            .revisions
            .iter()
            .filter(|row| row.source_key != manifest.source_key)
            .all(|row| row.assembly.is_none())
    );
    let saved = cache_request(
        &worker,
        task.id,
        CacheOperation::Save(initial.revisions.clone()),
    );
    assert_eq!(saved, sorted_rows(&initial.revisions));
    assert_eq!(cache_request(&worker, task.id, CacheOperation::Load), saved);
    drop(worker);

    let (worker, loaded) = open_store(&storage, project);
    assert_eq!(loaded.tasks[0].id, task.id);
    let restored = cache_request(&worker, task.id, CacheOperation::Load);
    assert_eq!(restored, saved);
    let warm = optimize(&report, &[], selection_facts, initial.budget, &restored);
    assert_eq!(warm.packet, initial.packet);
    assert_eq!(warm.tokens, initial.tokens);
    assert_eq!(warm.included, initial.included);
    assert_eq!(warm.omissions, initial.omissions);
    assert_eq!(warm.guards, initial.guards);
    assert_eq!(warm.revisions, restored);
    assert_eq!(
        warm.cache_hits,
        files.len() + 1,
        "only full-file token rows and the assembly may hit: parsing/ranking fallback would also hit excerpt rows"
    );
    let warm_manifest = warm
        .revisions
        .iter()
        .find(|row| row.assembly.is_some())
        .unwrap();
    assert_eq!(warm_manifest.source_key, manifest.source_key);
}

#[test]
fn assembly_metadata_validation_is_strict_and_counts_utf8_bytes_at_the_limit() {
    let valid = json!({"included": [], "omissions": []}).to_string();
    optimizer::validate_assembly_metadata(&valid).unwrap();
    for invalid in [
        json!({"included": [], "omissions": [], "credential": "secret"}),
        json!({"included": [], "omissions": [], "text": "source body"}),
        json!({
            "included": [{
                "path": "src/archive.rs",
                "start_line": 1,
                "end_line": 3,
                "reason": "definition; 1 goal/instruction matches",
                "mandatory": false,
                "credential": "secret"
            }],
            "omissions": []
        }),
    ] {
        assert!(
            optimizer::validate_assembly_metadata(&invalid.to_string()).is_err(),
            "unknown fields must not smuggle credentials or source bodies"
        );
    }
    for unsafe_text in [
        "token=private-value",
        "[REDACTED]",
        "\u{1b}[31m",
        "unsafe\u{202e}text",
    ] {
        let omission = json!({"included": [], "omissions": [unsafe_text]});
        assert!(optimizer::validate_assembly_metadata(&omission.to_string()).is_err());
        let reason = json!({
            "included": [{
                "path": "src/archive.rs",
                "start_line": 1,
                "end_line": 3,
                "reason": unsafe_text,
                "mandatory": false
            }],
            "omissions": []
        });
        assert!(optimizer::validate_assembly_metadata(&reason.to_string()).is_err());
    }
    let limit = optimizer::MAX_ASSEMBLY_METADATA_BYTES;
    assert_eq!(limit, 4 * 1024 * 1024);
    let mut exact = valid.clone();
    exact.push_str(&" ".repeat(limit - exact.len()));
    assert_eq!(exact.len(), limit);
    optimizer::validate_assembly_metadata(&exact).unwrap();
    exact.push(' ');
    assert!(optimizer::validate_assembly_metadata(&exact).is_err());

    let multibyte = json!({"included": [], "omissions": ["界".repeat(limit / 3)]}).to_string();
    assert!(multibyte.len() > limit);
    assert!(multibyte.chars().count() < limit);
    assert!(optimizer::validate_assembly_metadata(&multibyte).is_err());
}

#[test]
fn persisted_revision_cache_survives_store_restart_and_invalidates_source_policy_and_tokenizer() {
    use crate::store::context_cache::Operation as CacheOperation;
    let fixture = Fixture::new();
    let source = padded_source(
        "pub fn normalize_total(amount: u32) -> u32 {\n    // subtotal underflow\n    amount - 1\n}\n",
    );
    let report = fixture.report(&[("src/accounting.rs", &source)]);
    let storage = fixture.0.join("local-data");
    let project = fixture.0.to_str().unwrap();
    let (worker, loaded) = open_store(&storage, project);
    assert!(loaded.tasks.is_empty());
    let mut task = crate::store::Snapshot::empty(project.into());
    task.goal = "Repair subtotal underflow".into();
    task.revision = 1;
    let mut pending = Some(task);
    assert!(worker.try_save(&mut pending));
    let crate::store::Event::Saved(task) = store_event(&worker) else {
        panic!("task was not saved")
    };
    let initial = optimize(&report, &[], facts(task.id), 4_000, &[]);
    assert_eq!(initial.cache_hits, 0);
    assert!(
        initial.revisions.len() > 1,
        "fixture must cache excerpt and assembly rows"
    );
    assert!(
        initial
            .revisions
            .iter()
            .any(|row| row.source_key.starts_with("assembly:"))
    );
    assert!(
        initial
            .revisions
            .iter()
            .any(|row| !row.source_key.starts_with("assembly:"))
    );
    let persisted = cache_request(
        &worker,
        task.id,
        CacheOperation::Save(initial.revisions.clone()),
    );
    assert_eq!(persisted, sorted_rows(&initial.revisions));
    drop(worker);

    let (worker, loaded) = open_store(&storage, project);
    assert_eq!(loaded.tasks[0].id, task.id);
    let restored = cache_request(&worker, task.id, CacheOperation::Load);
    assert_eq!(restored, persisted);
    let warm = optimize(&report, &[], facts(task.id), initial.budget, &restored);
    assert_eq!(warm.packet, initial.packet);
    assert_eq!(warm.tokens, initial.tokens);
    assert_eq!(
        warm.cache_hits, 2,
        "warm cache reuses the full-file token row and assembly manifest"
    );
    for dimension in ["source revision", "policy", "tokenizer"] {
        let mut invalidated = restored.clone();
        for row in &mut invalidated {
            match dimension {
                "source revision" => row.source_sha256 = "0".repeat(64),
                "policy" => row.policy = "previous-context-policy".into(),
                "tokenizer" => row.tokenizer = "previous-tokenizer".into(),
                _ => unreachable!(),
            }
        }
        let stored = cache_request(&worker, task.id, CacheOperation::Save(invalidated.clone()));
        assert_eq!(stored, sorted_rows(&invalidated));
        let cold = optimize(&report, &[], facts(task.id), initial.budget, &stored);
        assert_eq!(
            cold.cache_hits, 0,
            "{dimension} must invalidate real engine hits"
        );
        assert_eq!(cold.packet, initial.packet);
        assert_eq!(cold.tokens, token_count(&cold.packet));
        assert!(
            cold.revisions
                .iter()
                .all(|row| row.policy == POLICY && row.tokenizer == TOKENIZER)
        );
    }
    let changed_configuration =
        optimize(&report, &[], facts(task.id), initial.budget + 1, &restored);
    assert_eq!(changed_configuration.cache_hits, 0);
    let revised = source.replacen("amount - 1", "amount.saturating_sub(1)", 1);
    let revised_report = fixture.report(&[("src/accounting.rs", &revised)]);
    let changed_source = optimize(
        &revised_report,
        &[],
        facts(task.id),
        initial.budget,
        &restored,
    );
    assert_eq!(changed_source.cache_hits, 0);
    assert_ne!(changed_source.packet, initial.packet);
    let current = cache_request(
        &worker,
        task.id,
        CacheOperation::Save(changed_source.revisions.clone()),
    );
    assert_eq!(current, sorted_rows(&changed_source.revisions));
    assert!(
        restored
            .iter()
            .all(|old| current.iter().all(|new| old.source_key != new.source_key))
    );
    drop(worker);

    let (worker, _) = open_store(&storage, project);
    let restarted = cache_request(&worker, task.id, CacheOperation::Load);
    assert_eq!(restarted, current);
    let warm_revised = optimize(
        &revised_report,
        &[],
        facts(task.id),
        initial.budget,
        &restarted,
    );
    assert_eq!(
        warm_revised.cache_hits, 2,
        "restarted cache reuses the revised full-file row and assembly manifest"
    );
    assert_eq!(warm_revised.packet, changed_source.packet);
    assert!(cache_request(&worker, task.id, CacheOperation::Purge).is_empty());
    assert_eq!(
        cache_request(&worker, task.id, CacheOperation::Save(current.clone())),
        current
    );
    let mut deletion = Some(crate::store::Management::Delete {
        ids: vec![task.id],
        expired_before: None,
    });
    assert!(worker.try_manage(&mut deletion));
    match store_event(&worker) {
        crate::store::Event::Managed {
            project, affected, ..
        } => {
            assert_eq!(affected, vec![task.id]);
            assert!(project.tasks.is_empty());
        }
        other => panic!("expected task deletion to clean its revision index: {other:?}"),
    }
    drop(worker);
    let (worker, loaded) = open_store(&storage, project);
    assert!(loaded.tasks.is_empty());
    drop(worker);
    let connection = rusqlite::Connection::open(storage.join("workbench.sqlite3")).unwrap();
    let remaining: i64 = connection
        .query_row(
            "SELECT count(*) FROM context_revisions WHERE task_id=?1",
            [task.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        remaining, 0,
        "task deletion must explicitly remove revision rows even without FK enforcement"
    );
    crate::store::context_cache::inspect(&connection).unwrap();
    // Simulate an old writer leaving a structurally valid orphan: schema inspection
    // must reject it, not silently accept a cache owned by a nonexistent task.
    connection
        .pragma_update(None, "foreign_keys", false)
        .unwrap();
    let row = &current[0];
    connection.execute(
        "INSERT INTO context_revisions(task_id,source_key,source_sha256,policy,tokenizer,tokens,assembly) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![task.id, row.source_key, row.source_sha256, row.policy, row.tokenizer, row.tokens as i64, row.assembly],
    ).unwrap();
    assert!(
        crate::store::context_cache::inspect(&connection).is_err(),
        "orphan cache owners must prevent opening storage"
    );
}

#[test]
fn optimizer_cancellation_and_changed_policy_never_produce_a_dispatchable_packet() {
    let fixture = Fixture::new();
    let report = fixture.report(&[(
        "src/accounting.rs",
        "pub fn normalize_total() { /* subtotal underflow */ }\n",
    )]);
    let error = execute(
        7,
        Operation::Optimize {
            report: Arc::clone(&report),
            excerpts: Vec::new(),
            facts: facts(7),
            budget: 4_000,
            cache: Vec::new(),
        },
        &[],
        &AtomicBool::new(true),
    )
    .unwrap_err();
    assert!(error.contains("cancelled"));
    let plan = optimize(&report, &[], facts(7), 4_000, &[]);
    for change in ["policy", "tokenizer", "exclusion"] {
        let mut applied = plan.clone();
        let exclusions = if change == "exclusion" {
            vec![PathBuf::from("src")]
        } else {
            Vec::new()
        };
        if change == "policy" {
            applied.policy.push_str("-old");
        }
        if change == "tokenizer" {
            applied.tokenizer.push_str("-old");
        }
        assert!(
            optimizer::planning_payload(7, Some(&report), &[], Some(&applied), &exclusions)
                .is_err()
        );
    }
}

fn compact_capture(
    report: &Arc<repo::Report>,
    capture: &Excerpt,
    exclusions: &[PathBuf],
) -> Result<Response, String> {
    let mut task = crate::store::Snapshot::empty(report.repository.root.to_str().unwrap().into());
    task.id = capture.task_id;
    task.revision = 1;
    task.goal = "Retain historical source evidence".into();
    execute(
        task.id,
        Operation::Compact {
            report: Arc::clone(report),
            task: Box::new(task),
            excerpts: vec![capture.clone()],
            runs: Vec::new(),
            budget: 8_000,
        },
        exclusions,
        &AtomicBool::new(false),
    )
}

#[test]
fn restored_compaction_capture_cannot_bypass_current_exclusions_even_without_plan_guards() {
    let fixture = Fixture::new();
    let report = fixture.report(&[("src/private.rs", "pub fn historical_secret() {}\n")]);
    let capture = pin(&report, "src/private.rs", 1, 1, 41);
    // Exercise the persisted-capture boundary rather than attempting a new capture
    // under the new policy.
    let restored: Excerpt =
        serde_json::from_str(&serde_json::to_string(&capture).unwrap()).unwrap();
    let Response::Optimized(plan) = compact_capture(&report, &restored, &[]).unwrap() else {
        panic!("expected an actual compact plan");
    };
    assert!(
        optimizer::planning_payload(
            7,
            Some(&report),
            std::slice::from_ref(&restored),
            Some(&plan),
            &[]
        )
        .unwrap()
        .dispatch
    );
    // Privacy is independent of freshness: both unchanged and stale historical
    // bytes must be refused before a ledger is returned.
    for changed in [false, true] {
        if changed {
            fs::write(
                fixture.0.join("src/private.rs"),
                "pub fn revised_secret() {}\n",
            )
            .unwrap();
        }
        let exclusions = vec![PathBuf::from("src")];
        assert!(
            compact_capture(&report, &restored, &exclusions).is_err(),
            "excluded restored capture must fail compact preview (changed={changed})"
        );
        assert!(
            optimizer::planning_payload(
                7,
                Some(&report),
                std::slice::from_ref(&restored),
                Some(&plan),
                &exclusions,
            )
            .is_err(),
            "a previously valid compact plan must reject a changed exclusion policy"
        );
        // A restored/external plan can claim the current policy and omit all
        // guards. Every selected capture still needs an independent privacy check.
        let mut external = plan.clone();
        external.policy = optimizer::policy_identity(&exclusions);
        external.guards.clear();
        assert!(
            optimizer::planning_payload(
                7,
                Some(&report),
                std::slice::from_ref(&restored),
                Some(&external),
                &exclusions,
            )
            .is_err(),
            "matching policy and missing guards must not authorize excluded pins"
        );
    }
}

#[test]
fn permitted_changed_historical_capture_compacts_but_later_mutation_stops_dispatch() {
    let fixture = Fixture::new();
    let original = "pub fn historical_value() -> u32 { 1 }\n";
    let report = fixture.report(&[("src/history.rs", original)]);
    let capture = pin(&report, "src/history.rs", 1, 1, 42);
    fs::write(
        fixture.0.join("src/history.rs"),
        "pub fn historical_value() -> u32 { 2 }\n",
    )
    .unwrap();
    let Response::Optimized(plan) = compact_capture(&report, &capture, &[]).unwrap() else {
        panic!("expected an actual compact plan for permitted historical bytes");
    };
    let ledger: serde_json::Value = serde_json::from_str(&plan.packet).unwrap();
    assert_eq!(
        ledger["source_observations"][0]["observed_status"],
        "changed"
    );
    assert_eq!(ledger["captures"][0]["text"], original);
    assert!(
        optimizer::planning_payload(
            7,
            Some(&report),
            std::slice::from_ref(&capture),
            Some(&plan),
            &[]
        )
        .unwrap()
        .dispatch
    );
    fs::write(
        fixture.0.join("src/history.rs"),
        "pub fn historical_value() -> u32 { 3 }\n",
    )
    .unwrap();
    assert!(
        optimizer::planning_payload(7, Some(&report), &[capture], Some(&plan), &[]).is_err(),
        "dispatch must guard the observed current revision, not the historical digest"
    );
}

#[test]
fn native_read_deduplication_requires_exact_observed_capture_identity_and_preserves_permissions() {
    let fixture = Fixture::new();
    let source = "pub fn shared_bytes() -> u32 {\r\n    let value = 42;\r\n    value\r\n}\r\n// final exact source boundary\r\n";
    let report = fixture.report(&[("src/first.rs", source), ("src/copy.rs", source)]);
    let narrow = pin(&report, "src/first.rs", 2, 3, 22);
    let widest = pin(&report, "src/first.rs", 1, 5, 21);
    let mut tied = widest.clone();
    tied.id = 23;
    // The widest carrier wins even when the narrower exact capture is first;
    // equal-width carriers retain the ordinary attachment's earliest identity.
    let pins = vec![narrow, widest.clone(), tied];
    let (raw, observed_digest) = repo::read_source(
        &report.repository.root,
        Path::new("src/first.rs"),
        MAX_SOURCE_BYTES,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(observed_digest, widest.source_sha256);
    let observed_text = std::str::from_utf8(&raw).unwrap();
    let retained_observation = json!({
        "status": "success",
        "digest_scope": "full_file",
        "path": "src/first.rs",
        "sha256": sha256_hex(&observed_digest),
        "start_line": 2,
        "end_line": 3,
        "content": line_range(observed_text, 2, 3).unwrap(),
        "permissions": {"write": false, "execute": false, "approval_required": true},
        "observation_id": "local-read-17",
    });
    assert_eq!(
        retained_observation["content"],
        "    let value = 42;\r\n    value\r\n",
    );
    let root = fixture.0.to_str().unwrap();
    let mut outgoing = retained_observation.clone();
    assert!(optimizer::deduplicate_read_result(
        &mut outgoing,
        7,
        root,
        &pins
    ));
    assert!(outgoing.get("content").is_none());
    assert_eq!(outgoing["same_bytes_as"], widest.id);
    assert_eq!(outgoing["capture_reference"]["task_id"], 7);
    assert_eq!(outgoing["capture_reference"]["capture_id"], widest.id);
    assert_eq!(outgoing["capture_reference"]["start_line"], 2);
    assert_eq!(outgoing["capture_reference"]["end_line"], 3);
    let meaning = outgoing["capture_reference"]["meaning"].as_str().unwrap();
    assert!(meaning.contains("Fresh read still occurred"));
    assert!(meaning.contains("no provider retention"));
    assert!(meaning.contains("write permission or check result"));
    let mut expected_remaining = retained_observation.clone();
    expected_remaining
        .as_object_mut()
        .unwrap()
        .remove("content");
    let mut actual_remaining = outgoing.clone();
    actual_remaining
        .as_object_mut()
        .unwrap()
        .remove("same_bytes_as");
    actual_remaining
        .as_object_mut()
        .unwrap()
        .remove("capture_reference");
    assert_eq!(actual_remaining, expected_remaining);
    assert_eq!(
        retained_observation["content"],
        line_range(observed_text, 2, 3).unwrap()
    );
    assert!(retained_observation.get("same_bytes_as").is_none());
    assert!(outgoing.get("cache_hit").is_none());
    assert!(outgoing.get("source_observation").is_none());
    assert!(outgoing.get("approved").is_none());

    for mismatch in [
        "root",
        "task",
        "path",
        "digest",
        "range",
        "zero start",
        "reversed range",
        "range bytes",
        "content",
        "status",
        "digest scope",
        "missing digest",
    ] {
        let mut result = retained_observation.clone();
        let other_root = fixture.0.join("another-project");
        let mut owner = 7;
        let mut launch_root = root;
        match mismatch {
            "root" => launch_root = other_root.to_str().unwrap(),
            "task" => owner = 8,
            "path" => result["path"] = json!("src/copy.rs"),
            "digest" => result["sha256"] = json!("0".repeat(64)),
            "range" => result["end_line"] = json!(6),
            "zero start" => result["start_line"] = json!(0),
            "reversed range" => result["end_line"] = json!(1),
            "range bytes" => result["start_line"] = json!(1),
            "content" => result["content"] = json!("different observed bytes\n"),
            "status" => result["status"] = json!("error"),
            "digest scope" => result["digest_scope"] = json!("excerpt"),
            "missing digest" => {
                result.as_object_mut().unwrap().remove("sha256");
            }
            _ => unreachable!(),
        }
        let original = result.clone();
        assert!(
            !optimizer::deduplicate_read_result(&mut result, owner, launch_root, &pins),
            "{mismatch}"
        );
        assert_eq!(
            result, original,
            "a rejected {mismatch} must leave every payload and permission field unchanged"
        );
    }
    for invalid_capture in ["unavailable", "unsaved", "foreign task", "foreign root"] {
        let mut capture = widest.clone();
        match invalid_capture {
            "unavailable" => capture.available = false,
            "unsaved" => capture.id = 0,
            "foreign task" => capture.task_id = 8,
            "foreign root" => capture.root.push_str("/another-project"),
            _ => unreachable!(),
        }
        let mut result = retained_observation.clone();
        assert!(
            !optimizer::deduplicate_read_result(&mut result, 7, root, &[capture]),
            "{invalid_capture}"
        );
        assert_eq!(result, retained_observation);
    }
}
