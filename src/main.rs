use anyhow::Result;
use futures::StreamExt;
use solana_client::{
    nonblocking::{pubsub_client::PubsubClient, rpc_client::RpcClient},
    rpc_config::{RpcTransactionLogsConfig, RpcTransactionLogsFilter},
};
use solana_commitment_config::CommitmentConfig;
use solana_sdk::{pubkey::Pubkey, signature::Signer};
use spatial_arbitrage_bot::{
    config::*,
    executor,
    jito::{BundleOutcome, JitoClient},
    listener, load_env_variables, pricing,
    risk::{RiskConfig, RiskState},
    state, strategy, ws_url,
};
use std::{
    fmt,
    str::FromStr,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Skip trading if no account update has arrived recently (e.g. a stalled websocket).
const MAX_STATE_AGE_SECS: u64 = 30;
const SIM_COOLDOWN: Duration = Duration::from_secs(2);
/// Pause after submitting a live bundle, so state and balances settle.
const SEND_COOLDOWN: Duration = Duration::from_secs(5);
const BUNDLE_TIMEOUT: Duration = Duration::from_secs(30);

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[derive(Default)]
struct Metrics {
    events: u64,
    opportunities: u64,
    simulated: u64,
    sim_ok: u64,
    sent: u64,
    landed: u64,
}

impl fmt::Display for Metrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "slots {} opportunities {} simulated {} ok {} sent {} landed {}",
            self.events, self.opportunities, self.simulated, self.sim_ok, self.sent, self.landed
        )
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let (rpc_url, payer) = load_env_variables()?;
    let ws = ws_url(&rpc_url);
    let rpc = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
    let risk_cfg = RiskConfig::from_env()?;

    if risk_cfg.live {
        tracing::warn!(
            "LIVE MODE: real bundles will be sent. limits: {} lamports/trade, {} lamports/day loss, {} lamports min balance, kill file {:?}",
            risk_cfg.max_trade_lamports,
            risk_cfg.max_daily_loss_lamports,
            risk_cfg.min_balance_lamports,
            risk_cfg.kill_file
        );
    } else {
        tracing::info!("bot active (simulation mode, set LIVE=true to send)");
    }
    tracing::info!("wallet: {}", payer.pubkey());

    let keys = executor::fetch_market_keys(&rpc).await?;
    tracing::info!("openbook market keys fetched");

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
        state::set_ray_sol(&shared, state::token_amount(&s.data)?)?;
        state::set_ray_usdc(&shared, state::token_amount(&u.data)?)?;
        state::set_orca(&shared, &o.data)?;
        tracing::info!("bootstrap complete");
    }

    tokio::spawn(listener::run(ws.clone(), shared.clone()));

    let logs_client = PubsubClient::new(&ws).await?;
    let filter = RpcTransactionLogsFilter::Mentions(vec![RAY_POOL.to_string()]);
    let cfg = RpcTransactionLogsConfig {
        commitment: Some(CommitmentConfig::processed()),
    };
    let (mut log_stream, _unsub) = logs_client.logs_subscribe(filter, cfg).await?;
    tracing::info!("listening for raydium pool activity");

    let cfg = strategy::StrategyConfig::from_env()?;
    tracing::info!("strategy config: {cfg:?}");
    let jito = match std::env::var("JITO_URL") {
        Ok(url) => JitoClient::with_url(url),
        Err(_) => JitoClient::new(),
    };
    let mut risk = RiskState::new();
    let mut last_send = Instant::now() - SEND_COOLDOWN;
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
        if m.events % 50 == 1
            && let Some(s) =
                pricing::spread_from_state(snap.ray_sol, snap.ray_usdc, snap.orca_sqrt_price)
        {
            tracing::info!(
                "ray {:.4}  orca {:.4}  spread {:.2} bps | {m}",
                s.ray_price,
                s.orca_price,
                s.bps
            );
        }

        let Some(o) = strategy::evaluate(&snap, &cfg) else {
            continue;
        };
        m.opportunities += 1;
        tracing::info!(
            "opportunity slot {slot} {:?}: in {:.4} SOL, net {:.6} SOL",
            o.direction,
            o.amount_in as f64 / 1e9,
            o.net_profit as f64 / 1e9
        );

        if risk_cfg.live {
            if last_send.elapsed() < SEND_COOLDOWN {
                continue;
            }
            if let Err(e) = try_live(
                &rpc, &jito, &payer, &keys, &snap, &cfg, &risk_cfg, &mut risk, &mut m,
            )
            .await
            {
                tracing::warn!("live attempt failed: {e:#}");
            }
            last_send = Instant::now();
            continue;
        }

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
                tracing::info!("simulation OK (live sending is disabled)");
            }
            Ok((Some(err), _)) => tracing::info!("simulation rejected: {err}"),
            Err(e) => tracing::warn!("simulation failed: {e:#}"),
        }
    }
    Ok(())
}

/// One guarded live attempt: re-quote against real balances, check limits, send, confirm, record PnL.
#[allow(clippy::too_many_arguments)]
async fn try_live(
    rpc: &RpcClient,
    jito: &JitoClient,
    payer: &solana_sdk::signature::Keypair,
    keys: &executor::MarketKeys,
    snap: &state::MarketState,
    cfg: &strategy::StrategyConfig,
    risk_cfg: &RiskConfig,
    risk: &mut RiskState,
    m: &mut Metrics,
) -> Result<()> {
    let owner = payer.pubkey();
    let before = executor::fetch_balances(rpc, &owner).await?;
    let (Some(wsol), Some(_)) = (before.wsol, before.usdc) else {
        anyhow::bail!("wallet needs both a WSOL and a USDC token account");
    };

    // Re-evaluate with the size clamped to what the wallet and risk limits allow.
    let mut live_cfg = *cfg;
    live_cfg.max_trade_lamports = cfg
        .max_trade_lamports
        .min(risk_cfg.max_trade_lamports)
        .min(wsol);
    let Some(opp) = strategy::evaluate(snap, &live_cfg) else {
        tracing::info!("no profitable trade within wallet and risk limits");
        return Ok(());
    };

    let now = now_unix();
    let kill = risk_cfg.kill_file.exists();
    if let Err(reject) = risk.check(risk_cfg, now, kill, before.native, &opp) {
        tracing::warn!("trade blocked: {reject}");
        return Ok(());
    }

    let price = pricing::raydium_price(snap.ray_sol, snap.ray_usdc).unwrap_or(0.0);
    let value_before = before.total_lamports(price);

    let ixs = executor::build_arb_instructions(snap, &opp, &live_cfg, payer, keys, jito)?;
    let tx = executor::compile_tx(payer, &ixs, &[], rpc.get_latest_blockhash().await?)?;
    if executor::tx_size(&tx)? > executor::MAX_TX_SIZE {
        anyhow::bail!("transaction exceeds the size limit");
    }

    // Last line of defence: never send something the simulator rejects.
    let (sim_err, _) = executor::simulate(rpc, &tx).await?;
    if let Some(err) = sim_err {
        tracing::info!("pre-send simulation rejected, not sending: {err}");
        return Ok(());
    }

    tracing::warn!(
        "SENDING {:?}: in {:.4} SOL, expected net {:.6} SOL",
        opp.direction,
        opp.amount_in as f64 / 1e9,
        opp.net_profit as f64 / 1e9
    );
    let id = jito.send_bundle(vec![tx]).await?;
    m.sent += 1;

    match jito.wait_for_bundle(&id, BUNDLE_TIMEOUT).await? {
        BundleOutcome::Landed { slot } => {
            m.landed += 1;
            tokio::time::sleep(Duration::from_secs(2)).await;
            let after = executor::fetch_balances(rpc, &owner).await?;
            let delta = after.total_lamports(price) - value_before;
            risk.record_pnl(now_unix(), delta);
            tracing::warn!(
                "bundle landed in slot {slot}: pnl {:+.6} SOL, day total {:+.6} SOL",
                delta as f64 / 1e9,
                risk.pnl(now_unix()) as f64 / 1e9
            );
        }
        other => tracing::info!(
            "bundle {id} did not land: {other:?} (no cost; tip is only paid on landing)"
        ),
    }
    Ok(())
}
