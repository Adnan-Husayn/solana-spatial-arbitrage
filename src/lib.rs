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

    let keypair = parse_keypair(&key_string)?;
    Ok((rpc_url, keypair))
}

/// Parses a keypair from a JSON byte array or a base58 string. Error messages never echo the key.
pub fn parse_keypair(key: &str) -> Result<Keypair> {
    let key = key.trim();
    let bytes: Vec<u8> = match serde_json::from_str(key) {
        Ok(bytes) => bytes,
        Err(_) => bs58::decode(key)
            .into_vec()
            .map_err(|_| anyhow!("invalid key format (expected JSON array or base58)"))?,
    };
    Keypair::try_from(bytes.as_slice()).map_err(|_| anyhow!("invalid keypair bytes"))
}

/// Derives the websocket URL from an HTTP RPC URL.
pub fn ws_url(rpc_url: &str) -> String {
    rpc_url
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::signature::Signer;

    #[test]
    fn parses_json_and_base58_keys() {
        let kp = Keypair::new();
        let bytes = kp.to_bytes();
        let json = serde_json::to_string(&bytes.to_vec()).unwrap();
        let b58 = bs58::encode(bytes).into_string();
        assert_eq!(parse_keypair(&json).unwrap().pubkey(), kp.pubkey());
        assert_eq!(parse_keypair(&b58).unwrap().pubkey(), kp.pubkey());
    }

    #[test]
    fn rejects_bad_keys_without_echoing_them() {
        let err = parse_keypair("not-a-key-0OIl").unwrap_err().to_string();
        assert!(!err.contains("not-a-key"));
        assert!(parse_keypair("[1,2,3]").is_err());
    }

    #[test]
    fn ws_url_swaps_scheme() {
        assert_eq!(ws_url("https://x.io/?k=1"), "wss://x.io/?k=1");
        assert_eq!(ws_url("http://localhost:8899"), "ws://localhost:8899");
    }
}
