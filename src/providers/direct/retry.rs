//! Bounded automatic retry for native inference failures that prove the
//! provider did not process the request. Tool effects are never repeated:
//! only the current round's frozen body is resent on the same transport.
use super::*;
use std::sync::atomic::AtomicU8;

/// Extra attempts after the first one, per model round.
pub(super) const MAX_RETRIES: usize = 3;
/// A longer `Retry-After` ends the run instead of holding it.
pub(super) const MAX_WAIT: Duration = Duration::from_secs(60);
const BACKOFF: [Duration; MAX_RETRIES] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Policy {
    Off,
    #[default]
    Safe,
}

static POLICY: AtomicU8 = AtomicU8::new(1);

/// Launch-scoped choice (`--retry off|safe`), frozen before any request.
pub fn set_policy(policy: Policy) {
    POLICY.store(matches!(policy, Policy::Safe) as u8, Ordering::Release);
}
pub(super) fn policy() -> Policy {
    if POLICY.load(Ordering::Acquire) == 0 {
        Policy::Off
    } else {
        Policy::Safe
    }
}

/// HTTP statuses whose documented meaning is "not processed, try later".
pub(super) fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 503 | 529)
}

/// Integer-seconds `Retry-After` only; an HTTP date falls back to backoff.
pub(super) fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// Why one attempt was retried; listed in the run's completion message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Attempt {
    pub round: usize,
    pub cause: String,
    pub wait: Duration,
}

pub(super) fn summary(attempts: &[Attempt]) -> String {
    if attempts.is_empty() {
        return String::new();
    }
    let causes = attempts
        .iter()
        .map(|attempt| {
            format!(
                "round {} {} (waited {} s)",
                attempt.round,
                attempt.cause,
                attempt.wait.as_secs()
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        " Automatic retries: {} [{causes}]; rejected attempts were not processed by the provider according to its status, which Eevee cannot prove.",
        attempts.len()
    )
}

pub(super) enum Decision {
    Retry(Duration),
    /// Not eligible; carries the reason appended to the final failure.
    Stop(&'static str),
}

/// Chooses the wait before attempt `done + 1`. `requested` is a server
/// `Retry-After`; `deadline` bounds the whole round.
pub(super) fn decide(
    policy: Policy,
    done: usize,
    requested: Option<Duration>,
    deadline: Instant,
    jitter: Duration,
) -> Decision {
    if policy == Policy::Off {
        return Decision::Stop("automatic retry is off (--retry off)");
    }
    if done >= MAX_RETRIES {
        return Decision::Stop("automatic retries exhausted");
    }
    let wait = match requested {
        Some(wait) if wait > MAX_WAIT => {
            return Decision::Stop("provider asked to wait longer than 60 s; not retried");
        }
        Some(wait) => wait,
        None => BACKOFF[done] + jitter,
    };
    if Instant::now() + wait >= deadline {
        return Decision::Stop("retry wait would exceed the round deadline");
    }
    Decision::Retry(wait)
}

pub(super) fn jitter() -> Duration {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |time| time.subsec_nanos());
    Duration::from_millis(u64::from(nanos % 250))
}

/// Waits without consuming cancellation: Esc during backoff stops the run.
pub(super) async fn backoff(wait: Duration, control: &Control<'_>) -> Result<(), String> {
    let until = Instant::now() + wait;
    while Instant::now() < until {
        if control.cancelled() {
            return Err(
                "Native call cancelled locally during retry backoff; no request was in flight"
                    .into(),
            );
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn far() -> Instant {
        Instant::now() + Duration::from_secs(600)
    }

    #[test]
    fn only_documented_not_processed_statuses_are_retryable() {
        for status in [429, 503, 529] {
            assert!(retryable_status(status), "{status}");
        }
        for status in [200, 400, 401, 403, 404, 408, 422, 500, 502, 504] {
            assert!(!retryable_status(status), "{status}");
        }
    }

    #[test]
    fn decisions_are_bounded_by_policy_count_retry_after_and_deadline() {
        let zero = Duration::ZERO;
        assert!(matches!(
            decide(Policy::Off, 0, None, far(), zero),
            Decision::Stop(reason) if reason.contains("off")
        ));
        for (done, expected) in BACKOFF.iter().enumerate() {
            assert!(matches!(
                decide(Policy::Safe, done, None, far(), zero),
                Decision::Retry(wait) if wait == *expected
            ));
        }
        assert!(matches!(
            decide(Policy::Safe, MAX_RETRIES, None, far(), zero),
            Decision::Stop(reason) if reason.contains("exhausted")
        ));
        assert!(matches!(
            decide(Policy::Safe, 0, Some(Duration::from_secs(7)), far(), zero),
            Decision::Retry(wait) if wait == Duration::from_secs(7)
        ));
        assert!(matches!(
            decide(Policy::Safe, 0, Some(Duration::from_secs(120)), far(), zero),
            Decision::Stop(reason) if reason.contains("60 s")
        ));
        assert!(matches!(
            decide(Policy::Safe, 0, None, Instant::now() + Duration::from_millis(500), zero),
            Decision::Stop(reason) if reason.contains("deadline")
        ));
    }

    #[test]
    fn retry_after_accepts_integer_seconds_only() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert_eq!(retry_after(&headers), None);
        headers.insert(reqwest::header::RETRY_AFTER, "3".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(Duration::from_secs(3)));
        headers.insert(
            reqwest::header::RETRY_AFTER,
            "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&headers), None);
    }

    #[test]
    fn summary_names_every_retry_and_disclaims_proof() {
        assert_eq!(summary(&[]), "");
        let text = summary(&[Attempt {
            round: 2,
            cause: "HTTP 429".into(),
            wait: Duration::from_secs(1),
        }]);
        assert!(text.contains("Automatic retries: 1"));
        assert!(text.contains("round 2 HTTP 429 (waited 1 s)"));
        assert!(text.contains("cannot prove"));
    }
}
