use std::time::Instant;

use diesel::connection::Instrumentation;
use once_cell::sync::Lazy;
use prometheus::{HistogramOpts, HistogramVec};

#[derive(Default)]
pub struct DbInstrumentation {
    query_start: Option<Instant>,
}

pub static QUERY_HISTOGRAM: Lazy<HistogramVec> = Lazy::new(|| {
    HistogramVec::new(
        HistogramOpts::new("diesel_query_seconds", "SQL query duration").buckets(vec![
            0.000005, 0.00001, 0.00005, 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.1, 1.0,
        ]),
        &["query"],
    )
    .expect("Failed to create query histogram")
});

impl Instrumentation for DbInstrumentation {
    fn on_connection_event(&mut self, event: diesel::connection::InstrumentationEvent<'_>) {
        match event {
            diesel::connection::InstrumentationEvent::StartQuery { .. } => {
                tracing::event!(tracing::Level::INFO, "Query started");
                self.query_start = Some(Instant::now());
            }
            diesel::connection::InstrumentationEvent::FinishQuery { query, .. } => {
                let Some(query_start) = self.query_start else {
                    return;
                };
                let elapsed = query_start.elapsed();
                let query = query.to_string().replace('\n', " ");
                let query = query.split("--").next().unwrap().trim();
                QUERY_HISTOGRAM
                    .with_label_values(&[query])
                    .observe(elapsed.as_secs_f64());
                tracing::event!(tracing::Level::INFO, %query, "Query finished");
            }
            diesel::connection::InstrumentationEvent::StartEstablishConnection { .. } => {
                tracing::event!(tracing::Level::INFO, "StartEstablishConnection");
            }
            diesel::connection::InstrumentationEvent::FinishEstablishConnection { .. } => {
                tracing::event!(tracing::Level::INFO, "FinishEstablishConnection");
            }
            diesel::connection::InstrumentationEvent::CacheQuery { .. } => {
                tracing::event!(tracing::Level::INFO, "CacheQuery");
            }
            diesel::connection::InstrumentationEvent::BeginTransaction { .. } => {
                tracing::event!(tracing::Level::INFO, "BeginTransaction");
            }
            diesel::connection::InstrumentationEvent::CommitTransaction { .. } => {
                tracing::event!(tracing::Level::INFO, "CommitTransaction");
            }
            diesel::connection::InstrumentationEvent::RollbackTransaction { .. } => {
                tracing::event!(tracing::Level::INFO, "RollbackTransaction");
            }
            _ => {}
        };
    }
}
