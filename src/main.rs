use anyhow::Result;
use futures::StreamExt;
use solana_client::{
    nonblocking::{pubsub_client::PubsubClient, rpc_client::RpcClient},
    rpc_config::{RpcTransactionLogsConfig, RpcTransactionLogsFilter},
};
use solana_sdk::{commitment_config::CommitmentConfig, program_pack::Pack, pubkey::Pubkey, signature::Signer};
use spatial_arbitrage_bot::{
    config::*, executor, listener, load_env_variables, pricing, state, strategy, ws_url,
};
use spl_token::state::Account as TokenAccount;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<()> {
    let (rpc_url, payer) = load_env_variables()?;
    let ws = ws_url(&rpc_url);
    let rpc = RpcClient::new(rpc_url);

    println!("Bot Active (SIMULATION MODE)");
    println!("   Wallet: {}", payer.pubkey());

    let keys = executor::fetch_market_keys(&rpc).await?;
    println!("   OpenBook market keys fetched");

    let shared = state::new_shared();

    // Bootstrap so the first log event already has data.
    let accounts = rpc
        .get_multiple_accounts(&[
            Pubkey::from_str(RAY_COIN_VAULT)?,
            Pubkey::from_str(RAY_PC_VAULT)?,
            Pubkey::from_str(ORCA_WHIRLPOOL)?,
        ])
        .await?;
    if let [Some(s), Some(u), Some(o)] = accounts.as_slice() {
        state::set_ray_sol(&shared, TokenAccount::unpack(&s.data)?.amount)?;
        state::set_ray_usdc(&shared, TokenAccount::unpack(&u.data)?.amount)?;
        state::set_orca(&shared, &o.data)?;
        println!("   Bootstrap complete");
    }

    tokio::spawn(listener::run(ws.clone(), shared.clone()));

    let logs_client = PubsubClient::new(&ws).await?;
    let filter = RpcTransactionLogsFilter::Mentions(vec![RAY_POOL.to_string()]);
    let cfg = RpcTransactionLogsConfig { commitment: Some(CommitmentConfig::processed()) };
    let (mut log_stream, _unsub) = logs_client.logs_subscribe(filter, cfg).await?;
    println!("Listening for Raydium pool activity...");

    let cfg = strategy::StrategyConfig::default();
    let mut simulated = false;
    while log_stream.next().await.is_some() {
        let snap = state::snapshot(&shared)?;
        if !snap.is_ready() {
            continue;
        }
        if let Some(s) = pricing::spread_from_state(snap.ray_sol, snap.ray_usdc, snap.orca_sqrt_price) {
            println!(
                "ray {:.4}  orca {:.4}  spread {:.2} bps  {:?}",
                s.ray_price, s.orca_price, s.bps, s.direction
            );
        }

        if let Some(o) = strategy::evaluate(&snap, &cfg) {
            println!(
                "OPPORTUNITY {:?}: in {:.4} SOL, net {:.6} SOL",
                o.direction,
                o.amount_in as f64 / 1e9,
                o.net_profit as f64 / 1e9
            );
        }

        // The combined transaction replaces this one-shot simulation with the real strategy loop.
        if !simulated {
            simulated = true;
            match executor::simulate_raydium_swap(&rpc, &payer, &keys, 10_000_000).await {
                Ok(None) => println!("Simulation succeeded"),
                Ok(Some(err)) => println!("Simulation returned error: {err}"),
                Err(e) => println!("RPC error: {e:#}"),
            }
        }
    }
    Ok(())
}
