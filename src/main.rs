use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    pubkey::Pubkey,
    program_pack::Pack,
};
use spatial_arbitrage_bot::load_env_variables;
use spl_token::state::Account as TokenAccount;
use std::str::FromStr;


const OFFSET_STATUS: usize = 0;
const OFFSET_COIN_VAULT: usize = 336; 
const OFFSET_PC_VAULT: usize = 368;   
const OFFSET_SWAP_FEE_NUM: usize = 176;
const OFFSET_SWAP_FEE_DEN: usize = 184;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (rpc_url, _) = load_env_variables()?;
    let client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

    
    let pool_pubkey = Pubkey::from_str("58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2")?;
    let account = client.get_account(&pool_pubkey)?;

    
    let status = u64::from_le_bytes(account.data[OFFSET_STATUS..OFFSET_STATUS+8].try_into()?);
    let fee_num = u64::from_le_bytes(account.data[OFFSET_SWAP_FEE_NUM..OFFSET_SWAP_FEE_NUM+8].try_into()?);
    let fee_den = u64::from_le_bytes(account.data[OFFSET_SWAP_FEE_DEN..OFFSET_SWAP_FEE_DEN+8].try_into()?);
    
    let coin_vault = Pubkey::new_from_array(account.data[OFFSET_COIN_VAULT..OFFSET_COIN_VAULT+32].try_into()?);
    let pc_vault = Pubkey::new_from_array(account.data[OFFSET_PC_VAULT..OFFSET_PC_VAULT+32].try_into()?);

    println!("Config Loaded:");
    println!("   Status: {} (6=Active)", status);
    println!("   Fee:    {}/{} ({}%)", fee_num, fee_den, (fee_num as f64 / fee_den as f64) * 100.0);

    
    
    let vaults = client.get_multiple_accounts(&[coin_vault, pc_vault])?;
    
    let coin_data = vaults[0].as_ref().ok_or(anyhow::anyhow!("Base Vault not found"))?;
    let pc_data = vaults[1].as_ref().ok_or(anyhow::anyhow!("Quote Vault not found"))?;

    let coin_balance = TokenAccount::unpack(&coin_data.data)?.amount;
    let pc_balance = TokenAccount::unpack(&pc_data.data)?.amount;

    
    let sol_real = coin_balance as f64 / 1_000_000_000.0; 
    let usdc_real = pc_balance as f64 / 1_000_000.0;     
    let price = usdc_real / sol_real;

    println!("\nMARKET SNAPSHOT:");
    println!("   SOL Liquidity:  {:.2}", sol_real);
    println!("   USDC Liquidity: {:.2}", usdc_real);
    println!("   Current Price:  ${:.4}", price);
    println!("   K (Constant):   {:.0}", sol_real * usdc_real);

    Ok(())
}