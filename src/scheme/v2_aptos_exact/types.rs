use crate::chain::aptos::Address;
use crate::proto::util::U64String;
use crate::proto::v2;
use crate::scheme::v1_eip155_exact::types::ExactScheme;
use serde::{Deserialize, Serialize};

pub type VerifyRequest = v2::VerifyRequest<PaymentPayload, PaymentRequirements>;
pub type SettleRequest = VerifyRequest;
pub type PaymentPayload = v2::PaymentPayload<PaymentRequirements, ExactAptosPayload>;
pub type PaymentRequirements = v2::PaymentRequirements<ExactScheme, U64String, Address, Option<()>>;

/// Aptos payment payload containing a base64-encoded BCS transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExactAptosPayload {
    pub transaction: String,
}
