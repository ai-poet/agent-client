//! Side questions answered in the background, polled for their text.
//!
//! Fork addition (`/btw`, ⌘;). A workspace operation is one request and one
//! answer, and an answer streams, so a side question runs on a thread of its
//! own: `start` hands back a job id at once, `poll` returns what the answer
//! has added since the caller's offset (waiting briefly for more), and
//! `cancel` stops it. Nothing is written to disk; a finished job nobody polls
//! is forgotten after a while, and only a few run at once.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::bail;
use uuid::Uuid;
use waku_agent_bridge::side_chat::{CancellationToken, SideMessage, ask_streaming};
use waku_protocol::SideChatMessage;

/// How many side questions may be answered at once.
const MAX_JOBS: usize = 8;
/// A finished job unpolled this long is dropped.
const FINISHED_TTL: Duration = Duration::from_secs(120);
/// The longest a poll waits for more text.
const MAX_WAIT: Duration = Duration::from_millis(250);

#[derive(Default)]
struct Job {
    text: String,
    done: bool,
    error: Option<String>,
    cancel: CancellationToken,
    last_poll: Option<Instant>,
}

#[derive(Default)]
struct Jobs {
    jobs: Mutex<HashMap<Uuid, Job>>,
    changed: Condvar,
}

fn jobs() -> &'static Arc<Jobs> {
    static JOBS: OnceLock<Arc<Jobs>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

/// What a poll reports.
#[derive(Debug, PartialEq, Eq)]
pub struct Progress {
    pub text: String,
    pub next: usize,
    pub done: bool,
    pub error: Option<String>,
}

/// Start answering `messages` (the last one the question) on a thread of
/// its own.
pub fn start(
    cwd: PathBuf,
    model: Option<String>,
    system: String,
    messages: Vec<SideChatMessage>,
) -> anyhow::Result<Uuid> {
    let registry = jobs().clone();
    let id = Uuid::new_v4();
    let cancel = CancellationToken::new();
    {
        let mut jobs = registry.jobs.lock().expect("side chat jobs");
        forget_stale(&mut jobs, Instant::now());
        if jobs.values().filter(|job| !job.done).count() >= MAX_JOBS {
            bail!("too many side questions are being answered at once");
        }
        jobs.insert(
            id,
            Job {
                cancel: cancel.clone(),
                ..Job::default()
            },
        );
    }
    let thread: Vec<SideMessage> = messages
        .into_iter()
        .map(|message| SideMessage {
            assistant: message.assistant,
            text: message.text,
        })
        .collect();
    let spawned = std::thread::Builder::new()
        .name(format!("waku-side-chat-{id}"))
        .spawn({
            let registry = registry.clone();
            move || {
                let mut on_text = |delta: &str| {
                    let mut jobs = registry.jobs.lock().expect("side chat jobs");
                    if let Some(job) = jobs.get_mut(&id) {
                        job.text.push_str(delta);
                    }
                    registry.changed.notify_all();
                };
                let result = ask_streaming(&cwd, model.as_deref(), &system, &thread, &mut on_text, cancel);
                let mut jobs = registry.jobs.lock().expect("side chat jobs");
                if let Some(job) = jobs.get_mut(&id) {
                    match result {
                        // A stream that delivered no deltas (a provider that
                        // answers whole) still has its text.
                        Ok(answer) if job.text.trim().is_empty() => job.text = answer,
                        Ok(_) => {}
                        Err(error) => job.error = Some(format!("{error:#}")),
                    }
                    job.done = true;
                }
                registry.changed.notify_all();
            }
        });
    if let Err(error) = spawned {
        registry.jobs.lock().expect("side chat jobs").remove(&id);
        bail!("could not start the side question: {error}");
    }
    Ok(id)
}

/// The answer's text from byte `from` on, waiting up to `wait_ms` for more
/// when there is none yet.
pub fn poll(job: Uuid, from: usize, wait_ms: u64) -> anyhow::Result<Progress> {
    let registry = jobs();
    let deadline = Instant::now() + Duration::from_millis(wait_ms).min(MAX_WAIT);
    let mut jobs = registry.jobs.lock().expect("side chat jobs");
    loop {
        let Some(entry) = jobs.get_mut(&job) else {
            bail!("the side question is gone");
        };
        entry.last_poll = Some(Instant::now());
        if entry.done || entry.text.len() > from {
            return Ok(progress_of(entry, from));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(progress_of(entry, from));
        }
        jobs = registry
            .changed
            .wait_timeout(jobs, deadline - now)
            .expect("side chat jobs")
            .0;
    }
}

/// Stop answering. The job stays until it is polled or expires, so a last
/// poll reads that it ended.
pub fn cancel(job: Uuid) {
    let registry = jobs();
    if let Some(entry) = registry.jobs.lock().expect("side chat jobs").get(&job) {
        entry.cancel.cancel();
    }
    registry.changed.notify_all();
}

fn progress_of(job: &mut Job, from: usize) -> Progress {
    let from = floor_char_boundary(&job.text, from);
    let progress = Progress {
        text: job.text[from..].to_owned(),
        next: job.text.len(),
        done: job.done,
        error: job.error.clone(),
    };
    progress
}

fn floor_char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn forget_stale(jobs: &mut HashMap<Uuid, Job>, now: Instant) {
    jobs.retain(|_, job| {
        !job.done
            || job
                .last_poll
                .is_none_or(|polled| now.duration_since(polled) < FINISHED_TTL)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finished(text: &str, polled_ago: Option<Duration>, now: Instant) -> Job {
        Job {
            text: text.to_owned(),
            done: true,
            last_poll: polled_ago.map(|ago| now - ago),
            ..Job::default()
        }
    }

    #[test]
    fn a_poll_returns_what_was_added_since_its_offset() {
        let mut job = Job {
            text: "hello wörld".to_owned(),
            ..Job::default()
        };
        let progress = progress_of(&mut job, 6);
        assert_eq!(progress.text, "wörld");
        assert_eq!(progress.next, job.text.len());
        assert!(!progress.done);
        // An offset inside a character backs up to its start.
        let progress = progress_of(&mut job, 8);
        assert_eq!(progress.text, "örld");
    }

    #[test]
    fn finished_jobs_nobody_polls_are_forgotten() {
        let now = Instant::now() + Duration::from_secs(1_000);
        let mut jobs = HashMap::from([
            (Uuid::from_u128(1), finished("a", Some(Duration::from_secs(300)), now)),
            (Uuid::from_u128(2), finished("b", Some(Duration::from_secs(10)), now)),
            (Uuid::from_u128(3), Job::default()),
        ]);
        forget_stale(&mut jobs, now);
        assert!(!jobs.contains_key(&Uuid::from_u128(1)));
        assert!(jobs.contains_key(&Uuid::from_u128(2)));
        assert!(jobs.contains_key(&Uuid::from_u128(3)));
    }

    #[test]
    fn an_unknown_job_is_an_error() {
        assert!(poll(Uuid::new_v4(), 0, 0).is_err());
    }
}
