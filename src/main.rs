use anyhow::Result;
use futures::StreamExt;
use solana_client::{
    nonblocking::{pubsub_client::PubsubClient, rpc_client::RpcClient},
    rpc_config::{RpcTransactionLogsConfig, RpcTransactionLogsFilter},
};
use solana_sdk::{
    commitment_config::CommitmentConfig, program_pack::Pack, pubkey::Pubkey, signature::Signer,
};
use spatial_arbitrage_bot::{
    config::*, executor, jito::JitoClient, listener, load_env_variables, pricing, state, strategy,
    ws_url,
};
use spl_token::state::Account as TokenAccount;
use std::{
    fmt,
    str::FromStr,
    time::{Duration, Instant},
};

/// Skip trading if no account update has arrived recently (e.g. a stalled websocket).
const MAX_STATE_AGE_SECS: u64 = 30;
const SIM_COOLDOWN: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Metrics {
    events: u64,
    opportunities: u64,
    simulated: u64,
    sim_ok: u64,
}

impl fmt::Display for Metrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "slots {} opportunities {} simulated {} ok {}",
            self.events, self.opportunities, self.simulated, self.sim_ok
        )
    }
}

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
    let cfg = RpcTransactionLogsConfig {
        commitment: Some(CommitmentConfig::processed()),
    };
    let (mut log_stream, _unsub) = logs_client.logs_subscribe(filter, cfg).await?;
    println!("Listening for Raydium pool activity...");

    let cfg = strategy::StrategyConfig::default();
    let jito = JitoClient::new();
    let mut m = Metrics::default();
    let mut last_slot = 0u64;
    let mut last_sim = Instant::now() - SIM_COOLDOWN;

    while let Some(event) = log_stream.next().await {
        // One evaluation per slot is plenty; the state only changes when accounts update.
        let slot = event.context.slot;
        if slot == last_slot {
            continue;
        }
        last_slot = slot;
        m.events += 1;

        let snap = state::snapshot(&shared)?;
        if !snap.is_ready() || snap.is_stale(MAX_STATE_AGE_SECS) {
            continue;
        }
        if m.events % 50 == 1 {
            if let Some(s) =
                pricing::spread_from_state(snap.ray_sol, snap.ray_usdc, snap.orca_sqrt_price)
            {
                println!(
                    "ray {:.4}  orca {:.4}  spread {:.2} bps | {m}",
                    s.ray_price, s.orca_price, s.bps
                );
            }
        }

        let Some(o) = strategy::evaluate(&snap, &cfg) else {
            continue;
        };
        m.opportunities += 1;
        println!(
            "OPPORTUNITY slot {slot} {:?}: in {:.4} SOL, net {:.6} SOL",
            o.direction,
            o.amount_in as f64 / 1e9,
            o.net_profit as f64 / 1e9
        );

        if last_sim.elapsed() < SIM_COOLDOWN {
            continue;
        }
        last_sim = Instant::now();

        let built = async {
            let ixs = executor::build_arb_instructions(&snap, &o, &cfg, &payer, &keys, &jito)?;
            let tx = executor::compile_tx(&payer, &ixs, &[], rpc.get_latest_blockhash().await?)?;
            executor::simulate(&rpc, &tx).await
        }
        .await;
        m.simulated += 1;
        match built {
            Ok((None, _)) => {
                m.sim_ok += 1;
                println!("   simulation OK (live sending is disabled)");
            }
            Ok((Some(err), _)) => println!("   simulation rejected: {err}"),
            Err(e) => println!("   simulation failed: {e:#}"),
        }
    }
    Ok(())
}
