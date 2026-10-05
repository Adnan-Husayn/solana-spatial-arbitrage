use crate::config::*;
use crate::instructions::{build_raydium_swap_instruction, get_associated_token_address};
use anyhow::{Result, anyhow};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};
use std::str::FromStr;

/// OpenBook market accounts the Raydium V4 swap instruction needs.
#[derive(Debug, Clone, Copy)]
pub struct MarketKeys {
    pub market: Pubkey,
    pub event_queue: Pubkey,
    pub bids: Pubkey,
    pub asks: Pubkey,
    pub coin_vault: Pubkey,
    pub pc_vault: Pubkey,
    pub vault_signer: Pubkey,
}

fn pk(data: &[u8], range: std::ops::Range<usize>) -> Result<Pubkey> {
    Ok(Pubkey::new_from_array(data[range].try_into()?))
}

pub async fn fetch_market_keys(rpc: &RpcClient) -> Result<MarketKeys> {
    let market = Pubkey::from_str(OB_MARKET_ID)?;
    let program = Pubkey::from_str(OB_PROG_ID)?;
    let data = rpc.get_account(&market).await?.data;
    if data.len() < 352 {
        return Err(anyhow!("OpenBook market account too small: {}", data.len()));
    }
    // vault_signer_nonce: u64 at offset 45; the signer is a PDA of [market, nonce].
    let nonce = u64::from_le_bytes(data[45..53].try_into()?);
    let vault_signer = Pubkey::create_program_address(&[market.as_ref(), &nonce.to_le_bytes()], &program)
        .map_err(|e| anyhow!("vault signer derivation failed: {e}"))?;

    Ok(MarketKeys {
        market,
        event_queue: pk(&data, 256..288)?,
        bids: pk(&data, 288..320)?,
        asks: pk(&data, 320..352)?,
        coin_vault: pk(&data, 160..192)?,
        pc_vault: pk(&data, 192..224)?,
        vault_signer,
    })
}

/// Builds a Raydium SOL->USDC swap and simulates it against mainnet. Returns the sim error, if any.
pub async fn simulate_raydium_swap(
    rpc: &RpcClient,
    payer: &Keypair,
    keys: &MarketKeys,
    amount_in_lamports: u64,
) -> Result<Option<String>> {
    let sol = Pubkey::from_str(SOL_MINT)?;
    let usdc = Pubkey::from_str(USDC_MINT)?;
    let ix = build_raydium_swap_instruction(
        Pubkey::from_str(RAY_POOL)?,
        Pubkey::from_str(RAY_AUTH)?,
        Pubkey::from_str(RAY_OPEN_ORDERS)?,
        Pubkey::from_str(RAY_TARGET_ORDERS)?,
        Pubkey::from_str(RAY_COIN_VAULT)?,
        Pubkey::from_str(RAY_PC_VAULT)?,
        Pubkey::from_str(OB_PROG_ID)?,
        keys.market,
        keys.bids,
        keys.asks,
        keys.event_queue,
        keys.coin_vault,
        keys.pc_vault,
        keys.vault_signer,
        get_associated_token_address(&payer.pubkey(), &sol),
        get_associated_token_address(&payer.pubkey(), &usdc),
        payer.pubkey(),
        amount_in_lamports,
        1,
    );
    let blockhash = rpc.get_latest_blockhash().await?;
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&payer.pubkey()), &[payer], blockhash);
    let res = rpc.simulate_transaction(&tx).await?;
    Ok(res.value.err.map(|e| format!("{e:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raydium_authority_is_the_amm_authority_pda() {
        let program = Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8").unwrap();
        let (pda, _) = Pubkey::find_program_address(&[b"amm authority"], &program);
        assert_eq!(pda.to_string(), RAY_AUTH);
    }
}
