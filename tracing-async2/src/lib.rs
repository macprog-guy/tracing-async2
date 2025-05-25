//!
//! This crate makes it easy to create your own custom tracing layers using a
//! simple callback mechanism. One abvious use is to store tracing events into
//! a database but you could just as well send them to a downstream service
//! using gRPC or http. Or, for testing purposes, to accumulate them into a
//! vector.
//!
//! This crate provides a set of tracing layers that can be used to process
//! [`tracing::Event`]s using simple callbacks or even in an asynchronous
//! context. This is done using variations the [`CallbackLayer`].
//!
//!
//! # Using the `tracing_layer_async_within` macro
//!
//! This macro simplifies some async scenarios where the provided callback was
//! not `Sync` or `Send`. Here is an example of how you could use this macro to
//! create a layer that saves tracing events into a database using `tokio_postgres`:
//!
//! ```no_run
//! #[tracing_layer_async_within]
//! pub fn pg_tracing_layer(client: &PGClient, event: OwnedEvent) -> Result<(), tokio_postgres::Error> {
//!     // Do what needs to be done!
//! }
//! ```
//!
//! The above code gets expanded into the code below:
//!
//! ```no_run
//! pub fn pg_tracing_layer(client: PGClient, buffer_size: usize) -> CallbackLayer {
//!
//!     let (tx, mut rx) = mpsc::channel::<OwnedEvent>(buffer_size);
//!
//!     tokio::spawn(async move {
//!         let client = Arc::new(client);
//!         while let Some(event) = rx.recv().await {
//!             if let Err(e) = save_tracing_event_to_database(&client, event).await {
//!                 eprintln!("{} error: {}", "pg_tracing_layer", e);
//!             }
//!         }
//!     });
//!
//!     pub async fn save(
//!        client: &Arc<tokio_postgres::Client>,
//!        event: OwnedEvent,
//!     ) -> Result<(), tokio_postgres::Error> {
//!         // Do what needs to be done!
//!     }
//!
//!     channel_layer(tx)
//! }
//! ```
//!
//! Of note are the following:
//! - The `PGClient` was declared as a reference but the generated returned function requires it to be owned.
//! - `buffer_size` is an additional parameter to the generated function.
//!
//!
//!
//! # Using `callback_layer`
//!
//! If your needs are really simple, like accumulating traces in a vector.
//! Then you can use the [`callback_layer`]:
//!
//! ```rust
//! use tracing_async2::{callback_layer, OwnedEvent};
//! use tracing_subscriber::{EnvFilter, prelude::*};
//! use std::sync::{Arc, RwLock};
//!
//! let log = Arc::new(RwLock::new(Vec::new()));
//! let cb_log = log.clone();
//!
//! tracing_subscriber::registry()
//!     .with(EnvFilter::new("tracing_async2=trace"))
//!     .with(callback_layer(move |event| {
//!         if let Ok(mut log) = cb_log.write() {
//!             log.push(OwnedEvent::from(event));
//!         }
//!     }))
//!     .init();
//! ```
//!
//!
//! # Using `channel_layer`
//!
//! If you ever had the need to record traces in a database or do something
//! asynchronously with [`tracing::Event`], then you can use the
//! [`channel_layer`]:
//!
//! ```rust
//! use tracing_async2::channel_layer;
//! use tracing_subscriber::{EnvFilter, prelude::*};
//! use tokio::{sync::mpsc, runtime, task};
//!
//! let rt = tokio::runtime::Builder::new_current_thread()
//!     .build()
//!     .expect("could not start tokio runtime");
//!
//! rt.block_on(async move {
//!
//!     let (tx, mut rx) = mpsc::channel(100);
//!     tracing_subscriber::registry()
//!         .with(EnvFilter::new("tracing_async2=trace"))
//!         .with(channel_layer(tx)) // <--- use the channel
//!         .init();
//!
//!     tokio::spawn(async move {
//!         while let Some(event) = rx.recv().await {
//!             // Do something with the event like saving it to the database.
//!         }
//!     });
//! });
//! ```
//!
//! # Using `async_layer`
//!
//! If you don't want to handle the channel yourself, then you might consider
//! the use of [`async_layer`] instead:
//!
//! ```rust
//! use tracing_async2::async_layer;
//! use tracing_subscriber::{EnvFilter, prelude::*};
//! use tokio::{sync::mpsc, runtime, task};
//!
//! let rt = tokio::runtime::Builder::new_current_thread()
//!     .build()
//!     .expect("could not start tokio runtime");
//!
//! rt.block_on(async move {
//!
//!     tracing_subscriber::registry()
//!         .with(EnvFilter::new("tracing_async2=trace"))
//!         .with(async_layer(16, move |event| {
//!             async move {
//!                 // Do something with the event like saving it to the database.
//!             }
//!         }))
//!         .init();
//! });
//! ```
//!
#[cfg(feature = "async")]
use tokio::sync::mpsc::{self, error::TrySendError};

#[cfg(feature = "accumulator")]
use std::sync::{Arc, RwLock};

use {
    std::fmt::{self, Display},
    tracing::{
        Event, Subscriber,
        field::{Field, Visit},
        span,
    },
    tracing_subscriber::{Layer, layer::Context, registry::LookupSpan},
};

#[cfg(feature = "macros")]
pub use tracing_async2_macros::*;

#[cfg(feature = "span")]
type JsonMap = serde_json::Map<String, serde_json::Value>;

///
/// Returns a simple callback layer that will call the provided callback
/// on each relevant event.
///
pub fn callback_layer<F>(callback: F) -> CallbackLayer
where
    F: Fn(&Event<'_>) + Send + Sync + 'static,
{
    CallbackLayer::new(callback)
}

///
/// Returns a simple callback layer that will call the provided callback
/// on each relevant event and will include parent spans.
///
#[cfg(feature = "span")]
pub fn callback_layer_with_spans<F>(callback: F) -> CallbackLayerWithSpan
where
    F: Fn(&Event<'_>, Option<Vec<JsonMap>>) + Send + Sync + 'static,
{
    CallbackLayerWithSpan::new(callback)
}

///
/// Returns a layer that will send an [`OwnedEvent`] on the provided channel
/// on each relevant event.
///
#[cfg(feature = "async")]
pub fn channel_layer(tx: mpsc::Sender<OwnedEvent>) -> CallbackLayer {
    CallbackLayer::new(move |event: &Event<'_>| {
        tx.try_send(event.into()).ok();
    })
}

///
/// Returns a layer that will send an [`OwnedEvent`] on the provided channel
/// on each relevant event along with a vector of parent spans.
///
#[cfg(all(feature = "async", feature = "span"))]
pub fn channel_layer_with_spans(tx: mpsc::Sender<OwnedEventWithSpans>) -> CallbackLayerWithSpan {
    CallbackLayerWithSpan::new(move |event, spans| {
        if let Err(e) = tx.try_send(OwnedEventWithSpans::new(event, spans)) {
            match e {
                TrySendError::Full(o) => {
                    eprintln!("dropping tracing event: {:?}", o);
                }
                TrySendError::Closed(o) => {
                    eprintln!("channel closed for tracing event: {:?}", o);
                }
            }
        }
    })
}

///
/// Returns a layer that will call an async callback on each relevant event.
/// This type of layer can be used, for example, to save tracing events into
/// a database.
///
/// Note that this is NOT an async closure but a normal function that
/// returns a future. In practice that will often be a function whose return
/// value is an async block that is not awaited.
///
#[cfg(feature = "async")]
pub fn async_layer<F, Fut>(buffer_size: usize, callback: F) -> CallbackLayer
where
    F: Fn(OwnedEvent) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + Sync + 'static,
{
    let (tx, mut rx) = mpsc::channel(buffer_size);
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            callback(event).await;
        }
    });
    channel_layer(tx)
}

///
/// Returns a layer that will call an async callback on each relevant event
/// along with parent spans. This type of layer can be used, for example,
/// to save tracing events into a database.
///
/// Note that this is NOT an async closure but a normal function that
/// returns a future. In practice that will often be a function whose return
/// value is an async block that is not awaited.
///
#[cfg(all(feature = "async", feature = "span"))]
pub fn async_layer_with_spans<F, Fut>(buffer_size: usize, callback: F) -> CallbackLayerWithSpan
where
    F: Fn(OwnedEventWithSpans) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel(buffer_size);
    tokio::spawn(async move {
        while let Some(owned_with_span) = rx.recv().await {
            callback(owned_with_span).await;
        }
    });
    channel_layer_with_spans(tx)
}

#[cfg(feature = "accumulator")]
pub type AccumulatingLog = Arc<RwLock<Vec<OwnedEvent>>>;

///
/// Returns a layer that accumulates [`OwnedEvent`] into a shared vector.
/// This can be useful for testing and achives something similar to what the
/// tracing_test crate does but without the extra function injections.
///
#[cfg(feature = "accumulator")]
pub fn accumulating_layer(log: AccumulatingLog) -> CallbackLayer {
    CallbackLayer::new(move |event: &Event<'_>| {
        if let Ok(mut log) = log.write() {
            log.push(event.into());
        }
    })
}

///
/// Returns a layer that accumulates [`OwnedEvent`] and parent spans into a
/// shared vector. This can be useful for testing and achives something similar
/// to what the tracing_test crate does but without the extra function injections.
///
#[cfg(all(feature = "accumulator", feature = "span"))]
pub fn accumulating_layer_with_spans(
    log: Arc<RwLock<Vec<OwnedEventWithSpans>>>,
) -> CallbackLayerWithSpan {
    CallbackLayerWithSpan::new(move |event: &Event<'_>, spans| {
        if let Ok(mut log) = log.write() {
            log.push(OwnedEventWithSpans::new(event, spans));
        }
    })
}

///
/// [`CallbackLayer`] is the simple but is the basis for all of the functional
/// variations available in this crate. All it does is execute a callback for
/// every tracing event after filtering. If you require [`tracing::span::Span`]
/// information then use the [`CallbackLayerWithSpan`] instead.
///
pub struct CallbackLayer {
    callback: Box<dyn Fn(&Event<'_>) + Send + Sync + 'static>,
}

impl CallbackLayer {
    pub fn new<F: Fn(&Event<'_>) + Send + Sync + 'static>(callback: F) -> Self {
        let callback = Box::new(callback);
        Self { callback }
    }
}

impl<S> Layer<S> for CallbackLayer
where
    S: Subscriber,
    S: for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        (self.callback)(event);
    }
}

///
/// [`CallbackLayerWithSpan`] is similar to [`CallbackLayer`] but it also includes parent spans.
/// Therefor the provided callback must accept two parameters. The callback is executed for
/// every tracing event after filtering. If you don't require [`span::Span`] information
/// then use the [`CallbackLayer`] instead.
///
#[cfg(feature = "span")]
pub struct CallbackLayerWithSpan {
    #[allow(clippy::type_complexity)]
    callback: Box<dyn Fn(&Event<'_>, Option<Vec<JsonMap>>) + Send + Sync + 'static>,
}

#[cfg(feature = "span")]
impl CallbackLayerWithSpan {
    pub fn new<F: Fn(&Event<'_>, Option<Vec<JsonMap>>) + Send + Sync + 'static>(
        callback: F,
    ) -> Self {
        let callback = Box::new(callback);
        Self { callback }
    }
}

#[cfg(feature = "span")]
impl<S> Layer<S> for CallbackLayerWithSpan
where
    S: Subscriber,
    S: for<'lookup> tracing_subscriber::registry::LookupSpan<'lookup>,
{
    fn on_new_span(&self, attrs: &span::Attributes<'_>, id: &tracing::Id, ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        attrs.record(&mut visitor);
        if let Some(span) = ctx.span(id) {
            span.extensions_mut().insert(visitor);
        }
    }
    fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
        let spans: Option<Vec<_>> = ctx.event_scope(event).map(|scope| {
            scope
                .from_root()
                .map(|span| {
                    let fields: Option<JsonMap> = span
                        .extensions()
                        .get::<FieldVisitor>()
                        .cloned()
                        .map(|FieldVisitor(json_map)| json_map);

                    let meta = span.metadata();
                    let mut map = JsonMap::default();
                    map.insert("level".into(), format!("{}", meta.level()).into());
                    map.insert("file".into(), meta.file().into());
                    map.insert("target".into(), meta.target().into());
                    map.insert("line".into(), meta.line().into());
                    map.insert("name".into(), span.name().into());
                    map.insert("fields".into(), fields.into());
                    map
                })
                .collect()
        });
        (self.callback)(event, spans);
    }
}

///
/// # OwnedEvent
///
/// [`OwnedEvent`] is the an owned version of the [`Event`], which
/// is not `Send`. Our event must be `Send` to be sent on a channel.
///
#[derive(Debug, Clone, serde::Serialize)]
pub struct OwnedEvent {
    pub level: TracingLevel,
    pub file: Option<String>,
    pub target: String,
    pub line: Option<u32>,
    pub name: &'static str,
    pub message: Option<String>,
    pub fields: serde_json::Map<String, serde_json::Value>,
}

///
/// # OwnedEvent
///
/// [`OwnedEventWithSpan`] is the an owned version of the [`Event`],
/// which is not `Send` with their accompanying parent spans.
///
#[derive(Debug, Clone, serde::Serialize)]
pub struct OwnedEventWithSpans {
    pub event: OwnedEvent,
    pub spans: Option<Vec<JsonMap>>,
}

impl OwnedEventWithSpans {
    pub fn new(event: &Event<'_>, spans: Option<Vec<JsonMap>>) -> Self {
        OwnedEventWithSpans {
            event: event.into(),
            spans,
        }
    }
}

///
/// Same as [`tracing::Level`] but serializable.
///
#[derive(Copy, Clone, Debug, Hash, Eq, PartialEq, serde::Serialize)]
pub enum TracingLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl From<&tracing::Level> for TracingLevel {
    fn from(value: &tracing::Level) -> Self {
        match *value {
            tracing::Level::TRACE => TracingLevel::Trace,
            tracing::Level::DEBUG => TracingLevel::Debug,
            tracing::Level::INFO => TracingLevel::Info,
            tracing::Level::WARN => TracingLevel::Warn,
            tracing::Level::ERROR => TracingLevel::Error,
        }
    }
}

impl AsRef<str> for TracingLevel {
    fn as_ref(&self) -> &str {
        use TracingLevel::*;
        match self {
            Trace => "TRACE",
            Debug => "DEBUG",
            Info => "INFO",
            Warn => "WARN",
            Error => "ERROR",
        }
    }
}

impl Display for TracingLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_ref())
    }
}

///
/// Converts a [`tracing::Event`] to a [`OwnedEvent`].
///
/// The conversion essentially makes the `Event` an owned value,
/// which is necessary for it to be `Send` and hence to be sent on a channel.
///
impl From<&Event<'_>> for OwnedEvent {
    fn from(event: &Event<'_>) -> Self {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);

        // We make an exception for the message field because it's usually
        // the one we are always interested in.
        let message = visitor.0.remove("message").and_then(|v| {
            if let serde_json::Value::String(s) = v {
                Some(s)
            } else {
                None
            }
        });

        let meta = event.metadata();
        Self {
            name: meta.name(),
            target: meta.target().into(),
            level: meta.level().into(),
            file: meta.file().map(String::from),
            line: meta.line(),
            message,
            fields: visitor.0,
        }
    }
}

///
/// # FieldVisitor
///
/// Private structure that allows us to collect field values into as Json map.
///
#[cfg(feature = "span")]
#[derive(Default, Clone)]
struct FieldVisitor(serde_json::Map<String, serde_json::Value>);

impl Visit for FieldVisitor {
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().into(), value.into());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        let text = format!("{:?}", value);
        self.0.insert(field.name().into(), text.into());
    }
}

// ----------------------------------------------------------------------------
//
// Unit Testing
//
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {

    #[cfg(feature = "async")]
    use {
        std::sync::{Arc, RwLock},
        tokio::sync::mpsc,
    };

    use {
        super::*,
        insta::{
            internals::{Content, ContentPath},
            *,
        },
        tracing_subscriber::{EnvFilter, prelude::*},
    };

    // ----------------------------------------------------------------------------
    // Helper Functions
    // ----------------------------------------------------------------------------

    fn run_trace_events() {
        let span = tracing::info_span!("root-info", recurse = 0);
        span.in_scope(|| {
            tracing::trace!(foo = 1, bar = 2, "this is a trace message");
            tracing::debug!(pi = 3.14159265, "this is a debug message");
            tracing::info!(job = "foo", "this is an info message");
            tracing::warn!(job = "foo", "this is a warning message");
            tracing::error!(job = "foo", "this is an error message");
        });
    }

    fn extract_events<T: Clone>(logs: &Arc<RwLock<Vec<T>>>) -> Vec<T> {
        let events = logs.read().expect("could not read events");
        events.clone()
    }

    fn redact_name(value: Content, _path: ContentPath) -> String {
        let s = value.as_str().unwrap_or_default();
        if s.contains(":") {
            s.split_once(":")
                .map(|p| format!("{}:<line>", p.0))
                .unwrap_or_default()
        } else {
            s.to_string()
        }
    }

    // ----------------------------------------------------------------------------
    // Tests
    // ----------------------------------------------------------------------------

    #[test]
    fn test_callback_layer() {
        //
        // Collect logs into a vector
        let events = Arc::new(RwLock::new(Vec::<OwnedEvent>::new()));
        let cb_events = events.clone();

        let _guard = tracing_subscriber::registry()
            .with(EnvFilter::new("tracing_async2=trace"))
            .with(callback_layer(move |event| {
                if let Ok(mut events) = cb_events.write() {
                    events.push(event.into());
                }
            }))
            .set_default();

        run_trace_events();

        assert_json_snapshot!("callback-layer", extract_events(&events), {
            "[].line" => "<line>",
            "[].name" => dynamic_redaction(redact_name),
        });
    }

    #[cfg(feature = "span")]
    #[test]
    fn test_callback_layer_with_spans() {
        //
        // Collect logs into a vector
        let events = Arc::new(RwLock::new(Vec::<OwnedEventWithSpans>::new()));
        let cb_events = events.clone();

        let _guard = tracing_subscriber::registry()
            .with(EnvFilter::new("tracing_async2=trace"))
            .with(callback_layer_with_spans(move |event, spans| {
                if let Ok(mut events) = cb_events.write() {
                    events.push(OwnedEventWithSpans::new(event, spans));
                }
            }))
            .set_default();

        run_trace_events();

        assert_json_snapshot!("callback-layer-with-spans", extract_events(&events), {
            "[].event.line" => "<line>",
            "[].event.name" => dynamic_redaction(redact_name),
            "[].spans[].line" => "<line>",
            "[].spans[].name" => dynamic_redaction(redact_name),
        });
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn test_channel_layer() {
        //
        use std::time::Duration;
        use tokio::time::sleep;

        //
        // Collect logs into a vector
        let events = Arc::new(RwLock::new(Vec::<OwnedEvent>::new()));
        let cb_events = events.clone();

        let (tx, mut rx) = mpsc::channel(100);

        let _guard = tracing_subscriber::registry()
            .with(EnvFilter::new("tracing_async2=trace"))
            .with(channel_layer(tx))
            .set_default();

        let handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Ok(mut events) = cb_events.write() {
                    events.push(event);
                }
            }
        });

        run_trace_events();
        sleep(Duration::from_millis(100)).await;
        handle.abort();

        assert_json_snapshot!("channel-layer", extract_events(&events), {
            "[].line" => "<line>",
            "[].name" => dynamic_redaction(redact_name),
        });
    }

    #[cfg(all(feature = "async", feature = "span"))]
    #[tokio::test]
    async fn test_channel_layer_with_spans() {
        //
        use std::time::Duration;
        use tokio::time::sleep;

        //
        // Collect logs into a vector
        let events = Arc::new(RwLock::new(Vec::<OwnedEventWithSpans>::new()));
        let cb_events = events.clone();

        let (tx, mut rx) = mpsc::channel(100);

        let _guard = tracing_subscriber::registry()
            .with(EnvFilter::new("tracing_async2=trace"))
            .with(channel_layer_with_spans(tx))
            .set_default();

        let handle = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                if let Ok(mut events) = cb_events.write() {
                    events.push(event);
                }
            }
        });

        run_trace_events();
        sleep(Duration::from_millis(100)).await;
        handle.abort();

        assert_json_snapshot!("channel-layer-with-spans", extract_events(&events), {
            "[].event.line" => "<line>",
            "[].event.name" => dynamic_redaction(redact_name),
            "[].spans[].line" => "<line>",
            "[].spans[].name" => dynamic_redaction(redact_name),
        });
    }

    #[cfg(feature = "async")]
    #[tokio::test]
    async fn test_async_layer() {
        //
        use std::time::Duration;
        use tokio::time::sleep;

        //
        // Collect logs into a vector
        let events = Arc::new(RwLock::new(Vec::<OwnedEvent>::new()));
        let cb_events = events.clone();

        let _guard = tracing_subscriber::registry()
            .with(EnvFilter::new("tracing_async2=trace"))
            .with(async_layer(16, move |event| {
                let f_events = cb_events.clone();
                async move {
                    if let Ok(mut events) = f_events.write() {
                        events.push(event);
                    }
                }
            }))
            .set_default();

        run_trace_events();
        sleep(Duration::from_millis(100)).await;

        assert_json_snapshot!("async-layer", extract_events(&events), {
            "[].line" => "<line>",
            "[].name" => dynamic_redaction(redact_name),
        });
    }

    #[cfg(all(feature = "async", feature = "span"))]
    #[tokio::test]
    async fn test_async_layer_with_spans() {
        //
        use std::time::Duration;
        use tokio::time::sleep;

        //
        // Collect logs into a vector
        let events = Arc::new(RwLock::new(Vec::<OwnedEventWithSpans>::new()));
        let cb_events = events.clone();

        let _guard = tracing_subscriber::registry()
            .with(EnvFilter::new("tracing_async2=trace"))
            .with(async_layer_with_spans(16, move |event_with_span| {
                let f_events = cb_events.clone();
                async move {
                    if let Ok(mut events) = f_events.write() {
                        events.push(event_with_span);
                    }
                }
            }))
            .set_default();

        run_trace_events();
        sleep(Duration::from_millis(100)).await;

        assert_json_snapshot!("async-layer-with-spans", extract_events(&events), {
            "[].event.line" => "<line>",
            "[].event.name" => dynamic_redaction(redact_name),
            "[].spans[].line" => "<line>",
            "[].spans[].name" => dynamic_redaction(redact_name),
        });
    }

    #[cfg(feature = "accumulator")]
    #[test]
    fn test_accumulating_layer() {
        //
        // Collect logs into a vector
        let events = Arc::new(RwLock::new(Vec::<OwnedEvent>::new()));

        let _guard = tracing_subscriber::registry()
            .with(EnvFilter::new("tracing_async2=trace"))
            .with(accumulating_layer(events.clone()))
            .set_default();

        run_trace_events();

        assert_json_snapshot!("accumulating-layer", extract_events(&events), {
            "[].line" => "<line>",
            "[].name" => dynamic_redaction(redact_name),
        });
    }

    #[cfg(all(feature = "span", feature = "accumulator"))]
    #[test]
    fn test_accumulating_layer_with_spans() {
        //
        // Collect logs into a vector
        let events = Arc::new(RwLock::new(Vec::<OwnedEventWithSpans>::new()));

        let _guard = tracing_subscriber::registry()
            .with(EnvFilter::new("tracing_async2=trace"))
            .with(accumulating_layer_with_spans(events.clone()))
            .set_default();

        run_trace_events();

        assert_json_snapshot!("accumulating-layer-with-spans", extract_events(&events), {
            "[].event.line" => "<line>",
            "[].event.name" => dynamic_redaction(redact_name),
            "[].spans[].line" => "<line>",
            "[].spans[].name" => dynamic_redaction(redact_name),
        });
    }
}
