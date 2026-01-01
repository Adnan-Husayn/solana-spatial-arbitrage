use std::env;
use anyhow::{Ok, Result};
use dotenv::dotenv;
use solana_sdk::signature::{Keypair, read_keypair_file};
use solana_sdk::signer::Signer;

pub fn load_env_variables() -> Result<(String, Keypair)> {
    dotenv().ok();

    let rpc_url = env::var("RPC_URL").expect("RPC_URL must be set in .env");
    
    let key_string = env::var("PRIVATE_KEY")
        .expect("PRIVATE_KEY must be set in .env");

    
    let key_bytes: Vec<u8> = serde_json::from_str(&key_string)
        .or_else(|_| {
            
            bs58::decode(&key_string)
                .into_vec()
                .map_err(|e| serde_json::json!({"error": e.to_string()}))
        })
        .expect("Invalid key format - must be JSON array or base58 string");
    
    let keypair = Keypair::from_bytes(&key_bytes)
        .expect("Invalid Keypair bytes");

    Ok((rpc_url, keypair))
}