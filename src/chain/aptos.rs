use aptos_rest_client::Client as AptosClient;
use aptos_types::account_address::AccountAddress;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use crate::chain::{FacilitatorLocalError, FromEnvByNetworkBuild, NetworkProviderOps};
use crate::facilitator::Facilitator;
use crate::network::Network;
use crate::types::{
    MixedAddress, SettleRequest, SettleResponse, SupportedPaymentKind,
    SupportedPaymentKindsResponse, VerifyRequest, VerifyResponse,
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

impl Facilitator for AptosProvider {
    type Error = FacilitatorLocalError;

    async fn verify(&self, _request: &VerifyRequest) -> Result<VerifyResponse, Self::Error> {
        tracing::info!(
            "Verifying Aptos payment for network: {}",
            self.chain.network
        );

        // TODO: Implement verification logic
        // 1. Decode the signed transaction from the payment payload
        // 2. Verify the signature
        // 3. Check the user's balance
        // 4. Validate the transaction matches payment requirements

        Err(FacilitatorLocalError::ContractCall(
            "Aptos verification not yet implemented".to_string(),
        ))
    }

    async fn settle(&self, _request: &SettleRequest) -> Result<SettleResponse, Self::Error> {
        tracing::info!(
            "Settling Aptos payment for network: {}",
            self.chain.network
        );

        // TODO: Implement settlement logic
        // 1. Verify the payment (reuse verify logic)
        // 2. Submit the user's signed transaction as a fee-payer sponsored transaction
        // 3. Wait for confirmation
        // 4. Return the transaction hash

        Err(FacilitatorLocalError::ContractCall(
            "Aptos settlement not yet implemented".to_string(),
        ))
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
