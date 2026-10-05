use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    commitment_config::CommitmentConfig,
};
use std::str::FromStr;


const RAYDIUM_V4_POOL: &str = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";
const ORCA_WHIRLPOOL: &str = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    
    dotenv::dotenv().ok();
    let rpc_url = std::env::var("RPC_URL")?;
    println!("Connecting to RPC...");
    
    let client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
    
    
    let ray_pool_pk = Pubkey::from_str(RAYDIUM_V4_POOL)?;
    let account = client.get_account(&ray_pool_pk)?;
    let data = account.data;

    
    
    let market_id = Pubkey::new_from_array(data[528..560].try_into()?);
    let market_program_id = Pubkey::new_from_array(data[560..592].try_into()?);
    
    
    let amm_open_orders = Pubkey::new_from_array(data[496..528].try_into()?);
    
    let amm_target_orders = Pubkey::new_from_array(data[592..624].try_into()?);
    
    

    println!("RAYDIUM POOL FOUND");
    println!("   Market ID:      {}", market_id);
    println!("   AMM OpenOrders: {}", amm_open_orders);
    println!("   AMM TargetOrd:  {}", amm_target_orders);

    
    println!("\nFetching OpenBook Market: {}", market_id);
    let market_acc = client.get_account(&market_id)?;
    let m_data = market_acc.data;
    
    
    
    let market_event_q = Pubkey::new_from_array(m_data[256..288].try_into()?);
    let market_bids = Pubkey::new_from_array(m_data[288..320].try_into()?);
    let market_asks = Pubkey::new_from_array(m_data[320..352].try_into()?);
    let market_coin_vault = Pubkey::new_from_array(m_data[160..192].try_into()?);
    let market_pc_vault = Pubkey::new_from_array(m_data[192..224].try_into()?);
    let market_vault_signer = Pubkey::new_from_array(m_data[224..256].try_into()?); 

    println!("   Market Event Q: {}", market_event_q);
    println!("   Market Bids:    {}", market_bids);
    println!("   Market Asks:    {}", market_asks);
    println!("   Market Coin V:  {}", market_coin_vault);
    println!("   Market PC V:    {}", market_pc_vault);
    
    
    
    
    println!("\nORCA POOL FOUND");
    let orca_pk = Pubkey::from_str(ORCA_WHIRLPOOL)?;
    let orca_acc = client.get_account(&orca_pk)?;
    let o_data = orca_acc.data;
    let tick_spacing = u16::from_le_bytes(o_data[41..43].try_into()?);
    
    
    let orca_vault_a = Pubkey::new_from_array(o_data[133..165].try_into()?);
    let orca_vault_b = Pubkey::new_from_array(o_data[213..245].try_into()?);

    println!("   Tick Spacing:   {}", tick_spacing);
    println!("   Vault A (SOL):  {}", orca_vault_a);
    println!("   Vault B (USDC): {}", orca_vault_b);
    println!("   (oracle is a PDA: [\"oracle\", whirlpool], derived in Phase 3)");

    println!("\n------------------------------------------------");
    println!("SUCCESS! Copy these values.");
    
    Ok(())
}