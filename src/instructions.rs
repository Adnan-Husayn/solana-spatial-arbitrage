use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    system_program,
};
use std::mem::size_of;
use std::str::FromStr;


const RAYDIUM_V4_PROGRAM_ID: &str = "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8";


const ORCA_WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";


pub fn get_associated_token_address(
    wallet_address: &Pubkey,
    token_mint_address: &Pubkey,
) -> Pubkey {
    let program_id = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
    let associated_token_program_id = Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL").unwrap();

    let (address, _) = Pubkey::find_program_address(
        &[
            &wallet_address.to_bytes(),
            &program_id.to_bytes(),
            &token_mint_address.to_bytes(),
        ],
        &associated_token_program_id,
    );
    address
}

pub fn build_raydium_swap_instruction(
    pool_id: Pubkey,
    amm_authority: Pubkey,
    amm_open_orders: Pubkey,
    amm_target_orders: Pubkey,
    amm_coin_vault: Pubkey,
    amm_pc_vault: Pubkey,
    market_program: Pubkey,
    market_id: Pubkey,
    market_bids: Pubkey,
    market_asks: Pubkey,
    market_event_queue: Pubkey,
    market_coin_vault: Pubkey,
    market_pc_vault: Pubkey,
    market_vault_signer: Pubkey,
    user_source_token: Pubkey,  
    user_dest_token: Pubkey,    
    user_owner: Pubkey,         
    amount_in: u64,
    min_amount_out: u64,
) -> Instruction {
    let program_id = Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID).unwrap();

    
    let mut data = Vec::with_capacity(1 + 8 + 8);
    data.push(9); 
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());

    
    let accounts = vec![
        
        AccountMeta::new_readonly(Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap(), false),
        
        
        AccountMeta::new(pool_id, false),             
        AccountMeta::new_readonly(amm_authority, false),
        AccountMeta::new(amm_open_orders, false),     
        AccountMeta::new(amm_target_orders, false),   
        AccountMeta::new(amm_coin_vault, false),      
        AccountMeta::new(amm_pc_vault, false),        
        
        
        AccountMeta::new_readonly(market_program, false),
        AccountMeta::new(market_id, false),           
        AccountMeta::new(market_bids, false),         
        AccountMeta::new(market_asks, false),         
        AccountMeta::new(market_event_queue, false),  
        AccountMeta::new(market_coin_vault, false),   
        AccountMeta::new(market_pc_vault, false),     
        AccountMeta::new_readonly(market_vault_signer, false),
        
        
        AccountMeta::new(user_source_token, false),   
        AccountMeta::new(user_dest_token, false),     
        AccountMeta::new_readonly(user_owner, true),  
    ];

    Instruction {
        program_id,
        accounts,
        data,
    }
}
