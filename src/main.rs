use solana_client::{
    nonblocking::pubsub_client::PubsubClient,
    rpc_config::{RpcAccountInfoConfig, RpcTransactionLogsConfig, RpcTransactionLogsFilter},
};
use solana_sdk::{
    pubkey::Pubkey,
    program_pack::Pack,
    commitment_config::CommitmentConfig,
};
use solana_account_decoder::UiAccountEncoding;
use spatial_arbitrage_bot::load_env_variables;
use spl_token::state::Account as TokenAccount;
use std::{str::FromStr, sync::{Arc, RwLock}, time::{SystemTime, UNIX_EPOCH}};
use futures::{StreamExt, stream};

const SOL_VAULT_ADDR: &str = "DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz";
const USDC_VAULT_ADDR: &str = "HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz";
const POOL_ID_ADDR: &str = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";

#[derive(Debug, Clone, Copy)]
pub struct MarketState {
    pub sol_reserves: u64,
    pub usdc_reserves: u64,
    pub last_update: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (rpc_url, _) = load_env_variables()?;
    let ws_url = rpc_url.replace("https", "wss");
    
    println!("Connecting to Helius WSS: {}", ws_url);
    
    let client_state = PubsubClient::new(&ws_url).await?;
    let client_logs = PubsubClient::new(&ws_url).await?;

    let state = Arc::new(RwLock::new(MarketState {
        sol_reserves: 0,
        usdc_reserves: 0,
        last_update: 0,
    }));

    println!("Bootstrapping...");
    let rpc_client_http = solana_client::rpc_client::RpcClient::new(rpc_url.clone());
    let sol_vault_pk = Pubkey::from_str(SOL_VAULT_ADDR)?;
    let usdc_vault_pk = Pubkey::from_str(USDC_VAULT_ADDR)?;

    let accounts = rpc_client_http.get_multiple_accounts(&[sol_vault_pk, usdc_vault_pk])?;
    if let (Some(sol_acc), Some(usdc_acc)) = (&accounts[0], &accounts[1]) {
        let sol = TokenAccount::unpack(&sol_acc.data)?.amount;
        let usdc = TokenAccount::unpack(&usdc_acc.data)?.amount;
        
        let mut w = state.write().unwrap();
        w.sol_reserves = sol;
        w.usdc_reserves = usdc;
        w.last_update = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        println!("Bootstrap: SOL {} | USDC {}", sol, usdc);
    }

    let state_bg = state.clone();
    
    tokio::spawn(async move {
        println!("Background Account Stream Started...");
        
        let config = RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::confirmed()),
            ..RpcAccountInfoConfig::default()
        };

        let (sol_s, _) = client_state.account_subscribe(&sol_vault_pk, Some(config.clone())).await.unwrap();
        let (usdc_s, _) = client_state.account_subscribe(&usdc_vault_pk, Some(config)).await.unwrap();

        let mut combined = stream::select(
            sol_s.map(|x| ("SOL", x)), 
            usdc_s.map(|x| ("USDC", x))
        );

        while let Some((label, msg)) = combined.next().await {
            if let Some(decoded) = msg.value.data.decode() {
                if let Ok(acc) = TokenAccount::unpack(&decoded) {
                    let mut w = state_bg.write().unwrap();
                    if label == "SOL" { w.sol_reserves = acc.amount; }
                    else { w.usdc_reserves = acc.amount; }
                    w.last_update = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                }
            }
        }
    });

    println!("Listening for TRADES ...");
    
    let filter = RpcTransactionLogsFilter::Mentions(vec![POOL_ID_ADDR.to_string()]);
    let log_config = RpcTransactionLogsConfig {
        commitment: Some(CommitmentConfig::processed()), 
    };

    let (mut log_stream, _) = client_logs.logs_subscribe(filter, log_config).await?;

    while let Some(log) = log_stream.next().await {
        println!("TRADE DETECTED!! Tx: {}", log.value.signature);

        let r_state = state.read().unwrap();
        
        if r_state.sol_reserves > 0 && r_state.usdc_reserves > 0 {
            let price = (r_state.usdc_reserves as f64 / 1e6) / (r_state.sol_reserves as f64 / 1e9);
            println!("   Cached Price: ${:.4} (Freshness: {}s)", 
                price,
                SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() - r_state.last_update
            );

            let fake_orca_price = price * 1.005;
            let spread = fake_orca_price - price;
            
            if spread > (price * 0.003) {
                 println!("   OPPORTUNITY! Spread: ${:.4}", spread);
            }
        }
    }

    Ok(())
}