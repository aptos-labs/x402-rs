use async_trait::async_trait;
use aptos_crypto::ed25519::{Ed25519PrivateKey, Ed25519PublicKey};
use aptos_rest_client::Client as AptosClient;
use aptos_types::{
    account_address::AccountAddress,
    chain_id::ChainId,
    transaction::{
        authenticator::{AccountAuthenticator, AuthenticationKey},
        EntryFunction, RawTransaction, TransactionPayload,
    },
};
use base64::Engine;
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::ModuleId;
use std::str::FromStr;
use std::sync::Arc;
use x402_rs::chain::aptos::Address as AptosAddress;
use x402_rs::chain::ChainId as X402ChainId;
use x402_rs::network::NetworkFamily;
use x402_rs::proto::v2::PaymentPayload as V2PaymentPayload;
use x402_rs::scheme::v1_eip155_exact::types::ExactScheme;
use x402_rs::scheme::v2_aptos_exact::types::ExactAptosPayload;

use crate::chains::{IntoSenderWallet, SenderWallet};
use crate::X402PaymentsError;

/// Internal structure for JSON payload encoding
#[derive(serde::Serialize)]
struct AptosPayloadJson {
    transaction: Vec<u8>,
    #[serde(rename = "senderAuthenticator")]
    sender_authenticator: Vec<u8>,
}

#[derive(Clone)]
pub struct AptosSenderWallet {
    private_key: Arc<Ed25519PrivateKey>,
    account_address: AccountAddress,
    rest_client: Arc<AptosClient>,
}

impl AptosSenderWallet {
    /// Create a new Aptos sender wallet from a private key
    ///
    /// # Arguments
    /// * `private_key_hex` - Ed25519 private key as hex string (with or without 0x prefix)
    /// * `rest_client` - Aptos REST client
    pub fn new(
        private_key_hex: &str,
        rest_client: AptosClient,
    ) -> Result<Self, X402PaymentsError> {
        // Normalize the private key (remove 0x prefix if present)
        let normalized_key = private_key_hex.trim_start_matches("0x");

        // Parse the private key
        let private_key_bytes = hex::decode(normalized_key).map_err(|e| {
            X402PaymentsError::SigningError(format!("Failed to decode private key hex: {}", e))
        })?;

        let private_key = Ed25519PrivateKey::try_from(private_key_bytes.as_slice()).map_err(
            |e| X402PaymentsError::SigningError(format!("Failed to parse Ed25519 key: {}", e)),
        )?;

        // Derive public key and account address using AuthenticationKey
        let public_key = Ed25519PublicKey::from(&private_key);
        let authentication_key = AuthenticationKey::ed25519(&public_key);
        let account_address = authentication_key.account_address();

        Ok(Self {
            private_key: Arc::new(private_key),
            account_address,
            rest_client: Arc::new(rest_client),
        })
    }

    /// Get the account address
    pub fn address(&self) -> AccountAddress {
        self.account_address
    }
}

impl IntoSenderWallet for AptosSenderWallet {
    fn into_sender_wallet(self) -> Arc<dyn SenderWallet> {
        Arc::new(self)
    }
}

#[async_trait]
impl SenderWallet for AptosSenderWallet {
    fn can_handle(&self, requirements: &x402_rs::proto::PaymentRequirements) -> bool {
        // Check if this is an Aptos network by parsing the chain ID
        let chain_id_str = requirements.network.clone();
        if let Ok(chain_id) = X402ChainId::from_str(&chain_id_str) {
            chain_id.namespace() == "aptos"
        } else {
            false
        }
    }

    async fn payment_payload(
        &self,
        selected: x402_rs::proto::PaymentRequirements,
    ) -> Result<x402_rs::proto::PaymentPayload, X402PaymentsError> {
        // Fetch current sequence number from chain
        let account_info = self
            .rest_client
            .get_account(self.account_address)
            .await
            .map_err(|e| {
                X402PaymentsError::SigningError(format!("Failed to fetch account info: {}", e))
            })?
            .into_inner();

        let sequence_number = account_info.sequence_number;

        // Parse recipient address
        let recipient: AptosAddress = selected.pay_to.clone().parse().map_err(|e: String| {
            X402PaymentsError::SigningError(format!("Failed to parse recipient address: {}", e))
        })?;
        let recipient: AccountAddress = (*recipient.inner()).clone();

        // Parse amount
        let amount: u64 = selected
            .amount
            .parse()
            .map_err(|e| {
                X402PaymentsError::SigningError(format!("Failed to parse amount: {}", e))
            })?;

        // Parse FA (Fungible Asset) address
        let fa_address: AptosAddress = selected.asset.clone().parse().map_err(|e: String| {
            X402PaymentsError::SigningError(format!("Failed to parse FA address: {}", e))
        })?;
        let fa_address: AccountAddress = (*fa_address.inner()).clone();

        // Build transaction payload - using primary_fungible_store::transfer
        let module_id = ModuleId::new(
            AccountAddress::ONE,
            Identifier::new("primary_fungible_store").map_err(|e| {
                X402PaymentsError::SigningError(format!("Failed to create module id: {}", e))
            })?,
        );

        let function_name = Identifier::new("transfer").map_err(|e| {
            X402PaymentsError::SigningError(format!("Failed to create function name: {}", e))
        })?;

        let payload = TransactionPayload::EntryFunction(EntryFunction::new(
            module_id,
            function_name,
            vec![],
            vec![
                bcs::to_bytes(&fa_address).map_err(|e| {
                    X402PaymentsError::SigningError(format!("Failed to serialize FA address: {}", e))
                })?,
                bcs::to_bytes(&recipient).map_err(|e| {
                    X402PaymentsError::SigningError(format!("Failed to serialize recipient: {}", e))
                })?,
                bcs::to_bytes(&amount).map_err(|e| {
                    X402PaymentsError::SigningError(format!("Failed to serialize amount: {}", e))
                })?,
            ],
        ));

        // Determine chain ID based on network
        let chain_id_str = selected.network.clone();
        let x402_chain_id = X402ChainId::from_str(&chain_id_str)
            .map_err(|e| X402PaymentsError::SigningError(format!("Invalid chain ID: {}", e)))?;

        let chain_id = match x402_chain_id.reference() {
            "1" => ChainId::mainnet(),
            "2" => ChainId::testnet(),
            _ => {
                return Err(X402PaymentsError::SigningError(format!(
                    "Unsupported Aptos chain ID: {}",
                    x402_chain_id.reference()
                )))
            }
        };

        // Calculate expiration timestamp
        let expiration_timestamp_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 60; // 60 second expiration

        // Build raw transaction
        let raw_txn = RawTransaction::new(
            self.account_address,
            sequence_number,
            payload,
            100_000,                     // max_gas_amount
            100,                         // gas_unit_price
            expiration_timestamp_secs,
            chain_id,
        );

        // Sign the transaction
        use aptos_crypto::SigningKey;

        let signature = self
            .private_key
            .sign(&raw_txn)
            .map_err(|e| X402PaymentsError::SigningError(format!("Failed to sign: {:?}", e)))?;

        // Create authenticator
        let public_key = Ed25519PublicKey::from(&*self.private_key);
        let authenticator = AccountAuthenticator::ed25519(public_key, signature);

        // Serialize transaction and authenticator separately
        let raw_txn_bytes = bcs::to_bytes(&raw_txn).map_err(|e| {
            X402PaymentsError::SigningError(format!("Failed to serialize raw transaction: {}", e))
        })?;

        let authenticator_bytes = bcs::to_bytes(&authenticator).map_err(|e| {
            X402PaymentsError::SigningError(format!("Failed to serialize authenticator: {}", e))
        })?;

        // Create JSON payload
        let aptos_payload_json = AptosPayloadJson {
            transaction: raw_txn_bytes,
            sender_authenticator: authenticator_bytes,
        };

        // Serialize to JSON then base64 encode
        let json_str = serde_json::to_string(&aptos_payload_json).map_err(|e| {
            X402PaymentsError::SigningError(format!("Failed to serialize to JSON: {}", e))
        })?;

        let base64_transaction = base64::engine::general_purpose::STANDARD.encode(json_str);

        // Create the v2 payment payload using proto types
        let scheme_str = selected.scheme.clone();
        let network_str = selected.network.clone();

        let payload = ExactAptosPayload {
            transaction: base64_transaction,
        };

        // Serialize the Aptos payload
        let payload_json = serde_json::to_value(&payload).map_err(|e| {
            X402PaymentsError::SigningError(format!("Failed to serialize payload: {}", e))
        })?;

        // Create the proto payment payload
        let payment_payload = x402_rs::proto::PaymentPayload {
            x402_version: 2,
            scheme: scheme_str,
            network: network_str,
            payload: payload_json,
            accepted: Some(selected),
        };

        Ok(payment_payload)
    }
}
