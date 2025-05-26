# tracing-async2

![Maintenance](https://img.shields.io/badge/maintenance-activly--developed-brightgreen.svg)
[![crates.io](https://img.shields.io/crates/v/tracing-async2)](https://crates.io/crates/tracing-async2)

This crate makes it easy to create your own custom tracing layers using a
simple callback mechanism. One abvious use is to store tracing events into
a database but you could just as well send them to a downstream service
using gRPC or http. Or, for testing purposes, to accumulate them into a
vector.

This crate provides a set of tracing layers that can be used to process
[`tracing::Event`]s using simple callbacks or even in an asynchronous
context. This is done using variations the [`CallbackLayer`].


## Using the `tracing_layer_async_within` macro

This macro simplifies some async scenarios where the provided callback was
not `Sync` or `Send`. Here is an example of how you could use this macro to
create a layer that saves tracing events into a database using `tokio_postgres`:

```rust
#[tracing_layer_async_within]
pub fn pg_tracing_layer(client: &PGClient, event: OwnedEvent) -> Result<(), tokio_postgres::Error> {
    // Do what needs to be done!
}
```

The above code gets expanded into the code below:

```rust
pub fn pg_tracing_layer(client: PGClient, buffer_size: usize) -> CallbackLayer {

    let (tx, mut rx) = mpsc::channel::<OwnedEvent>(buffer_size);

    tokio::spawn(async move {
        let client = Arc::new(client);
        while let Some(event) = rx.recv().await {
            if let Err(e) = save_tracing_event_to_database(&client, event).await {
                eprintln!("{} error: {}", "pg_tracing_layer", e);
            }
        }
    });

    pub async fn save(
       client: &Arc<tokio_postgres::Client>,
       event: OwnedEvent,
    ) -> Result<(), tokio_postgres::Error> {
        // Do what needs to be done!
    }

    channel_layer(tx)
}
```

Of note are the following:
- The `PGClient` was declared as a reference but the generated returned function requires it to be owned.
- `buffer_size` is an additional parameter to the generated function.



## Using `callback_layer`

If your needs are really simple, like accumulating traces in a vector.
Then you can use the [`callback_layer`]:

```rust
use tracing_async2::{callback_layer, OwnedEvent};
use tracing_subscriber::{EnvFilter, prelude::*};
use std::sync::{Arc, RwLock};

let log = Arc::new(RwLock::new(Vec::new()));
let cb_log = log.clone();

tracing_subscriber::registry()
    .with(EnvFilter::new("tracing_async2=trace"))
    .with(callback_layer(move |event| {
        if let Ok(mut log) = cb_log.write() {
            log.push(OwnedEvent::from(event));
        }
    }))
    .init();
```


## Using `channel_layer`

If you ever had the need to record traces in a database or do something
asynchronously with [`tracing::Event`], then you can use the
[`channel_layer`]:

```rust
use tracing_async2::channel_layer;
use tracing_subscriber::{EnvFilter, prelude::*};
use tokio::{sync::mpsc, runtime, task};

let rt = tokio::runtime::Builder::new_current_thread()
    .build()
    .expect("could not start tokio runtime");

rt.block_on(async move {

    let (tx, mut rx) = mpsc::channel(100);
    tracing_subscriber::registry()
        .with(EnvFilter::new("tracing_async2=trace"))
        .with(channel_layer(tx)) // <--- use the channel
        .init();

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            // Do something with the event like saving it to the database.
        }
    });
});
```

## Using `async_layer`

If you don't want to handle the channel yourself, then you might consider
the use of [`async_layer`] instead:

```rust
use tracing_async2::async_layer;
use tracing_subscriber::{EnvFilter, prelude::*};
use tokio::{sync::mpsc, runtime, task};

let rt = tokio::runtime::Builder::new_current_thread()
    .build()
    .expect("could not start tokio runtime");

rt.block_on(async move {

    tracing_subscriber::registry()
        .with(EnvFilter::new("tracing_async2=trace"))
        .with(async_layer(16, move |event| {
            async move {
                // Do something with the event like saving it to the database.
            }
        }))
        .init();
});
```


License: MIT
