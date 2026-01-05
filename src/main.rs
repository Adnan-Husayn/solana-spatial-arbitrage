use futures::{StreamExt, stream};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    nonblocking::pubsub_client::PubsubClient,
    rpc_config::{RpcAccountInfoConfig, RpcTransactionLogsConfig, RpcTransactionLogsFilter},
};
use solana_sdk::{commitment_config::CommitmentConfig, program_pack::Pack, pubkey::Pubkey};
use spatial_arbitrage_bot::load_env_variables;
use spl_token::state::Account as TokenAccount;
use std::{
    str::FromStr,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};


const SOL_VAULT_ADDR: &str = "DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz";
const USDC_VAULT_ADDR: &str = "HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz";
const RAYDIUM_POOL_ID: &str = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";


const ORCA_POOL_ADDR: &str = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE";
const ORCA_SQRT_PRICE_OFFSET: usize = 65;

#[derive(Debug, Clone, Copy)]
pub struct MarketState {
    pub ray_sol: u64,
    pub ray_usdc: u64,
    pub orca_sqrt_price: u128,
    pub last_update: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (rpc_url, _) = load_env_variables()?;
    let ws_url = rpc_url.replace("https", "wss");

    println!("Connecting to Helius WSS");

    
    let client_state = PubsubClient::new(&ws_url).await?;
    let client_logs = PubsubClient::new(&ws_url).await?;

    
    let state = Arc::new(RwLock::new(MarketState {
        ray_sol: 0,
        ray_usdc: 0,
        orca_sqrt_price: 0,
        last_update: 0,
    }));

    
    println!("Bootstrapping Raydium & Orca State");
    let rpc_http = solana_client::rpc_client::RpcClient::new(rpc_url.clone());

    let ray_sol_pk = Pubkey::from_str(SOL_VAULT_ADDR)?;
    let ray_usdc_pk = Pubkey::from_str(USDC_VAULT_ADDR)?;
    let orca_pk = Pubkey::from_str(ORCA_POOL_ADDR)?;

    let accounts = rpc_http.get_multiple_accounts(&[ray_sol_pk, ray_usdc_pk, orca_pk])?;

    if let (Some(s), Some(u), Some(o)) = (&accounts[0], &accounts[1], &accounts[2]) {
        let r_sol = TokenAccount::unpack(&s.data)?.amount;
        let r_usdc = TokenAccount::unpack(&u.data)?.amount;

        
        if o.data.len() >= ORCA_SQRT_PRICE_OFFSET + 16 {
            let sqrt_bytes: [u8; 16] =
                o.data[ORCA_SQRT_PRICE_OFFSET..ORCA_SQRT_PRICE_OFFSET + 16].try_into()?;
            let o_sqrt = u128::from_le_bytes(sqrt_bytes);

            let mut w = state.write().unwrap();
            w.ray_sol = r_sol;
            w.ray_usdc = r_usdc;
            w.orca_sqrt_price = o_sqrt;
            w.last_update = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

            println!("Bootstrap Complete");
            println!("   Raydium: {} SOL / {} USDC", r_sol, r_usdc);
            println!("   Orca Sqrt: {}", o_sqrt);
        }
    } else {
        println!("Bootstrap Failed: Could not fetch accounts.");
        return Ok(());
    }

    
    let state_bg = state.clone();
    tokio::spawn(async move {
        println!("Background Account Stream Started...");
        let config = RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::confirmed()),
            ..RpcAccountInfoConfig::default()
        };

        let (s_stream, _) = client_state
            .account_subscribe(&ray_sol_pk, Some(config.clone()))
            .await
            .unwrap();
        let (u_stream, _) = client_state
            .account_subscribe(&ray_usdc_pk, Some(config.clone()))
            .await
            .unwrap();
        let (o_stream, _) = client_state
            .account_subscribe(&orca_pk, Some(config))
            .await
            .unwrap();

        let mut combined = stream::select_all(vec![
            s_stream.map(|x| ("RAY_SOL", x)).boxed(),
            u_stream.map(|x| ("RAY_USDC", x)).boxed(),
            o_stream.map(|x| ("ORCA", x)).boxed(),
        ]);

        while let Some((label, msg)) = combined.next().await {
            if let Some(decoded) = msg.value.data.decode() {
                let mut w = state_bg.write().unwrap();

                if label == "ORCA" {
                    if decoded.len() >= ORCA_SQRT_PRICE_OFFSET + 16 {
                        let bytes: [u8; 16] = decoded
                            [ORCA_SQRT_PRICE_OFFSET..ORCA_SQRT_PRICE_OFFSET + 16]
                            .try_into()
                            .unwrap();
                        w.orca_sqrt_price = u128::from_le_bytes(bytes);
                    }
                } else {
                    if let Ok(acc) = TokenAccount::unpack(&decoded) {
                        if label == "RAY_SOL" {
                            w.ray_sol = acc.amount;
                        } else {
                            w.ray_usdc = acc.amount;
                        }
                    }
                }
                w.last_update = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
            }
        }
    });

    
    println!("Listening for logs & calculating net profit...");
    let filter = RpcTransactionLogsFilter::Mentions(vec![RAYDIUM_POOL_ID.to_string()]);
    let log_config = RpcTransactionLogsConfig {
        commitment: Some(CommitmentConfig::processed()),
    };
    let (mut log_stream, _) = client_logs.logs_subscribe(filter, log_config).await?;

    while let Some(log) = log_stream.next().await {
        
        let should_print = log.value.signature.as_bytes()[0] % 5 == 0;

        let r = state.read().unwrap();

        if r.ray_sol > 0 && r.ray_usdc > 0 && r.orca_sqrt_price > 0 {
            
            let ray_price = (r.ray_usdc as f64 / 1e6) / (r.ray_sol as f64 / 1e9);

            let sqrt_f64 = r.orca_sqrt_price as f64;
            let shift_64 = (1u128 << 64) as f64;
            let orca_price = (sqrt_f64 / shift_64).powi(2) * 1000.0; 

            
            let ray_fee = 0.0025; 
            let orca_fee = 0.0004; 

            
            let mut profit_usd = 0.0;
            let mut direction = "NONE";

            if ray_price < orca_price {
                
                let buy_cost = ray_price * (1.0 + ray_fee);
                let sell_val = orca_price * (1.0 - orca_fee);
                profit_usd = sell_val - buy_cost;
                direction = "Buy RAY -> Sell ORCA";
            } else {
                
                let buy_cost = orca_price * (1.0 + orca_fee);
                let sell_val = ray_price * (1.0 - ray_fee);
                profit_usd = sell_val - buy_cost;
                direction = "Buy ORCA -> Sell RAY";
            }

            if profit_usd > -0.50 && should_print {
                println!("TRADE DETECTED: {}", log.value.signature);
                println!("   Prices: Ray ${:.4} | Orca ${:.4}", ray_price, orca_price);
                println!("   Direction: {}", direction);
                println!("   Net Profit (1 SOL): ${:.4}", profit_usd);
                println!("---------------------------------------");
            }

            if profit_usd > 0.0 {
                println!("PROFITABLE OPPORTUNITY FOUND! Executing...");
                println!("   Prices: Ray ${:.4} | Orca ${:.4}", ray_price, orca_price);
                println!("   Direction: {}", direction);
                println!("   Net Profit (1 SOL): ${:.4}", profit_usd);
                println!("---------------------------------------");
            }
        }
    }

    Ok(())
}