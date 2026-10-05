use rand::seq::SliceRandom;
use reqwest::Client;
use serde_json::json;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::VersionedTransaction,
};
use std::str::FromStr;

pub const DEFAULT_JITO_URL: &str = "https://amsterdam.mainnet.block-engine.jito.wtf/api/v1/bundles";

/// Jito tip accounts, as returned by the block engine's `getTipAccounts` (identical on all regions).
pub const TIP_ACCOUNTS: [&str; 8] = [
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT",
];

/// What happened to a submitted bundle, per `getInflightBundleStatuses`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleOutcome {
    Landed { slot: u64 },
    Failed,
    Invalid,
    Pending,
}

/// Extracts one bundle's outcome from a `getInflightBundleStatuses` response.
pub fn parse_bundle_status(resp: &serde_json::Value, bundle_id: &str) -> Option<BundleOutcome> {
    let entry = resp
        .pointer("/result/value")?
        .as_array()?
        .iter()
        .find(|e| e.get("bundle_id").and_then(|v| v.as_str()) == Some(bundle_id))?;
    Some(match entry.get("status")?.as_str()? {
        "Landed" => BundleOutcome::Landed {
            slot: entry
                .get("landed_slot")
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
        },
        "Failed" => BundleOutcome::Failed,
        "Invalid" => BundleOutcome::Invalid,
        _ => BundleOutcome::Pending,
    })
}

pub struct JitoClient {
    client: Client,
    url: String,
}

impl Default for JitoClient {
    fn default() -> Self {
        Self::new()
    }
}

impl JitoClient {
    pub fn new() -> Self {
        Self::with_url(DEFAULT_JITO_URL)
    }

    /// `url` is a block engine bundles endpoint, e.g. `https://<region>.mainnet.block-engine.jito.wtf/api/v1/bundles`.
    pub fn with_url(url: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            url: url.into(),
        }
    }

    pub fn add_tip_instruction(
        &self,
        user_keypair: &Keypair,
        tip_amount_lamports: u64,
    ) -> solana_sdk::instruction::Instruction {
        let mut rng = rand::thread_rng();
        let tip_account_str = TIP_ACCOUNTS
            .choose(&mut rng)
            .expect("tip accounts non-empty");
        let tip_account =
            Pubkey::from_str(tip_account_str).expect("tip accounts are validated by tests");

        solana_system_interface::instruction::transfer(
            &user_keypair.pubkey(),
            &tip_account,
            tip_amount_lamports,
        )
    }

    async fn rpc(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let payload = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
        let resp: serde_json::Value = self
            .client
            .post(&self.url)
            .json(&payload)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if let Some(err) = resp.get("error") {
            anyhow::bail!("jito {method} error: {err}");
        }
        Ok(resp)
    }

    /// Submits a bundle (max 5 transactions) and returns its id.
    pub async fn send_bundle(
        &self,
        transactions: Vec<VersionedTransaction>,
    ) -> anyhow::Result<String> {
        let encoded: Vec<String> = transactions
            .iter()
            .map(|tx| Ok(bs58::encode(bincode::serialize(tx)?).into_string()))
            .collect::<anyhow::Result<_>>()?;

        tracing::info!("sending bundle to Jito");
        let resp = self
            .rpc("sendBundle", json!([encoded, { "encoding": "base58" }]))
            .await?;
        let id = resp
            .get("result")
            .and_then(|r| r.as_str())
            .ok_or_else(|| anyhow::anyhow!("jito sendBundle returned no bundle id"))?
            .to_string();
        tracing::info!("bundle sent: {id}");
        Ok(id)
    }

    /// Polls until the bundle lands, fails, is rejected, or `timeout` passes.
    /// Inflight status is only kept for a few minutes, so this is meant for short waits.
    pub async fn wait_for_bundle(
        &self,
        bundle_id: &str,
        timeout: std::time::Duration,
    ) -> anyhow::Result<BundleOutcome> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let resp = self
                .rpc("getInflightBundleStatuses", json!([[bundle_id]]))
                .await?;
            match parse_bundle_status(&resp, bundle_id) {
                Some(BundleOutcome::Pending) | None => {}
                Some(done) => return Ok(done),
            }
            if std::time::Instant::now() >= deadline {
                return Ok(BundleOutcome::Pending);
            }
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tip_accounts_are_valid_pubkeys() {
        for a in TIP_ACCOUNTS {
            assert!(Pubkey::from_str(a).is_ok(), "invalid tip account {a}");
        }
    }

    #[test]
    fn parses_bundle_statuses() {
        let resp = json!({"result": {"context": {"slot": 1}, "value": [
            {"bundle_id": "a", "status": "Landed", "landed_slot": 42},
            {"bundle_id": "b", "status": "Failed", "landed_slot": null},
            {"bundle_id": "c", "status": "Pending", "landed_slot": null},
            {"bundle_id": "d", "status": "Invalid", "landed_slot": null}
        ]}});
        assert_eq!(
            parse_bundle_status(&resp, "a"),
            Some(BundleOutcome::Landed { slot: 42 })
        );
        assert_eq!(parse_bundle_status(&resp, "b"), Some(BundleOutcome::Failed));
        assert_eq!(
            parse_bundle_status(&resp, "c"),
            Some(BundleOutcome::Pending)
        );
        assert_eq!(
            parse_bundle_status(&resp, "d"),
            Some(BundleOutcome::Invalid)
        );
        assert_eq!(parse_bundle_status(&resp, "zzz"), None);
        assert_eq!(
            parse_bundle_status(&json!({"result": {"value": null}}), "a"),
            None
        );
    }
}
