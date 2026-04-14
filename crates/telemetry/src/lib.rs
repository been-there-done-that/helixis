use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

pub fn init_logging() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info".to_string()));

    let telemetry_layer = tracing_opentelemetry::layer();

    tracing_subscriber::registry()
        .with(filter)
        .with(telemetry_layer)
        .init();
}

pub fn init_logging_noop() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info".to_string()));

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_line_number(false);

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
}

pub fn shutdown() {}

#[macro_export]
macro_rules! instrument {
    ($name:expr) => {
        tracing::info_span!($name)
    };
    ($name:expr, $($key:expr => $value:expr),*) => {
        tracing::info_span!($name, $($key => $value),*)
    };
}
