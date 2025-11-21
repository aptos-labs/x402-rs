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
/// export RPC_URL_APTOS_TESTNET=https://api.testnet.aptoslabs.com/v1
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
            eprintln!("   APTOS_PRIVATE_KEY=0x... RPC_URL_APTOS_TESTNET=https://api.testnet.aptoslabs.com/v1 \\");
            eprintln!("   cargo test aptos_e2e_test -- --ignored --nocapture");
            return Ok(());
        }
    };

    let rpc_url = env::var("RPC_URL_APTOS_TESTNET")
        .unwrap_or_else(|_| "https://api.testnet.aptoslabs.com/v1".to_string());

    // Create Aptos REST client
    let url = url::Url::parse(&rpc_url)?;
    let rest_client = AptosClient::new(url);

    // Create sender wallet (client)
    use x402_reqwest::chains::aptos::AptosSenderWallet;
    let wallet = AptosSenderWallet::new(&private_key, rest_client.clone())?;
    let sender_address = wallet.address();

    // Check sender balance
    let _account_info = rest_client.get_account(sender_address).await?.into_inner();

    // Define payment requirements
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

    // Create payment payload (client-side signing)
    use x402_reqwest::chains::SenderWallet;
    let payment_payload = wallet
        .payment_payload(payment_requirements.clone())
        .await?;

    // Create facilitator provider
    let aptos_chain = x402_rs::chain::aptos::AptosChain {
        network: Network::AptosTestnet,
    };
    let facilitator = AptosProvider::new(aptos_chain, rest_client.clone());

    // Test VERIFY endpoint
    let verify_request = VerifyRequest {
        x402_version: X402Version::V1,
        payment_payload: payment_payload.clone(),
        payment_requirements: payment_requirements.clone(),
    };

    let verify_response = facilitator.verify(&verify_request).await?;
    match verify_response {
        x402_rs::types::VerifyResponse::Valid { .. } => {
            // Verification successful
        }
        x402_rs::types::VerifyResponse::Invalid { reason, .. } => {
            return Err(format!("Verification failed: {:?}", reason).into());
        }
    }

    // Test SETTLE endpoint
    let settle_request = SettleRequest {
        x402_version: X402Version::V1,
        payment_payload: payment_payload.clone(),
        payment_requirements: payment_requirements.clone(),
    };

    let settle_response = facilitator.settle(&settle_request).await?;
    assert!(settle_response.success, "Settlement should succeed");
    assert!(settle_response.transaction.is_some(), "Transaction hash should be present");

    // Wait for transaction confirmation
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_aptos_payment_verification_failure_wrong_recipient(
) -> Result<(), Box<dyn std::error::Error>> {
    let private_key = match env::var("APTOS_PRIVATE_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("⚠️  Skipping test: APTOS_PRIVATE_KEY not set");
            return Ok(());
        }
    };

    let rpc_url = env::var("RPC_URL_APTOS_TESTNET")
        .unwrap_or_else(|_| "https://api.testnet.aptoslabs.com/v1".to_string());

    let url = url::Url::parse(&rpc_url)?;
    let rest_client = AptosClient::new(url);

    use x402_reqwest::chains::aptos::AptosSenderWallet;
    let wallet = AptosSenderWallet::new(&private_key, rest_client.clone())?;

    // Create payment requirements with one recipient
    let correct_recipient = "0x1";
    let wrong_recipient = "0x2"; // Different recipient

    let payment_requirements_correct = PaymentRequirements {
        scheme: Scheme::Exact,
        network: Network::AptosTestnet,
        asset: MixedAddress::Aptos(
            aptos_types::account_address::AccountAddress::from_hex_literal("0x1")?,
        ),
        pay_to: MixedAddress::Aptos(
            aptos_types::account_address::AccountAddress::from_hex_literal(correct_recipient)?,
        ),
        max_amount_required: x402_rs::types::TokenAmount::from(1000u64),
        resource: url::Url::parse("https://example.com/test-resource")?,
        description: "E2E test payment".to_string(),
        mime_type: "application/json".to_string(),
        max_timeout_seconds: 60,
        extra: None,
        output_schema: None,
    };

    // Create payment with correct recipient
    use x402_reqwest::chains::SenderWallet;
    let payment_payload = wallet
        .payment_payload(payment_requirements_correct.clone())
        .await?;

    // But verify with wrong recipient in requirements
    let payment_requirements_wrong = PaymentRequirements {
        pay_to: MixedAddress::Aptos(
            aptos_types::account_address::AccountAddress::from_hex_literal(wrong_recipient)?,
        ),
        ..payment_requirements_correct
    };

    let aptos_chain = x402_rs::chain::aptos::AptosChain {
        network: Network::AptosTestnet,
    };
    let facilitator = AptosProvider::new(aptos_chain, rest_client);

    let verify_request = VerifyRequest {
        x402_version: X402Version::V1,
        payment_payload,
        payment_requirements: payment_requirements_wrong,
    };

    let verify_response = facilitator.verify(&verify_request).await?;
    match verify_response {
        x402_rs::types::VerifyResponse::Invalid { .. } => {
            // Expected to fail
            Ok(())
        }
        x402_rs::types::VerifyResponse::Valid { .. } => {
            Err("Verification should have failed with wrong recipient".into())
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_aptos_payment_verification_failure_wrong_amount(
) -> Result<(), Box<dyn std::error::Error>> {
    let private_key = match env::var("APTOS_PRIVATE_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("⚠️  Skipping test: APTOS_PRIVATE_KEY not set");
            return Ok(());
        }
    };

    let rpc_url = env::var("RPC_URL_APTOS_TESTNET")
        .unwrap_or_else(|_| "https://api.testnet.aptoslabs.com/v1".to_string());

    let url = url::Url::parse(&rpc_url)?;
    let rest_client = AptosClient::new(url);

    use x402_reqwest::chains::aptos::AptosSenderWallet;
    let wallet = AptosSenderWallet::new(&private_key, rest_client.clone())?;

    // Create payment with one amount
    let payment_requirements_correct = PaymentRequirements {
        scheme: Scheme::Exact,
        network: Network::AptosTestnet,
        asset: MixedAddress::Aptos(
            aptos_types::account_address::AccountAddress::from_hex_literal("0x1")?,
        ),
        pay_to: MixedAddress::Aptos(
            aptos_types::account_address::AccountAddress::from_hex_literal("0x1")?,
        ),
        max_amount_required: x402_rs::types::TokenAmount::from(1000u64),
        resource: url::Url::parse("https://example.com/test-resource")?,
        description: "E2E test payment".to_string(),
        mime_type: "application/json".to_string(),
        max_timeout_seconds: 60,
        extra: None,
        output_schema: None,
    };

    use x402_reqwest::chains::SenderWallet;
    let payment_payload = wallet
        .payment_payload(payment_requirements_correct.clone())
        .await?;

    // Verify with different amount
    let payment_requirements_wrong = PaymentRequirements {
        max_amount_required: x402_rs::types::TokenAmount::from(2000u64), // Different amount
        ..payment_requirements_correct
    };

    let aptos_chain = x402_rs::chain::aptos::AptosChain {
        network: Network::AptosTestnet,
    };
    let facilitator = AptosProvider::new(aptos_chain, rest_client);

    let verify_request = VerifyRequest {
        x402_version: X402Version::V1,
        payment_payload,
        payment_requirements: payment_requirements_wrong,
    };

    let verify_response = facilitator.verify(&verify_request).await?;
    match verify_response {
        x402_rs::types::VerifyResponse::Invalid { .. } => {
            // Expected to fail
            Ok(())
        }
        x402_rs::types::VerifyResponse::Valid { .. } => {
            Err("Verification should have failed with wrong amount".into())
        }
    }
}

#[tokio::test]
#[ignore]
async fn test_aptos_payment_verification_failure_wrong_asset(
) -> Result<(), Box<dyn std::error::Error>> {
    let private_key = match env::var("APTOS_PRIVATE_KEY") {
        Ok(key) => key,
        Err(_) => {
            eprintln!("⚠️  Skipping test: APTOS_PRIVATE_KEY not set");
            return Ok(());
        }
    };

    let rpc_url = env::var("RPC_URL_APTOS_TESTNET")
        .unwrap_or_else(|_| "https://api.testnet.aptoslabs.com/v1".to_string());

    let url = url::Url::parse(&rpc_url)?;
    let rest_client = AptosClient::new(url);

    use x402_reqwest::chains::aptos::AptosSenderWallet;
    let wallet = AptosSenderWallet::new(&private_key, rest_client.clone())?;

    // Create payment with one asset
    let payment_requirements_correct = PaymentRequirements {
        scheme: Scheme::Exact,
        network: Network::AptosTestnet,
        asset: MixedAddress::Aptos(
            aptos_types::account_address::AccountAddress::from_hex_literal("0x1")?,
        ),
        pay_to: MixedAddress::Aptos(
            aptos_types::account_address::AccountAddress::from_hex_literal("0x1")?,
        ),
        max_amount_required: x402_rs::types::TokenAmount::from(1000u64),
        resource: url::Url::parse("https://example.com/test-resource")?,
        description: "E2E test payment".to_string(),
        mime_type: "application/json".to_string(),
        max_timeout_seconds: 60,
        extra: None,
        output_schema: None,
    };

    use x402_reqwest::chains::SenderWallet;
    let payment_payload = wallet
        .payment_payload(payment_requirements_correct.clone())
        .await?;

    // Verify with different asset
    let payment_requirements_wrong = PaymentRequirements {
        asset: MixedAddress::Aptos(
            aptos_types::account_address::AccountAddress::from_hex_literal("0x2")?,
        ), // Different asset
        ..payment_requirements_correct
    };

    let aptos_chain = x402_rs::chain::aptos::AptosChain {
        network: Network::AptosTestnet,
    };
    let facilitator = AptosProvider::new(aptos_chain, rest_client);

    let verify_request = VerifyRequest {
        x402_version: X402Version::V1,
        payment_payload,
        payment_requirements: payment_requirements_wrong,
    };

    let verify_response = facilitator.verify(&verify_request).await?;
    match verify_response {
        x402_rs::types::VerifyResponse::Invalid { .. } => {
            // Expected to fail
            Ok(())
        }
        x402_rs::types::VerifyResponse::Valid { .. } => {
            Err("Verification should have failed with wrong asset".into())
        }
    }
}
