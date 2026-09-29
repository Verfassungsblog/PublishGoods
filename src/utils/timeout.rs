use std::future::Future;
use std::time::Duration;

/// Converts a timeout setting in seconds into a limit. 0 disables the timeout.
pub fn limit_from_seconds(seconds: u64) -> Option<Duration> {
    (seconds > 0).then(|| Duration::from_secs(seconds))
}

/// Runs `future`, failing with [`tokio::time::error::Elapsed`] if it takes longer than `limit`.
/// Without a limit the future runs to completion.
pub async fn with_timeout<T>(
    limit: Option<Duration>,
    future: impl Future<Output = T>,
) -> Result<T, tokio::time::error::Elapsed> {
    match limit {
        Some(limit) => tokio::time::timeout(limit, future).await,
        None => Ok(future.await),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_seconds_means_no_limit() {
        assert_eq!(limit_from_seconds(0), None);
        assert_eq!(limit_from_seconds(5), Some(Duration::from_secs(5)));
    }

    #[tokio::test]
    async fn with_timeout_aborts_slow_futures_only() {
        let slow = with_timeout(
            Some(Duration::from_millis(20)),
            tokio::time::sleep(Duration::from_secs(30)),
        )
        .await;
        assert!(slow.is_err());
        let fast = with_timeout(Some(Duration::from_secs(5)), async { 7 }).await;
        assert_eq!(fast.unwrap(), 7);
        let unlimited = with_timeout(None, async { 8 }).await;
        assert_eq!(unlimited.unwrap(), 8);
    }
}
