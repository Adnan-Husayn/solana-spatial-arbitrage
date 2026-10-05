use crate::config::*;
use crate::instructions::{
    OrcaSwapAccounts, build_orca_swap_instruction, build_raydium_swap_instruction,
    get_associated_token_address,
};
use crate::jito::JitoClient;
use crate::pricing::Direction;
use crate::state::MarketState;
use crate::strategy::{Opportunity, StrategyConfig, first_leg_usdc};
use anyhow::{Result, anyhow};
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_compute_budget_interface::ComputeBudgetInstruction;
use solana_sdk::{
    hash::Hash,
    instruction::Instruction,
    message::{VersionedMessage, v0},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::{Transaction, VersionedTransaction},
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
    let vault_signer =
        Pubkey::create_program_address(&[market.as_ref(), &nonce.to_le_bytes()], &program)
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

/// Compute budget for the two swaps plus tip. Measured values are ~25k (Orca) and ~50k (Raydium).
const COMPUTE_UNIT_LIMIT: u32 = 400_000;
/// Slippage tolerance on the intermediate USDC amount the second leg spends.
const LEG_SLIPPAGE_BPS: u64 = 10;
pub const MAX_TX_SIZE: usize = 1232;

/// Builds `[ComputeBudget, Swap A, Swap B, Tip]` for an opportunity.
///
/// The second leg's minimum output is `amount_in + cost`, so the whole transaction reverts
/// on-chain unless it ends in SOL profit after the tip and fees. The wallet needs funded
/// WSOL and USDC token accounts; wrapping SOL is the caller's responsibility.
pub fn build_arb_instructions(
    st: &MarketState,
    opp: &Opportunity,
    cfg: &StrategyConfig,
    payer: &Keypair,
    keys: &MarketKeys,
    jito: &JitoClient,
) -> Result<Vec<Instruction>> {
    let owner = payer.pubkey();
    let wsol = get_associated_token_address(&owner, &Pubkey::from_str(SOL_MINT)?);
    let usdc = get_associated_token_address(&owner, &Pubkey::from_str(USDC_MINT)?);

    let usdc_mid = first_leg_usdc(st, opp.direction, opp.amount_in)
        .ok_or_else(|| anyhow!("first leg is no longer quotable"))?;
    let usdc_mid = (usdc_mid as u128 * (10_000 - LEG_SLIPPAGE_BPS) as u128 / 10_000) as u64;
    let min_sol_back = opp.amount_in + opp.cost;

    let orca = OrcaSwapAccounts {
        whirlpool: Pubkey::from_str(ORCA_WHIRLPOOL)?,
        vault_a: Pubkey::from_str(ORCA_VAULT_A)?,
        vault_b: Pubkey::from_str(ORCA_VAULT_B)?,
        owner_account_a: wsol,
        owner_account_b: usdc,
        authority: owner,
        tick_current: st.orca_tick_index,
        tick_spacing: st.orca_tick_spacing,
    };
    let raydium =
        |amount_in: u64, min_out: u64, source: Pubkey, dest: Pubkey| -> Result<Instruction> {
            Ok(build_raydium_swap_instruction(
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
                source,
                dest,
                owner,
                amount_in,
                min_out,
            ))
        };

    let (leg1, leg2) = match opp.direction {
        Direction::BuyRaydiumSellOrca => (
            build_orca_swap_instruction(&orca, opp.amount_in, usdc_mid, true),
            raydium(usdc_mid, min_sol_back, usdc, wsol)?,
        ),
        Direction::BuyOrcaSellRaydium => (
            raydium(opp.amount_in, usdc_mid, wsol, usdc)?,
            build_orca_swap_instruction(&orca, usdc_mid, min_sol_back, false),
        ),
    };

    Ok(vec![
        ComputeBudgetInstruction::set_compute_unit_limit(COMPUTE_UNIT_LIMIT),
        ComputeBudgetInstruction::set_compute_unit_price(0),
        leg1,
        leg2,
        jito.add_tip_instruction(payer, cfg.tip_lamports),
    ])
}

/// Compiles instructions into a signed v0 transaction. Pass lookup tables to shrink the account list.
pub fn compile_tx(
    payer: &Keypair,
    ixs: &[Instruction],
    lookup_tables: &[solana_message::AddressLookupTableAccount],
    blockhash: Hash,
) -> Result<VersionedTransaction> {
    let msg = v0::Message::try_compile(&payer.pubkey(), ixs, lookup_tables, blockhash)
        .map_err(|e| anyhow!("compile failed: {e}"))?;
    VersionedTransaction::try_new(VersionedMessage::V0(msg), &[payer])
        .map_err(|e| anyhow!("sign failed: {e}"))
}

pub fn tx_size(tx: &VersionedTransaction) -> Result<usize> {
    Ok(bincode::serialize(tx)?.len())
}

/// Simulates a versioned transaction. Returns the error (if any), and the program logs.
pub async fn simulate(
    rpc: &RpcClient,
    tx: &VersionedTransaction,
) -> Result<(Option<String>, Vec<String>)> {
    let res = rpc.simulate_transaction(tx).await?;
    Ok((
        res.value.err.map(|e| format!("{e:?}")),
        res.value.logs.unwrap_or_default(),
    ))
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

    fn fixture_opportunity() -> (MarketState, Opportunity, StrategyConfig) {
        let st = crate::orca::fixtures::snapshot();
        let cfg = StrategyConfig::default();
        let opp = Opportunity {
            direction: Direction::BuyRaydiumSellOrca,
            amount_in: 1_000_000_000,
            expected_out: 1_001_000_000,
            gross_profit: 1_000_000,
            cost: cfg.fixed_cost(),
            net_profit: 1_000_000 - cfg.fixed_cost() as i64,
        };
        (st, opp, cfg)
    }

    fn dummy_keys() -> MarketKeys {
        let u = Pubkey::new_unique;
        MarketKeys {
            market: u(),
            event_queue: u(),
            bids: u(),
            asks: u(),
            coin_vault: u(),
            pc_vault: u(),
            vault_signer: u(),
        }
    }

    #[test]
    fn arb_transaction_has_expected_shape() {
        let (st, opp, cfg) = fixture_opportunity();
        let payer = Keypair::new();
        let ixs =
            build_arb_instructions(&st, &opp, &cfg, &payer, &dummy_keys(), &JitoClient::new())
                .unwrap();
        assert_eq!(ixs.len(), 5);
        // Last instruction is the tip transfer; second swap enforces profit on-chain.
        assert_eq!(ixs[4].program_id, solana_sdk_ids::system_program::id());
        let min_out = u64::from_le_bytes(ixs[3].data[9..17].try_into().unwrap());
        assert_eq!(min_out, opp.amount_in + opp.cost);
    }

    #[test]
    fn both_directions_build() {
        let (st, mut opp, cfg) = fixture_opportunity();
        opp.direction = Direction::BuyOrcaSellRaydium;
        let payer = Keypair::new();
        let ixs =
            build_arb_instructions(&st, &opp, &cfg, &payer, &dummy_keys(), &JitoClient::new())
                .unwrap();
        // Orca is the second leg here: min_out sits at data[16..24].
        let min_out = u64::from_le_bytes(ixs[3].data[16..24].try_into().unwrap());
        assert_eq!(min_out, opp.amount_in + opp.cost);
    }

    #[test]
    fn tx_size_without_lookup_table_is_reported() {
        let (st, opp, cfg) = fixture_opportunity();
        let payer = Keypair::new();
        let ixs =
            build_arb_instructions(&st, &opp, &cfg, &payer, &dummy_keys(), &JitoClient::new())
                .unwrap();
        let tx = compile_tx(&payer, &ixs, &[], Hash::default()).unwrap();
        let size = tx_size(&tx).unwrap();
        println!("tx size without ALT: {size} / {MAX_TX_SIZE}");
        assert!(size > 0);
    }
}
