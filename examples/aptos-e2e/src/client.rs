use anyhow::Result;
use aptos_rest_client::Client as AptosClient;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use x402_reqwest::chains::aptos::AptosSenderWallet;
use x402_reqwest::{ReqwestWithPayments, ReqwestWithPaymentsBuild};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "aptos_e2e=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    // Load environment variables
    dotenvy::dotenv().ok();

    // Get Aptos private key from environment
    let private_key = std::env::var("APTOS_PRIVATE_KEY")
        .expect("APTOS_PRIVATE_KEY must be set");

    // Get RPC URL (default to testnet)
    let rpc_url = std::env::var("APTOS_RPC_URL")
        .unwrap_or_else(|_| "https://api.testnet.aptoslabs.com/v1".to_string());

    tracing::info!("Using Aptos RPC: {}", rpc_url);

    // Create Aptos REST client
    let url = url::Url::parse(&rpc_url)?;
    let rest_client = AptosClient::new(url);

    // Create Aptos sender wallet
    let wallet = AptosSenderWallet::new(&private_key, rest_client)?;
    tracing::info!("Wallet address: 0x{}", hex::encode(wallet.address().to_vec()));

    // Create HTTP client with x402 payments
    let client = reqwest::Client::new()
        .with_payments(wallet)
        .build();

    // Make request to protected endpoint
    tracing::info!("Making request to http://127.0.0.1:3000/protected");
    let response = client
        .get("http://127.0.0.1:3000/protected")
        .send()
        .await?;

    tracing::info!("Response status: {}", response.status());

    let body = response.text().await?;
    tracing::info!("Response body: {}", body);

    Ok(())
}
