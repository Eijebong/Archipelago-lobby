use opentelemetry::{global, trace::TracerProvider, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    propagation::TraceContextPropagator,
    runtime,
    trace::{BatchConfig, RandomIdGenerator, Tracer},
    Resource,
};
use opentelemetry_semantic_conventions::{
    resource::{DEPLOYMENT_ENVIRONMENT, SERVICE_NAME},
    SCHEMA_URL,
};
use rocket::{
    fairing::{Fairing, Info, Kind},
    Request, Response,
};
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn resource() -> Resource {
    Resource::from_schema_url(
        [
            KeyValue::new(SERVICE_NAME, env!("CARGO_PKG_NAME")),
            KeyValue::new(
                DEPLOYMENT_ENVIRONMENT,
                std::env::var("ROCKET_PROFILE").unwrap_or_else(|_| "dev".to_string()),
            ),
        ],
        SCHEMA_URL,
    )
}

fn init_tracer(endpoint: &str) -> Tracer {
    let provider = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_trace_config(
            opentelemetry_sdk::trace::Config::default()
                .with_id_generator(RandomIdGenerator::default())
                .with_resource(resource()),
        )
        .with_batch_config(BatchConfig::default())
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint),
        )
        .install_batch(runtime::Tokio)
        .unwrap();

    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());
    provider.tracer("tracing-otel-subscriber")
}

pub fn init_tracing_subscriber(endpoint: Option<String>) -> OtelGuard {
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(
            Level::INFO,
        ))
        .with(tracing_subscriber::fmt::layer())
        .with(sentry_tracing::layer());

    if let Some(endpoint) = endpoint {
        let tracer = init_tracer(&endpoint);
        subscriber.with(OpenTelemetryLayer::new(tracer)).init();
    } else {
        subscriber.init();
        tracing::warn!("No OTLP_ENDPOINT specified, not enabling opentelemetry");
    };

    OtelGuard {}
}

pub struct OtelGuard {}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        opentelemetry::global::shutdown_tracer_provider();
    }
}

pub struct TracingFairing;

#[rocket::async_trait]
impl Fairing for TracingFairing {
    fn info(&self) -> Info {
        Info {
            name: "Tracing Fairing",
            kind: Kind::Response,
        }
    }

    async fn on_response<'r>(&self, req: &'r Request<'_>, _res: &mut Response<'r>) {
        let current_span = Span::current();
        let Some(route) = req.route() else { return };
        current_span.record("otel.name", route.uri.to_string());
    }
}
