//! A submitted unmanaged call may have effects even when its response is lost.
//! Persist before dispatch; a crash/restart must never turn missing evidence into permission.
use super::*;
use std::sync::atomic::AtomicBool;

// One metadata writer, including concurrent native partners sharing this project.
static WRITER: Mutex<()> = Mutex::new(());

struct Counts {
    active: usize,
    uncertain: bool,
}

pub struct Effects {
    data: PathBuf,
    project: String,
    unknown: AtomicBool,
    counts: Mutex<Counts>,
}

impl Effects {
    pub fn load(data: &Path, project: &str) -> Result<Arc<Self>, String> {
        let _writer = WRITER.lock();
        let document = Document::load(data)?;
        let uncertain = document.uncertain_projects.contains(project);
        Ok(Arc::new(Self {
            data: data.to_owned(),
            project: project.to_owned(),
            unknown: AtomicBool::new(uncertain),
            counts: Mutex::new(Counts {
                active: 0,
                uncertain,
            }),
        }))
    }

    pub fn unknown(&self) -> bool {
        self.unknown.load(Ordering::Acquire)
    }

    fn persist(&self, unknown: bool) -> Result<(), String> {
        let _writer = WRITER.lock();
        let mut document = Document::load(&self.data)?;
        if unknown {
            document.uncertain_projects.insert(self.project.clone());
        } else {
            document.uncertain_projects.remove(&self.project);
        }
        forward::set_critical(
            &mut document.critical,
            "uncertain_projects",
            !document.uncertain_projects.is_empty(),
        );
        document.save(&self.data)
    }

    pub fn begin(self: &Arc<Self>, automatic: bool) -> Result<Lease, String> {
        let mut counts = self.counts.lock();
        if automatic && (counts.uncertain || counts.active != 0) {
            return Err("MCP automatic rule grant invalidated by unknown/pending unmanaged effects; no call dispatched. Fresh explicit review is required.".into());
        }
        counts.active += 1;
        self.unknown.store(true, Ordering::Release);
        if let Err(error) = self.persist(true) {
            counts.active -= 1;
            self.unknown
                .store(counts.uncertain || counts.active != 0, Ordering::Release);
            return Err(format!(
                "MCP uncertainty marker could not be persisted before tools/call; no call dispatched: {error}"
            ));
        }
        Ok(Lease {
            effects: self.clone(),
            finished: false,
        })
    }

    fn settle(&self, success: bool) -> Result<bool, String> {
        let mut counts = self.counts.lock();
        counts.active = counts.active.saturating_sub(1);
        counts.uncertain |= !success;
        let unknown = counts.uncertain || counts.active != 0;
        self.unknown.store(unknown, Ordering::Release);
        if let Err(error) = self.persist(unknown) {
            // The pre-call durable marker is already true. Retain that evidence
            // floor in memory too, including if this was an observed success.
            counts.uncertain = true;
            self.unknown.store(true, Ordering::Release);
            return Err(format!(
                "MCP response observed but uncertainty marker settlement failed; automatic effect grants remain disabled: {error}"
            ));
        }
        Ok(unknown)
    }
}

pub struct Lease {
    effects: Arc<Effects>,
    finished: bool,
}
impl Lease {
    pub fn finish(mut self, success: bool) -> Result<bool, String> {
        self.finished = true;
        self.effects.settle(success)
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.effects.settle(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ev-mcp-effects-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn unknown_survives_restart_and_later_success_cannot_clear_it() {
        let root = root();
        let effects = Effects::load(&root, "project").unwrap();
        assert!(!effects.unknown());
        let lease = effects.begin(false).unwrap();
        assert!(effects.unknown());
        assert!(
            Document::load(&root)
                .unwrap()
                .uncertain_projects
                .contains("project")
        );
        assert!(lease.finish(false).unwrap());
        let restarted = Effects::load(&root, "project").unwrap();
        assert!(restarted.unknown());
        assert!(restarted.begin(false).unwrap().finish(true).unwrap());
        assert!(
            restarted.begin(true).is_err(),
            "later success must never restore automatic grants"
        );
        assert!(
            Document::load(&root)
                .unwrap()
                .critical
                .contains(&"uncertain_projects".into())
        );
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn overlapping_calls_keep_marker_until_all_observed_successes() {
        let root = root();
        let effects = Effects::load(&root, "project").unwrap();
        let first = effects.begin(true).unwrap();
        assert!(
            effects.begin(true).is_err(),
            "an overlapping automatic grant must recheck the effect boundary"
        );
        let second = effects.begin(false).unwrap();
        assert!(first.finish(true).unwrap());
        assert!(!second.finish(true).unwrap());
        assert!(!effects.unknown());
        assert!(
            !Document::load(&root)
                .unwrap()
                .uncertain_projects
                .contains("project")
        );
        let interrupted = effects.begin(false).unwrap();
        drop(interrupted);
        assert!(effects.unknown());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn failed_marker_persistence_refuses_before_dispatch() {
        let root = root();
        let effects = Effects::load(&root, "project").unwrap();
        std::fs::create_dir(root.join("mcp.json")).unwrap();
        assert!(
            effects
                .begin(false)
                .err()
                .unwrap()
                .contains("no call dispatched")
        );
        assert!(!effects.unknown());
        let _ = std::fs::remove_dir_all(root);
    }
}
