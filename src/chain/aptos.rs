use aptos_rest_client::Client as AptosClient;
use aptos_types::account_address::AccountAddress;
use aptos_types::transaction::{EntryFunction, RawTransaction, TransactionPayload};
use base64::Engine;
use bcs::from_bytes;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use crate::chain::{FacilitatorLocalError, FromEnvByNetworkBuild, NetworkProviderOps};
use crate::facilitator::Facilitator;
use crate::network::Network;
use crate::types::{
    AptosPayloadJson, ExactAptosPayload, ExactPaymentPayload, FacilitatorErrorReason, MixedAddress, SettleRequest,
    SettleResponse, SupportedPaymentKind, SupportedPaymentKindsResponse, TransactionHash,
    VerifyRequest, VerifyResponse,
};
use crate::types::{Scheme, X402Version};

#[derive(Clone, Debug)]
pub struct AptosChain {
    pub network: Network,
}

impl TryFrom<Network> for AptosChain {
    type Error = FacilitatorLocalError;

    fn try_from(value: Network) -> Result<Self, Self::Error> {
        match value {
            Network::Aptos => Ok(Self { network: value }),
            Network::AptosTestnet => Ok(Self { network: value }),
            _ => Err(FacilitatorLocalError::UnsupportedNetwork(None)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AptosAddress {
    account_address: AccountAddress,
}

impl From<AccountAddress> for AptosAddress {
    fn from(account_address: AccountAddress) -> Self {
        Self { account_address }
    }
}

impl From<AptosAddress> for AccountAddress {
    fn from(address: AptosAddress) -> Self {
        address.account_address
    }
}

impl TryFrom<MixedAddress> for AptosAddress {
    type Error = FacilitatorLocalError;

    fn try_from(value: MixedAddress) -> Result<Self, Self::Error> {
        match value {
            MixedAddress::Evm(_) => Err(FacilitatorLocalError::InvalidAddress(
                "expected Aptos address".to_string(),
            )),
            MixedAddress::Offchain(_) => Err(FacilitatorLocalError::InvalidAddress(
                "expected Aptos address".to_string(),
            )),
            MixedAddress::Solana(_) => Err(FacilitatorLocalError::InvalidAddress(
                "expected Aptos address".to_string(),
            )),
            MixedAddress::Aptos(account_address) => Ok(Self { account_address }),
        }
    }
}

impl From<AptosAddress> for MixedAddress {
    fn from(value: AptosAddress) -> Self {
        MixedAddress::Aptos(value.account_address)
    }
}

#[derive(Clone)]
pub struct AptosProvider {
    chain: AptosChain,
    rest_client: Arc<AptosClient>,
}

impl Debug for AptosProvider {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AptosProvider")
            .field("chain", &self.chain)
            .field("rest_client", &"<rest_client>")
            .finish()
    }
}

impl AptosProvider {
    pub fn new(chain: AptosChain, rest_client: AptosClient) -> Self {
        Self {
            chain,
            rest_client: Arc::new(rest_client),
        }
    }
}

impl FromEnvByNetworkBuild for AptosProvider {
    async fn from_env(
        network: Network,
    ) -> Result<Option<Self>, Box<dyn std::error::Error>> {
        let chain = AptosChain::try_from(network)?;

        // Get RPC URL from environment
        let rpc_url_var = format!("RPC_URL_{}", network.to_string().to_uppercase().replace('-', "_"));
        let rpc_url = match std::env::var(&rpc_url_var) {
            Ok(url) => url,
            Err(_) => {
                tracing::debug!("No RPC URL found for {}: {}", network, rpc_url_var);
                return Ok(None);
            }
        };

        // Create Aptos REST client
        let url = url::Url::parse(&rpc_url)?;
        let rest_client = AptosClient::new(url);

        Ok(Some(Self::new(chain, rest_client)))
    }
}

impl NetworkProviderOps for AptosProvider {
    fn signer_address(&self) -> MixedAddress {
        // Aptos facilitator doesn't need a signer address for verification/settlement
        MixedAddress::Aptos(AccountAddress::ZERO)
    }

    fn network(&self) -> Network {
        self.chain.network
    }
}

/// Deserialize the Aptos transaction from the base64-encoded JSON payload
fn deserialize_aptos_transaction(
    payload: &ExactAptosPayload,
) -> Result<(RawTransaction, aptos_types::transaction::authenticator::AccountAuthenticator, EntryFunction), FacilitatorLocalError> {
    // Decode base64
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&payload.transaction)
        .map_err(|e| {
            FacilitatorLocalError::DecodingError(format!("Failed to decode base64: {}", e))
        })?;

    // Parse JSON
    let json_str = String::from_utf8(decoded).map_err(|e| {
        FacilitatorLocalError::DecodingError(format!("Failed to decode UTF-8: {}", e))
    })?;

    let aptos_payload: AptosPayloadJson = serde_json::from_str(&json_str).map_err(|e| {
        FacilitatorLocalError::DecodingError(format!("Failed to parse JSON: {}", e))
    })?;

    // Deserialize transaction using BCS
    let raw_txn: RawTransaction = from_bytes(&aptos_payload.transaction).map_err(|e| {
        FacilitatorLocalError::DecodingError(format!("Failed to deserialize transaction: {}", e))
    })?;

    // Deserialize authenticator using BCS
    let authenticator: aptos_types::transaction::authenticator::AccountAuthenticator =
        from_bytes(&aptos_payload.sender_authenticator).map_err(|e| {
            FacilitatorLocalError::DecodingError(format!("Failed to deserialize authenticator: {}", e))
        })?;

    // Extract entry function from payload
    let entry_function = match raw_txn.clone().into_payload() {
        TransactionPayload::EntryFunction(ef) => ef,
        TransactionPayload::Script(_) => {
            return Err(FacilitatorLocalError::DecodingError(
                "Script payloads are not supported".to_string(),
            ));
        }
        TransactionPayload::Multisig(_) => {
            return Err(FacilitatorLocalError::DecodingError(
                "Multisig payloads are not supported".to_string(),
            ));
        }
        _ => {
            return Err(FacilitatorLocalError::DecodingError(
                "Unsupported transaction payload type".to_string(),
            ));
        }
    };

    Ok((raw_txn, authenticator, entry_function))
}

impl Facilitator for AptosProvider {
    type Error = FacilitatorLocalError;

    async fn verify(&self, request: &VerifyRequest) -> Result<VerifyResponse, Self::Error> {
        tracing::info!(
            "Verifying Aptos payment for network: {}",
            self.chain.network
        );

        // Extract Aptos payload
        let aptos_payload = match &request.payment_payload.payload {
            ExactPaymentPayload::Aptos(payload) => payload,
            _ => {
                return Err(FacilitatorLocalError::DecodingError(
                    "Expected Aptos payload".to_string(),
                ))
            }
        };

        // Deserialize transaction
        let (raw_txn, _authenticator, entry_function) = deserialize_aptos_transaction(aptos_payload)?;

        let sender = raw_txn.sender();
        let sender_mixed = MixedAddress::Aptos(sender);

        // Verify the function is the correct transfer function
        // Expected: 0x1::primary_fungible_store::transfer
        let module = entry_function.module();
        let function_name = entry_function.function();

        tracing::debug!(
            "Entry function: {}::{}::{}",
            module.address(),
            module.name(),
            function_name
        );

        let is_fungible_transfer = module.address() == &AccountAddress::ONE
            && module.name().as_str() == "primary_fungible_store"
            && function_name.as_str() == "transfer";

        if !is_fungible_transfer {
            tracing::warn!(
                "Invalid function: {}::{}::{}, only primary_fungible_store::transfer is supported",
                module.address(),
                module.name(),
                function_name
            );
            return Ok(VerifyResponse::Invalid {
                reason: FacilitatorErrorReason::FreeForm("invalid_payment".to_string()),
                payer: Some(sender_mixed),
            });
        }

        // Extract and verify arguments for primary_fungible_store::transfer
        // Expected: (fa_address: Object<T>, recipient: address, amount: u64)
        let args = entry_function.args();

        if args.len() != 3 {
            tracing::warn!("Invalid arguments length for primary_fungible_store::transfer");
            return Ok(VerifyResponse::Invalid {
                reason: FacilitatorErrorReason::FreeForm("invalid_payment".to_string()),
                payer: Some(sender_mixed),
            });
        }

        // Parse FA address
        let fa_address: AccountAddress = bcs::from_bytes(&args[0]).map_err(|e| {
            FacilitatorLocalError::DecodingError(format!("Failed to parse FA address: {}", e))
        })?;

        // Parse recipient address
        let recipient: AccountAddress = bcs::from_bytes(&args[1]).map_err(|e| {
            FacilitatorLocalError::DecodingError(format!("Failed to parse recipient: {}", e))
        })?;

        // Parse amount
        let amount: u64 = bcs::from_bytes(&args[2]).map_err(|e| {
            FacilitatorLocalError::DecodingError(format!("Failed to parse amount: {}", e))
        })?;

        // Verify FA address matches requirements
        let expected_asset: AccountAddress =
            AptosAddress::try_from(request.payment_requirements.asset.clone())?.into();

        if fa_address != expected_asset {
            tracing::warn!(
                "Asset mismatch: got {}, expected {}",
                fa_address,
                expected_asset
            );
            return Ok(VerifyResponse::Invalid {
                reason: FacilitatorErrorReason::FreeForm("invalid_payment".to_string()),
                payer: Some(sender_mixed),
            });
        }

        // Verify recipient matches requirements
        let expected_recipient: AccountAddress =
            AptosAddress::try_from(request.payment_requirements.pay_to.clone())?.into();

        if recipient != expected_recipient {
            tracing::warn!(
                "Recipient mismatch: got {}, expected {}",
                recipient,
                expected_recipient
            );
            return Ok(VerifyResponse::Invalid {
                reason: FacilitatorErrorReason::FreeForm("invalid_payment".to_string()),
                payer: Some(sender_mixed),
            });
        }

        // Verify amount matches requirements
        let amount_token = crate::types::TokenAmount::from(amount);
        let expected_amount = &request.payment_requirements.max_amount_required;

        if amount_token != *expected_amount {
            tracing::warn!(
                "Amount mismatch: got {}, expected {}",
                amount,
                expected_amount
            );
            return Ok(VerifyResponse::Invalid {
                reason: FacilitatorErrorReason::FreeForm("invalid_payment".to_string()),
                payer: Some(sender_mixed),
            });
        }

        // TODO: Simulate the transaction to ensure it will succeed
        // This requires using the Aptos REST client to submit a simulation request

        tracing::info!("Aptos payment verification successful for payer: {}", sender);

        Ok(VerifyResponse::Valid {
            payer: sender_mixed,
        })
    }

    async fn settle(&self, request: &SettleRequest) -> Result<SettleResponse, Self::Error> {
        tracing::info!(
            "Settling Aptos payment for network: {}",
            self.chain.network
        );

        // Extract Aptos payload
        let aptos_payload = match &request.payment_payload.payload {
            ExactPaymentPayload::Aptos(payload) => payload,
            _ => {
                return Err(FacilitatorLocalError::DecodingError(
                    "Expected Aptos payload".to_string(),
                ))
            }
        };

        // Deserialize transaction
        let (raw_txn, authenticator, _entry_function) = deserialize_aptos_transaction(aptos_payload)?;

        let sender = raw_txn.sender();
        let sender_mixed = MixedAddress::Aptos(sender);

        // Create signed transaction for submission by combining the raw transaction with its authenticator (signature)
        use aptos_types::transaction::SignedTransaction as AptosSignedTransaction;
        let signed_txn = AptosSignedTransaction::new_single_sender(raw_txn, authenticator);

        tracing::info!("Submitting transaction to Aptos network from sender: {}", sender);

        self.rest_client
            .submit_bcs(&signed_txn)
            .await
            .map_err(|e| {
                FacilitatorLocalError::ContractCall(format!("Failed to submit transaction: {}", e))
            })?;

        // Compute transaction hash for tracking
        // The transaction hash is derived from the BCS-serialized SignedTransaction using SHA3-256.
        // This hash matches the one assigned to the transaction on-chain once it's committed.
        let signed_txn_bytes = bcs::to_bytes(&signed_txn).map_err(|e| {
            FacilitatorLocalError::ContractCall(format!("Failed to serialize signed transaction: {}", e))
        })?;

        use aptos_crypto::HashValue;
        let txn_hash = HashValue::sha3_256_of(&signed_txn_bytes);
        let txn_hash_bytes: [u8; 32] = txn_hash.to_vec().try_into().map_err(|_| {
            FacilitatorLocalError::ContractCall("Invalid transaction hash length".to_string())
        })?;

        tracing::info!("Transaction submitted successfully with hash: 0x{}", alloy::hex::encode(&txn_hash_bytes));

        // Note: We don't wait for confirmation here. The transaction is submitted and will be processed by the network.
        // The client can check the transaction status using the returned hash.

        Ok(SettleResponse {
            success: true,
            error_reason: None,
            transaction: Some(TransactionHash::Aptos(txn_hash_bytes)),
            network: request.payment_payload.network,
            payer: sender_mixed,
        })
    }

    async fn supported(&self) -> Result<SupportedPaymentKindsResponse, Self::Error> {
        tracing::info!("Returning supported payment kinds for Aptos");

        Ok(SupportedPaymentKindsResponse {
            kinds: vec![SupportedPaymentKind {
                x402_version: X402Version::V1,
                scheme: Scheme::Exact,
                network: self.chain.network.to_string(),
                extra: None,
            }],
        })
    }
}
