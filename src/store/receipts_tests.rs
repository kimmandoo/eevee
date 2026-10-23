use super::*;
use crate::providers::{Agent, Operation as ProviderOperation};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "ev-outgoing-receipts-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn open(&self, project: &str) -> State {
        let mut state = State {
            data_dir: self.0.clone(),
            project: project.into(),
            policy: Policy::default(),
            lock: None,
            connection: None,
        };
        assert!(matches!(state.reopen(), Event::Loaded(_)));
        state
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn new_task(state: &mut State) -> Snapshot {
    let mut task = Snapshot::empty(state.project.clone());
    task.goal = "Receipt source ownership".into();
    task.revision = 1;
    state.save(task).unwrap()
}
fn outgoing(state: &mut State, task: &Snapshot, id: &str, payload: &str) -> Outgoing {
    let owner = Attribution::of(&partners(state.connection.as_ref().unwrap()).unwrap()[0]);
    let claim = WorkClaim {
        task_id: task.id,
        owner,
        request_id: id.into(),
        writing: false,
    };
    state
        .work_request(&WorkRequest::Acquire {
            claim: claim.clone(),
            task: Box::new(task.clone()),
        })
        .unwrap();
    Outgoing::frozen(
        payload,
        &claim,
        11,
        Agent::Codex,
        ProviderOperation::Plan,
        1234,
    )
    .unwrap()
}
fn request(outgoing: &Outgoing, operation: Operation) -> Request {
    Request {
        task_id: outgoing.record.task_id,
        partner: outgoing.record.owner.partner,
        operation,
    }
}

#[test]
fn restart_inspection_purge_counts_and_nonresurrection_are_one_owned_identity() {
    let fixture = Fixture::new();
    let mut state = fixture.open("receipt-project");
    let task = new_task(&mut state);
    let payload = "{\n  \"purpose\": \"Read only\",\n  \"selected_sources\": {\"captures\":[{\"id\":7,\"text\":\"東京 exact source\"}]}\n}";
    let outgoing = outgoing(&mut state, &task, "receipt-actual-request", payload);
    let put = request(&outgoing, Operation::Put(Box::new(outgoing.clone())));
    assert!(matches!(
        state.receipts_request(&put).unwrap(),
        Response::Saved(view) if view.source_available
    ));
    state
        .work_request(&WorkRequest::Release(outgoing.record.claim()))
        .unwrap();
    drop(state);
    let mut state = fixture.open("receipt-project");
    let list = state
        .receipts_request(&request(&outgoing, Operation::List))
        .unwrap();
    assert_eq!(
        list,
        Response::Records(vec![View {
            record: outgoing.record.clone(),
            source_available: true
        }])
    );
    assert_eq!(
        state
            .receipts_request(&request(
                &outgoing,
                Operation::ReadSource {
                    request_id: outgoing.record.request_id.clone()
                }
            ))
            .unwrap(),
        Response::Source {
            request_id: outgoing.record.request_id.clone(),
            text: payload.into()
        }
    );
    let mut wrong = request(
        &outgoing,
        Operation::ReadSource {
            request_id: outgoing.record.request_id.clone(),
        },
    );
    wrong.partner += 1;
    assert!(
        state
            .receipts_request(&wrong)
            .unwrap_err()
            .contains("Wrong-owner")
    );
    wrong.operation = Operation::PurgeSources {
        request_id: Some(outgoing.record.request_id.clone()),
    };
    assert!(
        state
            .receipts_request(&wrong)
            .unwrap_err()
            .contains("Wrong-owner")
    );
    let other = new_task(&mut state);
    wrong.task_id = other.id;
    wrong.partner = outgoing.record.owner.partner;
    assert!(
        state
            .receipts_request(&wrong)
            .unwrap_err()
            .contains("Wrong-owner")
    );
    state
        .receipts_request(&request(
            &outgoing,
            Operation::PurgeSources {
                request_id: Some(outgoing.record.request_id.clone()),
            },
        ))
        .unwrap();
    // A delayed acknowledgement/retry after purge cannot put the bytes back.
    assert_eq!(
        state.receipts_request(&put).unwrap(),
        Response::Saved(Box::new(View {
            record: outgoing.record.clone(),
            source_available: false
        }))
    );
    assert!(
        state
            .receipts_request(&request(
                &outgoing,
                Operation::ReadSource {
                    request_id: outgoing.record.request_id.clone()
                }
            ))
            .unwrap_err()
            .contains("unavailable")
    );
    drop(state);
    let mut state = fixture.open("receipt-project");
    assert_eq!(
        state
            .receipts_request(&request(&outgoing, Operation::List))
            .unwrap(),
        Response::Records(vec![View {
            record: outgoing.record.clone(),
            source_available: false
        }])
    );
    assert_eq!(outgoing.record.bytes, payload.len());
    assert_eq!(outgoing.record.captures[0].id, 7);
    drop(state);
    let mut foreign = fixture.open("foreign-project");
    assert!(
        foreign
            .receipts_request(&request(&outgoing, Operation::List))
            .is_err()
    );
}

#[test]
fn active_purge_is_refused_and_source_copy_purge_retains_exact_sections() {
    let fixture = Fixture::new();
    let mut state = fixture.open("source-purge-project");
    let task = new_task(&mut state);
    let outgoing = outgoing(
        &mut state,
        &task,
        "receipt-source-copy",
        r#"{"purpose":"Plan","task":{"goal":"public"},"approved_memories":[{"id":9,"text":"public memory"}]}"#,
    );
    state
        .receipts_request(&request(
            &outgoing,
            Operation::Put(Box::new(outgoing.clone())),
        ))
        .unwrap();
    assert!(
        state
            .receipts_request(&request(
                &outgoing,
                Operation::PurgeSources { request_id: None }
            ))
            .is_err()
    );
    state
        .work_request(&WorkRequest::Release(outgoing.record.claim()))
        .unwrap();
    purge_sources(state.connection.as_ref().unwrap(), task.id, Some(&[7])).unwrap();
    assert_eq!(
        state
            .receipts_request(&request(&outgoing, Operation::List))
            .unwrap(),
        Response::Records(vec![View {
            record: outgoing.record.clone(),
            source_available: false
        }])
    );
    assert_eq!(outgoing.record.references[0].kind, "memory");
    assert_eq!(outgoing.record.references[0].id, "9");
}

#[test]
fn recognizable_credentials_are_counted_but_never_persisted_in_source() {
    let fixture = Fixture::new();
    let mut state = fixture.open("private-source-project");
    let task = new_task(&mut state);
    let outgoing = outgoing(
        &mut state,
        &task,
        "receipt-private-counts",
        r#"{"request":"password=never-store-this-receipt-secret"}"#,
    );
    assert!(outgoing.source.is_none());
    state
        .receipts_request(&request(
            &outgoing,
            Operation::Put(Box::new(outgoing.clone())),
        ))
        .unwrap();
    let saved: String = state
        .connection
        .as_ref()
        .unwrap()
        .query_row(
            "SELECT record FROM outgoing_receipts WHERE request_id=?1",
            [&outgoing.record.request_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!saved.contains("never-store-this-receipt-secret"));
    assert!(outgoing.record.bytes > 0);
}

#[test]
fn final_receipt_slot_is_reserved_transactionally_before_provider_admission() {
    let fixture = Fixture::new();
    let mut state = fixture.open("receipt-capacity-project");
    let mut task = Snapshot::empty(state.project.clone());
    task.goal = "Capacity cannot submit an unrecordable request".into();
    task.revision = 1;
    task.partner_drafts.insert(
        1,
        PartnerDrafts {
            drafts: vec![Draft {
                field: EditorField::Request,
                text: "Keep this exact unsent draft 東京".into(),
                cursor: 5,
            }],
            ..PartnerDrafts::default()
        },
    );
    let task = state.save(task).unwrap();
    let roster = state.add_partner().unwrap();
    let first = Attribution::of(&roster[0]);
    let second = Attribution::of(&roster[1]);
    let seed_claim = WorkClaim {
        task_id: task.id,
        owner: first.clone(),
        request_id: "seed".into(),
        writing: false,
    };
    let seed = Outgoing::frozen(
        r#"{"request":"historical public request"}"#,
        &seed_claim,
        11,
        Agent::Codex,
        ProviderOperation::Plan,
        1234,
    )
    .unwrap();
    let transaction = state.connection.as_mut().unwrap().transaction().unwrap();
    for index in 0..MAX_RECORDS - 1 {
        let mut record = seed.record.clone();
        record.request_id = format!("seed-{index}");
        transaction.execute(
            "INSERT INTO outgoing_receipts(request_id,task_id,partner,record,source) VALUES(?1,?2,?3,?4,NULL)",
            params![record.request_id, task.id, first.partner, serde_json::to_string(&record).unwrap()],
        ).unwrap();
    }
    transaction.commit().unwrap();
    let claim = |owner: Attribution, request_id: &str| WorkClaim {
        task_id: task.id,
        owner,
        request_id: request_id.into(),
        writing: false,
    };
    let last = claim(first.clone(), "reserved-last-slot");
    state
        .work_request(&WorkRequest::Acquire {
            claim: last.clone(),
            task: Box::new(task.clone()),
        })
        .unwrap();
    let competing = claim(second, "concurrent-final-slot");
    let refused = state
        .work_request(&WorkRequest::Acquire {
            claim: competing.clone(),
            task: Box::new(task.clone()),
        })
        .unwrap_err();
    assert!(refused.contains("Outgoing receipt capacity reached"));
    assert_eq!(
        super::super::work::claims(state.connection.as_ref().unwrap()).unwrap(),
        vec![last.clone()]
    );
    // Cancellation before dispatch releases the reservation without a fake receipt.
    state.work_request(&WorkRequest::Release(last)).unwrap();
    state
        .work_request(&WorkRequest::Acquire {
            claim: competing.clone(),
            task: Box::new(task.clone()),
        })
        .unwrap();
    state
        .work_request(&WorkRequest::Release(competing))
        .unwrap();
    let committed = claim(first, "actually-submitted-final-slot");
    state
        .work_request(&WorkRequest::Acquire {
            claim: committed.clone(),
            task: Box::new(task.clone()),
        })
        .unwrap();
    let outgoing = Outgoing::frozen(
        r#"{"request":"actual final request"}"#,
        &committed,
        12,
        Agent::Codex,
        ProviderOperation::Plan,
        5678,
    )
    .unwrap();
    state
        .receipts_request(&request(
            &outgoing,
            Operation::Put(Box::new(outgoing.clone())),
        ))
        .unwrap();
    // Exact acquire remains idempotent when its reservation has become a receipt.
    state
        .work_request(&WorkRequest::Acquire {
            claim: committed.clone(),
            task: Box::new(task.clone()),
        })
        .unwrap();
    state
        .work_request(&WorkRequest::Release(committed.clone()))
        .unwrap();
    let full = claim(committed.owner, "full-capacity-must-not-send");
    assert!(
        state
            .work_request(&WorkRequest::Acquire {
                claim: full,
                task: Box::new(task.clone())
            })
            .unwrap_err()
            .contains("Outgoing receipt capacity reached")
    );
    assert!(
        super::super::work::claims(state.connection.as_ref().unwrap())
            .unwrap()
            .is_empty()
    );
    let restored = load_task(state.connection.as_ref().unwrap(), &state.project, task.id)
        .unwrap()
        .unwrap();
    assert_eq!(restored.partner_drafts, task.partner_drafts);
    let count: usize = state
        .connection
        .as_ref()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM outgoing_receipts WHERE task_id=?1",
            [task.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, MAX_RECORDS);
}

#[test]
fn quota_rejection_rolls_back_entire_receipt_and_storage_only_retry_uses_same_identity() {
    let fixture = Fixture::new();
    let mut state = fixture.open("receipt-quota-project");
    let task = new_task(&mut state);
    let outgoing = outgoing(
        &mut state,
        &task,
        "actual-quota-request",
        r#"{"request":"public exact source"}"#,
    );
    let put = request(&outgoing, Operation::Put(Box::new(outgoing.clone())));
    state.policy.quota_bytes = 0;
    let error = state.receipts_request(&put).unwrap_err();
    assert!(error.starts_with(
        "Local storage quota exceeded (live SQLite + retained output + committed reservations);"
    ));
    let count: usize = state
        .connection
        .as_ref()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM outgoing_receipts WHERE request_id=?1",
            [&outgoing.record.request_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "rollback proof must cover metadata and source together"
    );
    assert_eq!(
        super::super::work::claims(state.connection.as_ref().unwrap()).unwrap(),
        vec![outgoing.record.claim()]
    );
    state.policy = Policy::default();
    let Response::Saved(view) = state.receipts_request(&put).unwrap() else {
        panic!("expected saved receipt");
    };
    assert_eq!(view.record, outgoing.record);
    assert!(view.source_available);
}
