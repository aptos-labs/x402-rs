mod types;

use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

use crate::chain::aptos::AptosChainProvider;
use crate::chain::{ChainProvider, ChainProviderOps};
use crate::proto;
use crate::proto::PaymentVerificationError;
use crate::scheme::v1_eip155_exact::EXACT_SCHEME;
use crate::scheme::{SchemeSlug, X402SchemeBlueprint, X402SchemeHandler, X402SchemeHandlerError};
use aptos_types::account_address::AccountAddress;
use aptos_types::transaction::{EntryFunction, RawTransaction, SignedTransaction, TransactionPayload};
use aptos_types::transaction::authenticator::{AccountAuthenticator, TransactionAuthenticator};
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::ModuleId;
use base64::Engine;

pub struct V2AptosExact;

impl X402SchemeBlueprint for V2AptosExact {
    fn slug(&self) -> SchemeSlug {
        SchemeSlug::new(2, "aptos", EXACT_SCHEME.to_string())
    }

    fn build(
        &self,
        provider: ChainProvider,
        _config: Option<serde_json::Value>,
    ) -> Result<Box<dyn X402SchemeHandler>, Box<dyn Error>> {
        let provider = if let ChainProvider::Aptos(provider) = provider {
            provider
        } else {
            return Err("V2AptosExact::build: provider must be an AptosChainProvider".into());
        };
        Ok(Box::new(V2AptosExactHandler { provider }))
    }
}

pub struct V2AptosExactHandler {
    provider: Arc<AptosChainProvider>,
}

#[async_trait::async_trait]
impl X402SchemeHandler for V2AptosExactHandler {
    async fn verify(
        &self,
        request: &proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, X402SchemeHandlerError> {
        let request = types::VerifyRequest::from_proto(request.clone())?;
        let verification = verify_transfer(&self.provider, &request).await?;
        Ok(proto::v2::VerifyResponse::valid(verification.payer.to_string()).into())
    }

    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, X402SchemeHandlerError> {
        let request = types::SettleRequest::from_proto(request.clone())?;
        let verification = verify_transfer(&self.provider, &request).await?;
        let payer = verification.payer.to_string();
        let tx_hash = settle_transaction(&self.provider, verification).await?;
        Ok(proto::v2::SettleResponse::Success {
            payer,
            transaction: format!("0x{}", hex::encode(tx_hash)),
            network: self.provider.chain_id().to_string(),
        }
        .into())
    }

    async fn supported(&self) -> Result<proto::SupportedResponse, X402SchemeHandlerError> {
        let chain_id = self.provider.chain_id();
        let kinds: Vec<proto::SupportedPaymentKind> = vec![proto::SupportedPaymentKind {
            x402_version: proto::v2::X402Version2.into(),
            scheme: EXACT_SCHEME.to_string(),
            network: chain_id.to_string(),
            extra: None,
        }];
        let signers = {
            let mut signers = HashMap::with_capacity(1);
            signers.insert(chain_id, self.provider.signer_addresses());
            signers
        };
        Ok(proto::SupportedResponse {
            kinds,
            extensions: Vec::new(),
            signers,
        })
    }
}

pub struct VerifyTransferResult {
    pub payer: AccountAddress,
    pub raw_transaction: RawTransaction,
    pub authenticator_bytes: Vec<u8>,
}

pub async fn verify_transfer(
    provider: &AptosChainProvider,
    request: &types::VerifyRequest,
) -> Result<VerifyTransferResult, PaymentVerificationError> {
    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    // Validate accepted == requirements
    let accepted = &payload.accepted;
    if accepted != requirements {
        return Err(PaymentVerificationError::AcceptedRequirementsMismatch);
    }

    // Validate chain ID
    let chain_id = provider.chain_id();
    let payload_chain_id = &accepted.network;
    if payload_chain_id != &chain_id {
        return Err(PaymentVerificationError::UnsupportedChain);
    }

    // Deserialize transaction
    let transaction_b64 = &payload.payload.transaction;
    let (raw_transaction, authenticator_bytes, entry_function) =
        deserialize_aptos_transaction(transaction_b64)?;

    // Extract sender (payer)
    let payer = raw_transaction.sender();

    // Validate entry function is primary_fungible_store::transfer
    let expected_module = ModuleId::new(
        AccountAddress::ONE,
        Identifier::new("primary_fungible_store")
            .map_err(|e| PaymentVerificationError::InvalidFormat(format!("Invalid module identifier: {}", e)))?,
    );
    let expected_function = Identifier::new("transfer")
        .map_err(|e| PaymentVerificationError::InvalidFormat(format!("Invalid function identifier: {}", e)))?;

    if entry_function.module() != &expected_module {
        return Err(PaymentVerificationError::InvalidFormat(format!(
            "Invalid module: expected {}, got {}",
            expected_module,
            entry_function.module()
        )));
    }

    if *entry_function.function() != *expected_function {
        return Err(PaymentVerificationError::InvalidFormat(format!(
            "Invalid function: expected {}, got {}",
            expected_function,
            entry_function.function()
        )));
    }

    // Validate 3 arguments
    let args = entry_function.args();
    if args.len() != 3 {
        return Err(PaymentVerificationError::InvalidFormat(format!(
            "Expected 3 arguments, got {}",
            args.len()
        )));
    }

    // Parse arguments
    // Arg 0: Fungible asset address
    let fa_address: AccountAddress = bcs::from_bytes(&args[0])
        .map_err(|e| PaymentVerificationError::InvalidFormat(format!("Failed to parse FA address: {}", e)))?;

    // Arg 1: Recipient address
    let recipient: AccountAddress = bcs::from_bytes(&args[1])
        .map_err(|e| PaymentVerificationError::InvalidFormat(format!("Failed to parse recipient: {}", e)))?;

    // Arg 2: Amount
    let amount: u64 = bcs::from_bytes(&args[2])
        .map_err(|e| PaymentVerificationError::InvalidFormat(format!("Failed to parse amount: {}", e)))?;

    // Validate asset matches
    let expected_asset: AccountAddress = (*requirements.asset.inner()).clone();
    if fa_address != expected_asset {
        return Err(PaymentVerificationError::InvalidFormat(format!(
            "Asset mismatch: expected {}, got {}",
            expected_asset, fa_address
        )));
    }

    // Validate recipient matches
    let expected_recipient: AccountAddress = (*requirements.pay_to.inner()).clone();
    if recipient != expected_recipient {
        return Err(PaymentVerificationError::RecipientMismatch);
    }

    // Validate amount matches
    let expected_amount = requirements.amount.inner();
    if amount != expected_amount {
        return Err(PaymentVerificationError::InvalidPaymentAmount);
    }

    Ok(VerifyTransferResult {
        payer,
        raw_transaction,
        authenticator_bytes,
    })
}

pub async fn settle_transaction(
    provider: &AptosChainProvider,
    verification: VerifyTransferResult,
) -> Result<[u8; 32], PaymentVerificationError> {
    use aptos_crypto::hash::CryptoHash;

    // Compute transaction hash
    let tx_hash = verification.raw_transaction.hash();
    let tx_hash_bytes: [u8; 32] = tx_hash.to_vec().try_into()
        .map_err(|_| PaymentVerificationError::InvalidFormat("Invalid transaction hash".to_string()))?;

    // Deserialize authenticator
    let account_authenticator: AccountAuthenticator =
        bcs::from_bytes(&verification.authenticator_bytes)
            .map_err(|e| PaymentVerificationError::InvalidFormat(format!("Failed to deserialize authenticator: {}", e)))?;

    // Wrap in TransactionAuthenticator
    let tx_authenticator = TransactionAuthenticator::Ed25519 {
        public_key: match &account_authenticator {
            AccountAuthenticator::Ed25519 { public_key, signature: _ } => public_key.clone(),
            _ => return Err(PaymentVerificationError::InvalidSignature("Only Ed25519 authenticator supported".to_string())),
        },
        signature: match &account_authenticator {
            AccountAuthenticator::Ed25519 { public_key: _, signature } => signature.clone(),
            _ => return Err(PaymentVerificationError::InvalidSignature("Only Ed25519 authenticator supported".to_string())),
        },
    };

    // Create signed transaction
    let signed_txn = SignedTransaction::new_signed_transaction(
        verification.raw_transaction.clone(),
        tx_authenticator,
    );

    // Submit transaction
    provider
        .rest_client()
        .submit_bcs(&signed_txn)
        .await
        .map_err(|e| PaymentVerificationError::TransactionSimulation(format!("Transaction submission failed: {}", e)))?;

    Ok(tx_hash_bytes)
}

/// Deserialize Aptos transaction from base64-encoded JSON
fn deserialize_aptos_transaction(
    transaction_b64: &str,
) -> Result<(RawTransaction, Vec<u8>, EntryFunction), PaymentVerificationError> {
    // Base64 decode
    let json_bytes = base64::engine::general_purpose::STANDARD
        .decode(transaction_b64)
        .map_err(|e| PaymentVerificationError::InvalidFormat(format!("Base64 decode failed: {}", e)))?;

    // Parse JSON
    let json_payload: serde_json::Value = serde_json::from_slice(&json_bytes)
        .map_err(|e| PaymentVerificationError::InvalidFormat(format!("JSON parse failed: {}", e)))?;

    // Extract transaction and authenticator byte arrays
    let transaction_bytes = json_payload
        .get("transaction")
        .and_then(|v| v.as_array())
        .ok_or_else(|| PaymentVerificationError::InvalidFormat("Missing transaction field".to_string()))?;

    let authenticator_bytes = json_payload
        .get("senderAuthenticator")
        .and_then(|v| v.as_array())
        .ok_or_else(|| PaymentVerificationError::InvalidFormat("Missing senderAuthenticator field".to_string()))?;

    // Convert to Vec<u8>
    let transaction_bytes: Vec<u8> = transaction_bytes
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();

    let authenticator_bytes: Vec<u8> = authenticator_bytes
        .iter()
        .filter_map(|v| v.as_u64().map(|n| n as u8))
        .collect();

    // BCS deserialize transaction
    let raw_transaction: RawTransaction = bcs::from_bytes(&transaction_bytes)
        .map_err(|e| PaymentVerificationError::InvalidFormat(format!("BCS deserialize transaction failed: {}", e)))?;

    // Extract entry function from payload - clone the transaction to avoid consuming it
    let payload = raw_transaction.clone().into_payload();
    let entry_function = match payload {
        TransactionPayload::EntryFunction(ef) => ef,
        _ => {
            return Err(PaymentVerificationError::InvalidFormat(
                "Transaction payload is not an entry function".to_string(),
            ))
        }
    };

    Ok((raw_transaction, authenticator_bytes, entry_function))
}
