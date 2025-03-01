use opentelemetry::{global, trace::TracerProvider, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::{
    propagation::TraceContextPropagator,
    trace::{RandomIdGenerator, SdkTracerProvider},
    Resource,
};
use opentelemetry_semantic_conventions::attribute::DEPLOYMENT_ENVIRONMENT_NAME;
use rocket::{
    fairing::{Fairing, Info, Kind},
    Request, Response,
};
use tracing::{Level, Span};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn resource() -> Resource {
    Resource::builder()
        .with_service_name(env!("CARGO_PKG_NAME"))
        .with_attribute(KeyValue::new(
            DEPLOYMENT_ENVIRONMENT_NAME,
            std::env::var("ROCKET_PROFILE").unwrap_or_else(|_| "dev".to_string()),
        ))
        .build()
}

fn init_tracer(endpoint: &str) -> SdkTracerProvider {
    let provider = SdkTracerProvider::builder()
        .with_id_generator(RandomIdGenerator::default())
        .with_resource(resource())
        .with_batch_exporter(
            opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .build()
                .unwrap(),
        )
        .build();

    global::set_tracer_provider(provider.clone());
    global::set_text_map_propagator(TraceContextPropagator::new());
    provider
}

pub fn init_tracing_subscriber(endpoint: Option<String>) -> OtelGuard {
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(
            Level::INFO,
        ))
        .with(tracing_subscriber::fmt::layer())
        .with(sentry_tracing::layer());

    let provider = if let Some(endpoint) = endpoint {
        let provider = init_tracer(&endpoint);
        let tracer = provider.tracer("tracing-otel-subscriber");
        subscriber.with(OpenTelemetryLayer::new(tracer)).init();

        Some(provider)
    } else {
        subscriber.init();
        tracing::warn!("No OTLP_ENDPOINT specified, not enabling opentelemetry");
        None
    };

    OtelGuard { provider }
}

pub struct OtelGuard {
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Some(tracer) = self.provider.take() {
            let _ = tracer.shutdown();
        }
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
