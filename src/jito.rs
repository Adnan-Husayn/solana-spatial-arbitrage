use solana_sdk::{
    transaction::VersionedTransaction,
    pubkey::Pubkey,
    system_instruction,
    signature::{Keypair, Signer},
    hash::Hash,
};
use reqwest::Client;
use serde_json::json;
use rand::seq::SliceRandom;
use std::str::FromStr;

const JITO_URL: &str = "https://amsterdam.mainnet.block-engine.jito.wtf/api/v1/bundles";

const TIP_ACCOUNTS: [&str; 8] = [
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PuwqhdX3Hh9rPN",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKkf8tVg9LPBaXRWMrn",
    "ADuUkR4ykGytmnb5qY1RuXDpnYdZHN82n5pZGQX63Mr5",
    "DttWaMuVvTiduZRNgLcGW9t66tePvm6znjs5dB088",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnIzKZ6jJ",
];

pub struct JitoClient {
    client: Client,
}

impl JitoClient {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }

    pub fn add_tip_instruction(
        &self, 
        user_keypair: &Keypair, 
        tip_amount_lamports: u64
    ) -> solana_sdk::instruction::Instruction {
        let mut rng = rand::thread_rng();
        let tip_account_str = TIP_ACCOUNTS.choose(&mut rng).unwrap();
        let tip_account = Pubkey::from_str(tip_account_str).unwrap();

        system_instruction::transfer(
            &user_keypair.pubkey(),
            &tip_account,
            tip_amount_lamports,
        )
    }

    pub async fn send_bundle(
        &self, 
        transactions: Vec<VersionedTransaction>
    ) -> anyhow::Result<String> {
        
        let encoded_txs: Vec<String> = transactions
            .iter()
            .map(|tx| bs58::encode(bincode::serialize(tx).unwrap()).into_string())
            .collect();

        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendBundle",
            "params": [
                encoded_txs
            ]
        });

        println!("Sending Bundle to Jito...");

        let response = self.client
            .post(JITO_URL)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .await?;

        let resp_json: serde_json::Value = response.json().await?;
        
        if let Some(result) = resp_json.get("result") {
            let bundle_id = result.as_str().unwrap_or("Unknown").to_string();
            println!("Bundle Sent! ID: {}", bundle_id);
            Ok(bundle_id)
        } else {
            let err = resp_json.get("error").map(|e| e.to_string()).unwrap_or("Unknown Error".to_string());
            println!("Jito Error: {}", err);
            Err(anyhow::anyhow!("Jito Error: {}", err))
        }
    }
}