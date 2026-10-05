use crate::config::*;
use crate::state::{self, SharedState};
use anyhow::{Result, anyhow};
use futures::{StreamExt, stream};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{nonblocking::pubsub_client::PubsubClient, rpc_config::RpcAccountInfoConfig};
use solana_sdk::{commitment_config::CommitmentConfig, program_pack::Pack, pubkey::Pubkey};
use spl_token::state::Account as TokenAccount;
use std::{str::FromStr, time::Duration};

#[derive(Clone, Copy, Debug)]
enum Feed {
    RaySol,
    RayUsdc,
    Orca,
}

/// Streams account updates into `state`. Reconnects with backoff; never returns.
pub async fn run(ws_url: String, state: SharedState) {
    let mut backoff = Duration::from_secs(1);
    loop {
        match run_once(&ws_url, &state).await {
            Ok(()) => eprintln!("listener: stream ended, reconnecting"),
            Err(e) => eprintln!("listener: {e:#}, reconnecting in {backoff:?}"),
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

async fn run_once(ws_url: &str, state: &SharedState) -> Result<()> {
    let client = PubsubClient::new(ws_url).await?;
    let cfg = RpcAccountInfoConfig {
        encoding: Some(UiAccountEncoding::Base64),
        commitment: Some(CommitmentConfig::confirmed()),
        ..RpcAccountInfoConfig::default()
    };

    let sol = Pubkey::from_str(RAY_COIN_VAULT)?;
    let usdc = Pubkey::from_str(RAY_PC_VAULT)?;
    let orca = Pubkey::from_str(ORCA_WHIRLPOOL)?;

    let (s_s, _u1) = client.account_subscribe(&sol, Some(cfg.clone())).await?;
    let (u_s, _u2) = client.account_subscribe(&usdc, Some(cfg.clone())).await?;
    let (o_s, _u3) = client.account_subscribe(&orca, Some(cfg)).await?;

    let mut combined = stream::select_all(vec![
        s_s.map(|x| (Feed::RaySol, x)).boxed(),
        u_s.map(|x| (Feed::RayUsdc, x)).boxed(),
        o_s.map(|x| (Feed::Orca, x)).boxed(),
    ]);

    while let Some((feed, msg)) = combined.next().await {
        let Some(data) = msg.value.data.decode() else { continue };
        if let Err(e) = apply(feed, &data, state) {
            eprintln!("listener: bad {feed:?} update: {e:#}");
        }
    }
    Err(anyhow!("subscription closed"))
}

fn apply(feed: Feed, data: &[u8], st: &SharedState) -> Result<()> {
    match feed {
        Feed::Orca => state::set_orca(st, data),
        Feed::RaySol => state::set_ray_sol(st, TokenAccount::unpack(data)?.amount),
        Feed::RayUsdc => state::set_ray_usdc(st, TokenAccount::unpack(data)?.amount),
    }
}
