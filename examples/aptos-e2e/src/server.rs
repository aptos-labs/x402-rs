use anyhow::Result;
use axum::{routing::get, Router};
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use x402_axum::X402PaymentRequired;
use x402_rs::chain::aptos::Address as AptosAddress;
use x402_rs::chain::ChainId;
use x402_rs::config::Config;
use x402_rs::proto::util::U64String;
use x402_rs::proto::v2::PaymentRequirements;
use x402_rs::scheme::v1_eip155_exact::types::ExactScheme;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "aptos_e2e=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    // Load config
    let config = Config::load()?;

    // Build x402 payment middleware
    let payment_middleware = x402_axum::build_payment_middleware(config)?;

    // Build router
    let app = Router::new()
        .route("/", get(root))
        .route("/protected", get(protected))
        .layer(payment_middleware)
        .layer(TraceLayer::new_for_http());

    // Start server
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn root() -> &'static str {
    "Hello from Aptos x402 server!"
}

async fn protected() -> Result<String, X402PaymentRequired<ExactScheme, U64String, AptosAddress, Option<()>>> {
    // Require payment: 0.01 USDC on Aptos testnet
    // USDC testnet: 0x69091fbab5f7d635ee7ac5098cf0c1efbe31d68fec0f2cd565e8d168daf52832
    let usdc_testnet = "0x69091fbab5f7d635ee7ac5098cf0c1efbe31d68fec0f2cd565e8d168daf52832"
        .parse::<AptosAddress>()
        .map_err(|e| {
            X402PaymentRequired::new(vec![]).with_error(format!("Invalid USDC address: {}", e))
        })?;

    let pay_to_address = "0x1"
        .parse::<AptosAddress>()
        .map_err(|e| {
            X402PaymentRequired::new(vec![]).with_error(format!("Invalid pay_to address: {}", e))
        })?;

    let requirements = PaymentRequirements {
        scheme: ExactScheme,
        amount: U64String::from(10000u64), // 0.01 USDC (6 decimals)
        asset: usdc_testnet,
        pay_to: pay_to_address,
        network: ChainId::new("aptos", "2"), // Aptos testnet
        extra: None,
    };

    Err(X402PaymentRequired::new(vec![requirements]))
}
