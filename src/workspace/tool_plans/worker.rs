//! Scheduling adapter for the existing bounded file executor; never runs commands.
use super::{NodeSpec, Operation};
use crate::{
    providers::writes::{FileTool, WriteSet},
    workspace::{repo, runner},
};
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Instant,
};

pub struct Outcome {
    pub index: usize,
    pub started: bool,
    pub query_worker_started: bool,
    pub result: Result<(String, String), String>,
    pub elapsed_us: u64,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
}
pub enum Event {
    Ready(Result<runner::InputSnapshot, String>),
    Observed(Vec<Outcome>),
}
enum Job {
    Observe(Vec<(usize, NodeSpec)>),
}
pub struct Worker {
    sender: Option<SyncSender<Job>>,
    events: Receiver<Event>,
    cancelled: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Worker {
    pub fn start(
        root: PathBuf,
        data_dir: PathBuf,
        exclusions: Vec<PathBuf>,
        expected: runner::InputSnapshot,
    ) -> io::Result<Self> {
        let (sender, incoming) = mpsc::sync_channel(1);
        let (outgoing, events) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&cancelled);
        let join = thread::Builder::new()
            .name("tool-plan-observations".into())
            .spawn(move || {
                let initial = (|| {
                    let current = repo::observe_inputs(&root, &data_dir)?;
                    if expected.fingerprint.is_some() && current != expected {
                        return Err(
                            "Plan source identity changed; stage a fresh plan before starting"
                                .into(),
                        );
                    }
                    let set = WriteSet::capture_readonly(&root, &data_dir, &exclusions)?;
                    Ok((current, set))
                })();
                let (current, mut set) = match initial {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = outgoing.send(Event::Ready(Err(error)));
                        return;
                    }
                };
                if outgoing.send(Event::Ready(Ok(current))).is_err() {
                    return;
                }
                while let Ok(Job::Observe(nodes)) = incoming.recv() {
                    let results = thread::scope(|scope| {
                        let mut results = Vec::with_capacity(nodes.len());
                        let mut handles = Vec::with_capacity(nodes.len());
                        for (index, node) in nodes {
                            let clock = Instant::now();
                            let started_at_ms = super::now_ms();
                            if stopped.load(Ordering::Acquire) {
                                results.push(complete(
                                    index,
                                    false,
                                    false,
                                    clock,
                                    started_at_ms,
                                    Err("Cancelled before observation admission".into()),
                                ));
                                continue;
                            }
                            let tool = match node.operation {
                                Operation::Read {
                                    path,
                                    start_line,
                                    end_line,
                                } => FileTool::Read {
                                    path: path.into(),
                                    start_line,
                                    end_line,
                                },
                                Operation::List {
                                    directory,
                                    query,
                                    offset,
                                } => FileTool::List {
                                    directory: directory.into(),
                                    query,
                                    offset,
                                },
                                Operation::Command { .. } => {
                                    results.push(complete(
                                            index,
                                            false,
                                            false,
                                            clock,
                                            started_at_ms,
                                            Err("Commands must use the existing reviewed runner"
                                                .into()),
                                        ));
                                    continue;
                                }
                            };
                            let deadline = clock + std::time::Duration::from_secs(30);
                            let cancelled = || stopped.load(Ordering::Acquire);
                            let control = crate::providers::writes::ObservationControl {
                                stopped: stopped.as_ref(),
                                cancelled: &cancelled,
                                deadline,
                            };
                            // A known read fingerprints/resolves on this already-running
                            // coordinator. A valid cache hit starts no query worker;
                            // stale known keys compute one honest miss without rehashing.
                            if let Some(result) = set.observe_known_tool_controlled(&tool, &control)
                            {
                                results.push(complete(
                                    index,
                                    true,
                                    false,
                                    clock,
                                    started_at_ms,
                                    result,
                                ));
                                continue;
                            }
                            // A cold argument key has performed no filesystem work.
                            // Clone existing read authority only for the actual query;
                            // independent cold calls can overlap, bounded to three.
                            let mut reader = set.clone();
                            let stopped = &stopped;
                            let handle = scope.spawn(move || {
                                let started = !stopped.load(Ordering::Acquire);
                                let result = if started {
                                    let cancelled = || stopped.load(Ordering::Acquire);
                                    let control = crate::providers::writes::ObservationControl {
                                        stopped: stopped.as_ref(),
                                        cancelled: &cancelled,
                                        deadline,
                                    };
                                    reader.observe_tool_controlled(&tool, &control)
                                } else {
                                    Err("Cancelled before observation admission".into())
                                };
                                complete(index, started, true, clock, started_at_ms, result)
                            });
                            handles.push((index, clock, started_at_ms, handle));
                        }
                        for (index, clock, started_at_ms, handle) in handles {
                            results.push(handle.join().unwrap_or_else(|_| {
                                complete(
                                    index,
                                    true,
                                    true,
                                    clock,
                                    started_at_ms,
                                    Err("Observation worker panicked; no command ran".into()),
                                )
                            }));
                        }
                        results
                    });
                    if outgoing.send(Event::Observed(results)).is_err() {
                        return;
                    }
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            events,
            cancelled,
            join: Some(join),
        })
    }
    pub fn observe(&self, nodes: Vec<(usize, NodeSpec)>) -> bool {
        self.sender
            .as_ref()
            .is_some_and(|sender| sender.try_send(Job::Observe(nodes)).is_ok())
    }
    pub fn event(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
fn complete(
    index: usize,
    started: bool,
    query_worker_started: bool,
    clock: Instant,
    started_at_ms: u64,
    result: Result<serde_json::Value, String>,
) -> Outcome {
    let result = result.map(|mut value| {
        let cache = value
            .as_object_mut()
            .and_then(|object| object.remove("observation_cache"));
        let identity = super::digest(value.to_string().as_bytes());
        if let Some(cache) = cache
            && let Some(object) = value.as_object_mut()
        {
            object.insert("observation_cache".into(), cache);
        }
        (value.to_string(), identity)
    });
    Outcome {
        index,
        started,
        query_worker_started,
        result,
        elapsed_us: clock.elapsed().as_micros().min(u64::MAX as u128) as u64,
        started_at_ms,
        ended_at_ms: super::now_ms(),
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        self.sender.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
