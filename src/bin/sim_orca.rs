//! Simulates an Orca-only USDC -> SOL swap against mainnet to validate the instruction layout.
//! Uses the Raydium USDC vault and its owner as source and authority with signature verification off, so no
//! wallet or funds are needed. Nothing is ever sent.

use solana_client::{rpc_client::RpcClient, rpc_config::RpcSimulateTransactionConfig};
use solana_sdk::{
    commitment_config::CommitmentConfig, message::Message, program_pack::Pack, pubkey::Pubkey,
    transaction::Transaction,
};
use spatial_arbitrage_bot::{config::*, instructions::*, state::parse_whirlpool};
use spl_token::state::Account as TokenAccount;
use std::str::FromStr;

fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    let rpc_url =
        std::env::var("RPC_URL").unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into());
    let rpc = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

    let pool = Pubkey::from_str(ORCA_WHIRLPOOL)?;
    let pool_state = parse_whirlpool(&rpc.get_account(&pool)?.data)?;

    // A funded fee payer that never signs (simulation only), and a USDC account to spend from.
    let fee_payer = Pubkey::from_str("5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9")?;
    let wsol_dest = Pubkey::from_str(RAY_COIN_VAULT)?; // any WSOL token account works as a sink

    // Raydium's USDC vault is a large, known USDC account; its owner is the Raydium authority.
    let usdc_acc = Pubkey::from_str(RAY_PC_VAULT)?;
    let owner = TokenAccount::unpack(&rpc.get_account(&usdc_acc)?.data)?.owner;

    let ix = build_orca_swap_instruction(
        &OrcaSwapAccounts {
            whirlpool: pool,
            vault_a: Pubkey::from_str(ORCA_VAULT_A)?,
            vault_b: Pubkey::from_str(ORCA_VAULT_B)?,
            owner_account_a: wsol_dest,
            owner_account_b: usdc_acc,
            authority: owner,
            tick_current: pool_state.tick_index,
            tick_spacing: pool_state.tick_spacing,
        },
        100_000_000, // 100 USDC
        1,
        false,
    );
    let tx = Transaction::new_unsigned(Message::new(&[ix], Some(&fee_payer)));
    let res = rpc.simulate_transaction_with_config(
        &tx,
        RpcSimulateTransactionConfig {
            sig_verify: false,
            replace_recent_blockhash: true,
            commitment: Some(CommitmentConfig::confirmed()),
            ..Default::default()
        },
    )?;
    println!(
        "err={:?} units={:?}",
        res.value.err, res.value.units_consumed
    );
    for l in res.value.logs.unwrap_or_default() {
        println!("  {l}");
    }
    if res.value.err.is_none() {
        println!("OK: Orca swap instruction is valid against mainnet");
        return Ok(());
    }
    anyhow::bail!("simulation failed")
}
