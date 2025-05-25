#![cfg(test)]
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::time::sleep;
use tracing_async2::{CallbackLayer, OwnedEvent};
use tracing_async2_macros::*;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

#[tokio::test]
async fn test_trace_events() {
    #[derive(Default, Clone)]
    struct Counters {
        count_trace: u32,
        count_debug: u32,
        count_info: u32,
        count_warn: u32,
        count_error: u32,
    }

    #[tracing_layer_async_within]
    pub async fn tracing_layer_with_counters(
        counts: Arc<RwLock<Counters>>,
        evt: OwnedEvent,
    ) -> Result<(), String> {
        use tracing_async2::TracingLevel::*;
        if let Ok(mut counts) = counts.write() {
            match evt.level {
                Trace => counts.count_trace += 1,
                Debug => counts.count_debug += 1,
                Info => counts.count_info += 1,
                Warn => counts.count_warn += 1,
                Error => counts.count_error += 1,
            }
        } else {
            return Err("could not write counters".to_string());
        }
        Ok(())
    }

    let counters = Arc::new(RwLock::new(Counters::default()));

    let _guard = tracing_subscriber::registry()
        .with(EnvFilter::new("tracing_async2_macros_test=trace"))
        .with(tracing_layer_with_counters(counters.clone(), 16))
        .set_default();

    tracing::trace!(foo = 1, bar = 2, "this is a trace message");
    tracing::debug!(pi = 3.14159265, "this is a debug message");
    tracing::info!(job = "foo", "this is an info message");
    tracing::warn!(job = "foo", "this is a warning message");
    tracing::error!(job = "foo", "this is an error message");

    sleep(Duration::from_millis(500)).await;

    let c = counters.read().expect("could not read counters");

    assert_eq!(c.count_trace, 1);
    assert_eq!(c.count_debug, 1);
    assert_eq!(c.count_info, 1);
    assert_eq!(c.count_warn, 1);
    assert_eq!(c.count_error, 1);
}
