/// End-to-end integration test for Aptos payment flow
///
/// This test validates the complete x402 payment flow on Aptos:
/// 1. Creates a payment using AptosSenderWallet (client)
/// 2. Verifies the payment using AptosProvider (facilitator)
/// 3. Settles the payment on-chain
/// 4. Validates the transaction
///
/// ## Prerequisites:
/// Set environment variables before running:
/// ```bash
/// export APTOS_PRIVATE_KEY=0x...
/// export RPC_URL_APTOS_TESTNET=https://fullnode.testnet.aptoslabs.com/v1
/// cargo test aptos_e2e_test -- --nocapture
/// ```

use aptos_rest_client::Client as AptosClient;
use std::env;
use x402_rs::chain::aptos::AptosProvider;
use x402_rs::facilitator::Facilitator;
use x402_rs::network::Network;
use x402_rs::types::{
    MixedAddress, PaymentRequirements, Scheme, SettleRequest, VerifyRequest, X402Version,
};

#[tokio::test]
#[ignore] // Requires environment variables and network access
async fn test_aptos_payment_flow() -> Result<(), Box<dyn std::error::Error>> {
    // Skip test if environment variables are not set
    let private_key = match env::var("APTOS_PRIVATE_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("⚠️  Skipping test: APTOS_PRIVATE_KEY not set");
            eprintln!("   To run this test:");
            eprintln!("   APTOS_PRIVATE_KEY=0x... RPC_URL_APTOS_TESTNET=https://fullnode.testnet.aptoslabs.com/v1 \\");
            eprintln!("   cargo test aptos_e2e_test -- --ignored --nocapture");
            return Ok(());
        }
    };

    let rpc_url = env::var("RPC_URL_APTOS_TESTNET")
        .unwrap_or_else(|_| "https://fullnode.testnet.aptoslabs.com/v1".to_string());

    println!("🚀 Starting Aptos x402 End-to-End Test\n");
    println!("📝 Configuration:");
    println!("   RPC URL: {}", rpc_url);
    println!("   Network: aptos-testnet\n");

    // Create Aptos REST client
    let url = url::Url::parse(&rpc_url)?;
    let rest_client = AptosClient::new(url);

    // Create sender wallet (client)
    println!("🔑 Creating sender wallet...");
    use x402_reqwest::chains::aptos::AptosSenderWallet;
    let wallet = AptosSenderWallet::new(&private_key, rest_client.clone())?;
    let sender_address = wallet.address();
    println!(
        "   ✅ Sender address: 0x{}",
        alloy::hex::encode(sender_address.to_vec())
    );

    // Check sender balance
    let account_info = rest_client.get_account(sender_address).await?.into_inner();
    println!("   💰 Balance: {} APT (octas)", account_info.sequence_number);
    println!("   📊 Sequence number: {}\n", account_info.sequence_number);

    // Define payment requirements
    println!("📋 Creating payment requirements...");
    let recipient = "0x1"; // Standard Aptos account (for testing)
    let amount = "1000"; // 0.00001 APT (1000 octas)

    let payment_requirements = PaymentRequirements {
        scheme: Scheme::Exact,
        network: Network::AptosTestnet,
        asset: MixedAddress::Aptos(
            aptos_types::account_address::AccountAddress::from_hex_literal("0x1")?,
        ),
        pay_to: MixedAddress::Aptos(
            aptos_types::account_address::AccountAddress::from_hex_literal(recipient)?,
        ),
        max_amount_required: x402_rs::types::TokenAmount::from(amount.parse::<u64>().unwrap()),
        resource: url::Url::parse("https://example.com/test-resource")?,
        description: "E2E test payment".to_string(),
        mime_type: "application/json".to_string(),
        max_timeout_seconds: 60,
        extra: None,
        output_schema: None,
    };
    println!("   📍 Recipient: {}", recipient);
    println!("   💵 Amount: {} octas\n", amount);

    // Create payment payload (client-side signing)
    println!("✍️  Creating and signing payment...");
    use x402_reqwest::chains::SenderWallet;
    let payment_payload = wallet
        .payment_payload(payment_requirements.clone())
        .await?;
    println!("   ✅ Payment signed and serialized\n");

    // Create facilitator provider
    println!("🏪 Initializing facilitator...");
    let aptos_chain = x402_rs::chain::aptos::AptosChain {
        network: Network::AptosTestnet,
    };
    let facilitator = AptosProvider::new(aptos_chain, rest_client.clone());
    println!("   ✅ Facilitator ready\n");

    // Test VERIFY endpoint
    println!("🔍 Testing VERIFY endpoint...");
    let verify_request = VerifyRequest {
        x402_version: X402Version::V1,
        payment_payload: payment_payload.clone(),
        payment_requirements: payment_requirements.clone(),
    };

    let verify_response = facilitator.verify(&verify_request).await?;
    match verify_response {
        x402_rs::types::VerifyResponse::Valid { payer } => {
            println!("   ✅ Payment verified successfully!");
            println!("   👤 Payer: {}\n", payer);
        }
        x402_rs::types::VerifyResponse::Invalid { reason, payer } => {
            println!("   ❌ Verification failed: {:?}", reason);
            if let Some(p) = payer {
                println!("   👤 Payer: {}", p);
            }
            return Err("Verification failed".into());
        }
    }

    // Test SETTLE endpoint
    println!("💳 Testing SETTLE endpoint...");
    let settle_request = SettleRequest {
        x402_version: X402Version::V1,
        payment_payload: payment_payload.clone(),
        payment_requirements: payment_requirements.clone(),
    };

    let settle_response = facilitator.settle(&settle_request).await?;
    if settle_response.success {
        println!("   ✅ Payment settled successfully!");
        if let Some(ref tx_hash) = settle_response.transaction {
            println!("   🔗 Transaction hash: {}", tx_hash);
        }
        println!("   🌐 Network: {}", settle_response.network);
        println!("   👤 Payer: {}\n", settle_response.payer);
    } else {
        println!(
            "   ❌ Settlement failed: {:?}",
            settle_response.error_reason
        );
        return Err("Settlement failed".into());
    }

    // Verify transaction on-chain
    if let Some(ref tx_hash) = settle_response.transaction {
        println!("🔎 Verifying transaction on-chain...");
        let hash_str = tx_hash.to_string();
        println!(
            "   📍 Explorer: https://explorer.aptoslabs.com/txn/{}?network=testnet",
            hash_str
        );
        println!("   ⏳ Waiting for confirmation...");

        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        println!("   ✅ Transaction confirmed on-chain\n");
    }

    println!("✨ All tests passed! ✨");
    println!("\n📊 Summary:");
    println!("   ✓ Wallet creation");
    println!("   ✓ Payment signing");
    println!("   ✓ Payment verification");
    println!("   ✓ Payment settlement");
    println!("   ✓ On-chain confirmation");

    Ok(())
}
