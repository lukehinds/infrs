use anyhow::Result;
use axum::Router;
use clap::Parser;
use std::{sync::Arc, net::SocketAddr};
use tokio::sync::Mutex;
use tower_http::trace::TraceLayer;

mod api;
mod config;
mod models;
mod providers;

use models::load_model;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to config file
    #[arg(short, long, default_value = "config.json")]
    config: String,

    /// Model ID to use (overrides config file)
    #[arg(long)]
    model: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with more production-ready settings
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
        )
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_target(false)
        .with_thread_names(true)
        .with_ansi(true)
        .init();

    // Create a span for the main function
    let _guard = tracing::info_span!("main").entered();
    
    // Parse command line arguments
    let args = Args::parse();
    tracing::info!("Loading config from: {}", args.config);

    // Load configuration
    let mut config = config::Config::from_file(&args.config)?;
    
    // Override model if specified in CLI args
    if let Some(model_id) = args.model {
        config.model.model_id = model_id;
    }
    
    tracing::info!("Config loaded: {:?}", config);

    // Need to make device configurable or autosensing so that we also support
    // CPU, MPS, and CUDA devices.
    let device = candle_core::Device::new_metal(0)?;
    // let device = candle_core::Device::Cpu;  // Use CPU device
    let default_dtype = models::default_dtype();
    tracing::info!("Using default dtype: {:?} (may be overridden by model's config.json)", default_dtype);
    
    tracing::info!("Loading model: {}", config.model.model_id);
    // Determine model type from model ID
    let model = if config.model.model_id.contains("Qwen") {
        models::ModelWrapper::Qwen(
            load_model::<models::qwen::QwenWithConfig>(
                &config.model.model_id,
                &config.model.revision,
                default_dtype,
                &device,
            ).await?
        )
    } else if config.model.model_id.contains("Mistral") {
        tracing::info!("Loading Mistral model");
        models::ModelWrapper::Mistral(
            load_model::<models::mistral::MistralWithConfig>(
                &config.model.model_id,
                &config.model.revision,
                default_dtype,
                &device,
            ).await?
        )
    } else {
        // Default to Llama for other models
        tracing::info!("Loading Llama model");
        models::ModelWrapper::Llama(
            load_model::<models::llama::LlamaWithConfig>(
                &config.model.model_id,
                &config.model.revision,
                default_dtype,
                &device,
            ).await?
        )
    };
    tracing::info!("Model loaded successfully");

    // Create shared model state
    let model = Arc::new(Mutex::new(model));

    // Build the API router
    let app = Router::new()
        .merge(api::routes(model.clone()))
        .layer(TraceLayer::new_for_http());

    // Start the server
    let addr = format!("{}:{}", config.server.host, config.server.port)
        .parse::<SocketAddr>()
        .expect("Invalid address");

    tracing::info!("Starting server on {}", addr);
    axum::serve(
        tokio::net::TcpListener::bind(&addr).await?,
        app.into_make_service(),
    )
    .await?;

    Ok(())
}
