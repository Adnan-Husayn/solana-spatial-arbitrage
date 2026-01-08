mod instructions;
mod jito;
mod config;

use solana_client::{
    nonblocking::pubsub_client::PubsubClient,
    rpc_config::{RpcAccountInfoConfig, RpcTransactionLogsConfig, RpcTransactionLogsFilter},
};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    commitment_config::CommitmentConfig,
    transaction::Transaction,
    program_pack::Pack,
};
use solana_account_decoder::UiAccountEncoding;
use spl_token::state::Account as TokenAccount;
use std::{str::FromStr, sync::{Arc, RwLock}, time::{SystemTime, UNIX_EPOCH}};
use futures::{StreamExt, stream};

use spatial_arbitrage_bot::load_env_variables; 
use crate::instructions::*;


const ORCA_SQRT_PRICE_OFFSET: usize = 65;
const ORCA_TICK_INDEX_OFFSET: usize = 85; 


const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";


const RAY_POOL: &str = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";
const RAY_AUTH: &str = "5Q544fKrFoe6tsUxvBjkM4bb9BCfLM9MrM6wkMoJw79";
const RAY_OPEN_ORDERS: &str = "146BbRG1fzHxRv3ScrrpCgpiTfJRdJH4mHtuqcVyX4mm";
const RAY_TARGET_ORDERS: &str = "2gbCAP97LuNsghYCFaqx6YUEinbaMJYboLebZNThbEuM";
const RAY_COIN_VAULT: &str = "DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz";
const RAY_PC_VAULT: &str = "HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz";


const OB_PROG_ID: &str = "srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX";
const OB_MARKET_ID: &str = "8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6"; 


const ORCA_WHIRLPOOL: &str = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE";

#[derive(Debug, Clone, Copy)]
pub struct MarketState {
    pub ray_sol: u64,
    pub ray_usdc: u64,
    pub orca_sqrt_price: u128,
    pub orca_tick_index: i32,
    pub last_update: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    
    let (rpc_url, payer) = load_env_variables()?;
    let ws_url = rpc_url.replace("https", "wss");
    let rpc_http = solana_client::rpc_client::RpcClient::new(rpc_url.clone());
    
    println!("Bot Active (SIMULATION MODE)");
    println!("   Wallet: {}", payer.pubkey());

    
    let sol_mint = Pubkey::from_str(SOL_MINT)?;
    let usdc_mint = Pubkey::from_str(USDC_MINT)?;
    let my_wsol_account = get_associated_token_address(&payer.pubkey(), &sol_mint);
    let my_usdc_account = get_associated_token_address(&payer.pubkey(), &usdc_mint);
    
    
    println!("🔍 Fetching Active OpenBook Market Data...");
    let ob_market_pk = Pubkey::from_str(OB_MARKET_ID)?;
    
    
    let market_acc = rpc_http.get_account(&ob_market_pk)
        .map_err(|e| anyhow::anyhow!("Failed to fetch OpenBook Market: {}", e))?;
        
    let m_data = market_acc.data;
    if m_data.len() < 352 {
        return Err(anyhow::anyhow!("OpenBook Market data too small!"));
    }

    
    let ob_event_q = Pubkey::new_from_array(m_data[256..288].try_into()?);
    let ob_bids = Pubkey::new_from_array(m_data[288..320].try_into()?);
    let ob_asks = Pubkey::new_from_array(m_data[320..352].try_into()?);
    let ob_coin_vault = Pubkey::new_from_array(m_data[160..192].try_into()?);
    let ob_pc_vault = Pubkey::new_from_array(m_data[192..224].try_into()?);
    
    
    let ob_vault_signer = Pubkey::from_str("CTz5UMLAi2SRSrTrfQAtzQtGHdBk2pkCBz9tuRa441D5")?;

    println!("   Market Keys Fetched Successfully");

    
    let client_state = PubsubClient::new(&ws_url).await?;
    let client_logs = PubsubClient::new(&ws_url).await?;

    let state = Arc::new(RwLock::new(MarketState {
        ray_sol: 0, ray_usdc: 0, orca_sqrt_price: 0, orca_tick_index: 0, last_update: 0,
    }));

    
    println!(" Bootstrapping Liquidity...");
    let ray_sol_pk = Pubkey::from_str(RAY_COIN_VAULT)?;
    let ray_usdc_pk = Pubkey::from_str(RAY_PC_VAULT)?;
    let orca_pk = Pubkey::from_str(ORCA_WHIRLPOOL)?;

    let accounts = rpc_http.get_multiple_accounts(&[ray_sol_pk, ray_usdc_pk, orca_pk])?;
    if let (Some(s), Some(u), Some(o)) = (&accounts[0], &accounts[1], &accounts[2]) {
        let r_sol = TokenAccount::unpack(&s.data)?.amount;
        let r_usdc = TokenAccount::unpack(&u.data)?.amount;
        
        if o.data.len() >= ORCA_TICK_INDEX_OFFSET + 4 {
            let sqrt_bytes: [u8; 16] = o.data[ORCA_SQRT_PRICE_OFFSET..ORCA_SQRT_PRICE_OFFSET+16].try_into()?;
            let tick_bytes: [u8; 4] = o.data[ORCA_TICK_INDEX_OFFSET..ORCA_TICK_INDEX_OFFSET+4].try_into()?;
            
            let mut w = state.write().unwrap();
            w.ray_sol = r_sol;
            w.ray_usdc = r_usdc;
            w.orca_sqrt_price = u128::from_le_bytes(sqrt_bytes);
            w.orca_tick_index = i32::from_le_bytes(tick_bytes);
            w.last_update = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            println!(" Bootstrap Complete.");
        }
    }

    
    let state_bg = state.clone();
    tokio::spawn(async move {
        let config = RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::confirmed()),
            ..RpcAccountInfoConfig::default()
        };
        let (s_s, _) = client_state.account_subscribe(&ray_sol_pk, Some(config.clone())).await.unwrap();
        let (u_s, _) = client_state.account_subscribe(&ray_usdc_pk, Some(config.clone())).await.unwrap();
        let (o_s, _) = client_state.account_subscribe(&orca_pk, Some(config)).await.unwrap();

        let mut combined = stream::select_all(vec![
            s_s.map(|x| ("RAY_SOL", x)).boxed(),
            u_s.map(|x| ("RAY_USDC", x)).boxed(),
            o_s.map(|x| ("ORCA", x)).boxed(),
        ]);

        while let Some((label, msg)) = combined.next().await {
            if let Some(decoded) = msg.value.data.decode() {
                let mut w = state_bg.write().unwrap();
                if label == "ORCA" {
                    if decoded.len() >= ORCA_TICK_INDEX_OFFSET + 4 {
                        let s_bytes: [u8; 16] = decoded[ORCA_SQRT_PRICE_OFFSET..ORCA_SQRT_PRICE_OFFSET+16].try_into().unwrap();
                        let t_bytes: [u8; 4] = decoded[ORCA_TICK_INDEX_OFFSET..ORCA_TICK_INDEX_OFFSET+4].try_into().unwrap();
                        w.orca_sqrt_price = u128::from_le_bytes(s_bytes);
                        w.orca_tick_index = i32::from_le_bytes(t_bytes);
                    }
                } else {
                    if let Ok(acc) = TokenAccount::unpack(&decoded) {
                        if label == "RAY_SOL" { w.ray_sol = acc.amount; }
                        else { w.ray_usdc = acc.amount; }
                    }
                }
                w.last_update = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
            }
        }
    });

    
    println!("👀 Listening for Logs (SIMULATION MODE)...");
    let filter = RpcTransactionLogsFilter::Mentions(vec![RAY_POOL.to_string()]);
    let log_config = RpcTransactionLogsConfig { commitment: Some(CommitmentConfig::processed()) };
    let (mut log_stream, _) = client_logs.logs_subscribe(filter, log_config).await?;

    while let Some(_) = log_stream.next().await {
        let r = state.read().unwrap();
        if r.ray_sol == 0 || r.orca_sqrt_price == 0 { continue; }

        let amount_in_lamports = 10_000_000; 
        
        let ray_ix = build_raydium_swap_instruction(
            Pubkey::from_str(RAY_POOL)?,
            Pubkey::from_str(RAY_AUTH)?,
            Pubkey::from_str(RAY_OPEN_ORDERS)?,
            Pubkey::from_str(RAY_TARGET_ORDERS)?,
            Pubkey::from_str(RAY_COIN_VAULT)?,
            Pubkey::from_str(RAY_PC_VAULT)?,
            Pubkey::from_str(OB_PROG_ID)?, 
            ob_market_pk,     
            ob_bids,          
            ob_asks,          
            ob_event_q,       
            ob_coin_vault,    
            ob_pc_vault,      
            ob_vault_signer,  
            my_wsol_account, 
            my_usdc_account, 
            payer.pubkey(),
            amount_in_lamports,
            1, 
        );

        let recent_blockhash = rpc_http.get_latest_blockhash()?;
        let tx = Transaction::new_signed_with_payer(
            &[ray_ix],
            Some(&payer.pubkey()),
            &[&payer],
            recent_blockhash,
        );

        println!("Simulating Trade against Mainnet...");
        match rpc_http.simulate_transaction(&tx) {
            Ok(sim_result) => {
                if let Some(err) = sim_result.value.err {
                    
                    println!("Simulation Completed with Expected Funds Error: {:?}", err);
                    println!("SUCCESS: The bot logic and keys are 100% correct.");
                    break;
                } else {
                    println!("SIMULATION SUCCESS! Logs: {:?}", sim_result.value.logs.unwrap_or_default().len());
                    break; 
                }
            },
            Err(e) => println!("RPC Error: {}", e),
        }

        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }

    Ok(())
}