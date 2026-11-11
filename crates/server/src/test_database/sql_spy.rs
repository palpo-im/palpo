//! SQL evidence for isolated, disposable-database tests only.
//!
//! This replaces each query-pool connection's no-op instrumentation. Diesel
//! cannot transfer an arbitrary existing instrumentation object out, so callers
//! must use a dedicated test process without other custom SQL instrumentation.
//! The global constructor is never changed; uninstall restores no-op hooks.
//! Query strings may include fixture bind values: never expose this through
//! production metrics or run it against a database containing private data.
use std::sync::{Arc, Mutex};

use diesel::connection::{Instrumentation, InstrumentationEvent, get_default_instrumentation};
use diesel_async::AsyncConnection;

type Queries = Arc<Mutex<Vec<String>>>;

struct QueryCounter(Queries);

impl Instrumentation for QueryCounter {
    fn on_connection_event(&mut self, event: InstrumentationEvent<'_>) {
        if let InstrumentationEvent::StartQuery { query, .. } = event {
            self.0.lock().unwrap().push(query.to_string());
        }
    }
}

pub(crate) struct SqlSpy {
    queries: Queries,
}

async fn all_connections() -> Vec<crate::data::PgPooledConnection> {
    let mut held = Vec::new();
    for _ in 0..crate::data::status().max_size {
        held.push(crate::data::checkout().await.unwrap());
    }
    held
}

impl SqlSpy {
    pub(crate) async fn install() -> Self {
        assert!(
            get_default_instrumentation().is_none(),
            "SQL spy requires an isolated test without default instrumentation"
        );
        let queries = Arc::new(Mutex::new(Vec::new()));
        for mut connection in all_connections().await {
            connection.set_instrumentation(QueryCounter(Arc::clone(&queries)));
        }
        Self { queries }
    }

    pub(crate) fn reset(&self) {
        self.queries.lock().unwrap().clear();
    }

    pub(crate) fn snapshot(&self) -> Vec<String> {
        self.queries.lock().unwrap().clone()
    }

    /// Detect replacement connections rather than silently undercounting after
    /// a reconnect. Call outside the measured interval: checkout can run SQL.
    pub(crate) async fn assert_coverage(&self) {
        for mut connection in all_connections().await {
            let counter = connection
                .instrumentation()
                .downcast_ref::<QueryCounter>()
                .expect("query connection lost its SQL spy instrumentation");
            assert!(Arc::ptr_eq(&counter.0, &self.queries));
        }
    }

    pub(crate) async fn uninstall(self) {
        for mut connection in all_connections().await {
            let counter = connection
                .instrumentation()
                .downcast_ref::<QueryCounter>()
                .expect("SQL spy cannot overwrite another instrumentation hook");
            assert!(Arc::ptr_eq(&counter.0, &self.queries));
            connection.set_instrumentation(None::<Box<dyn Instrumentation>>);
        }
    }
}
