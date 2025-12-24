# Aptos E2E Example

This example demonstrates a complete end-to-end x402 payment flow using Aptos blockchain.

## Overview

- **Server**: An Axum HTTP server that requires x402 payments for protected endpoints
- **Client**: A reqwest client that automatically handles 402 responses and submits Aptos fungible asset transfers

## Setup

### Prerequisites

1. Aptos testnet account with USDC
2. Aptos testnet RPC access

### Configuration

1. Copy `.env.example` to `.env`
2. Set your environment variables:
   - `APTOS_PRIVATE_KEY`: Your Aptos Ed25519 private key (hex format, with or without 0x prefix)
   - `APTOS_RPC_URL`: Aptos RPC endpoint (defaults to testnet)

   **Note**: The payment recipient address (`pay_to`) in the 402 response is set to `0x1` in this example. In production, this should be the address derived from your facilitator's private key.

### Get Testnet USDC

Aptos testnet USDC address:
```
0x69091fbab5f7d635ee7ac5098cf0c1efbe31d68fec0f2cd565e8d168daf52832
```

You can obtain testnet USDC through the Aptos faucet or testnet USDC faucet.

## Running

### Start the Server

```bash
cargo run --bin server
```

The server will listen on `http://127.0.0.1:3000` with:
- `/` - Public endpoint
- `/protected` - Requires 0.01 USDC payment on Aptos testnet

### Run the Client

In another terminal:

```bash
cargo run --bin client
```

The client will:
1. Connect to the server
2. Receive a 402 Payment Required response
3. Construct an Aptos fungible asset transfer transaction
4. Sign and submit the transaction
5. Retry the request with the payment proof
6. Display the response

## How It Works

### Server Flow

1. Server receives request to `/protected`
2. Returns `402 Payment Required` with Aptos payment requirements:
   - Network: `aptos:2` (testnet)
   - Asset: USDC fungible asset address
   - Amount: 10000 (0.01 USDC with 6 decimals)
   - Recipient: pay to address
   - Scheme: `exact` (v2)

### Client Flow

1. Client receives 402 response
2. Parses payment requirements
3. Creates Aptos transaction using `primary_fungible_store::transfer`:
   - Fetches sender's sequence number
   - Constructs entry function call with FA address, recipient, and amount
   - Signs transaction with Ed25519 private key
   - Encodes as base64 JSON payload
4. Retries request with `X-Payment` header
5. Server verifies:
   - Transaction signature
   - Correct entry function
   - Correct asset, recipient, and amount
   - Submits transaction to Aptos network
6. Returns protected resource

## Architecture

### Aptos v2 Integration

- **Chain Provider** (`src/chain/aptos.rs`): Manages Aptos REST client and Ed25519 keys
- **Scheme Handler** (`src/scheme/v2_aptos_exact/`): Implements verify/settle for Aptos fungible assets
- **Client Wallet** (`crates/x402-reqwest/src/chains/aptos.rs`): Constructs and signs Aptos transactions

### Transaction Format

Transactions are encoded as base64 JSON:
```json
{
  "transaction": [/* BCS-serialized RawTransaction */],
  "senderAuthenticator": [/* BCS-serialized AccountAuthenticator */]
}
```

This follows the Aptos v2 x402 specification using the `exact` scheme.

## Troubleshooting

### "Failed to fetch account info"
- Ensure your Aptos account exists on testnet
- Fund your account with APT for gas fees

### "Asset mismatch"
- Verify you're using the correct USDC testnet address
- Check the server's required asset address

### "Insufficient balance"
- Ensure you have enough USDC in your account
- Check your account has APT for gas fees

### "Transaction submission failed"
- Verify RPC endpoint is accessible
- Check your sequence number is correct
- Ensure transaction hasn't expired

## Learn More

- [x402 Protocol](https://x402.org)
- [Aptos Documentation](https://aptos.dev)
- [Fungible Assets on Aptos](https://aptos.dev/en/build/smart-contracts/fungible-asset)
