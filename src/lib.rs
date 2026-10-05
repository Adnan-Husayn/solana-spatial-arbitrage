use anyhow::{Context, Result, anyhow};
use dotenv::dotenv;
use solana_sdk::signature::Keypair;
use std::env;

pub mod config;
pub mod executor;
pub mod instructions;
pub mod jito;
pub mod listener;
pub mod math;
pub mod orca;
pub mod pricing;
pub mod raydium;
pub mod state;
pub mod strategy;

/// Loads `RPC_URL` and `PRIVATE_KEY` (JSON byte array or base58) from the environment / `.env`.
pub fn load_env_variables() -> Result<(String, Keypair)> {
    dotenv().ok();

    let rpc_url = env::var("RPC_URL").context("RPC_URL must be set in .env")?;
    let key_string = env::var("PRIVATE_KEY").context("PRIVATE_KEY must be set in .env")?;

    let key_bytes: Vec<u8> = match serde_json::from_str(&key_string) {
        Ok(bytes) => bytes,
        Err(_) => bs58::decode(&key_string)
            .into_vec()
            .map_err(|e| anyhow!("invalid key format (expected JSON array or base58): {e}"))?,
    };

    let keypair =
        Keypair::from_bytes(&key_bytes).map_err(|e| anyhow!("invalid keypair bytes: {e}"))?;
    Ok((rpc_url, keypair))
}

/// Derives the websocket URL from an HTTP RPC URL.
pub fn ws_url(rpc_url: &str) -> String {
    rpc_url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1)
}
