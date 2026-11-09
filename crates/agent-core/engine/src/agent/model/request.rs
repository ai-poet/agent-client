//! Sailry's request-scoped recovery policies without Node observation.
use super::{Error, Model, ModelStream, Request, retry};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
pub enum Purpose {
    Main,
    Compaction,
}

pub fn wrap(
    inner: Arc<dyn Model>,
    purpose: Purpose,
    _cancel: CancellationToken,
    stop: CancellationToken,
    on_retry: Option<Arc<dyn Fn(retry::RetryNotice) + Send + Sync>>,
) -> Arc<dyn Model> {
    Arc::new(Recovered {
        inner,
        purpose,
        stop,
        on_retry,
    })
}

struct Recovered {
    inner: Arc<dyn Model>,
    purpose: Purpose,
    stop: CancellationToken,
    on_retry: Option<Arc<dyn Fn(retry::RetryNotice) + Send + Sync>>,
}

#[async_trait]
impl Model for Recovered {
    async fn call(
        &self,
        request: Request,
        cancel: CancellationToken,
    ) -> Result<ModelStream, Error> {
        let policy = match self.purpose {
            Purpose::Compaction => {
                retry::Policy::new(super::summary::RETRIES, super::summary::RETRY_DELAY, true)
            }
            Purpose::Main => retry::Policy::new(5, Duration::from_secs(60), false),
        };
        Ok(retry::stream_observed(
            self.inner.clone(),
            request,
            policy,
            cancel,
            self.stop.clone(),
            self.on_retry.clone(),
        ))
    }
}
