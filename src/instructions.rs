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
    let associated_token_program_id =
        Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL").unwrap();

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
        AccountMeta::new_readonly(
            Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap(),
            false,
        ),
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

// ---------------------------------------------------------------------------
// Orca Whirlpool
// ---------------------------------------------------------------------------

/// Anchor discriminator for `swap`: first 8 bytes of sha256("global:swap").
pub const ORCA_SWAP_DISCRIMINATOR: [u8; 8] = [0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8];

pub const MIN_SQRT_PRICE_X64: u128 = 4_295_048_016;
pub const MAX_SQRT_PRICE_X64: u128 = 79_226_673_515_401_279_992_447_579_055;

const TICK_ARRAY_SIZE: i32 = 88;

/// First tick index of the tick array containing `tick`.
pub fn tick_array_start_index(tick: i32, tick_spacing: u16) -> i32 {
    let ticks_per_array = tick_spacing as i32 * TICK_ARRAY_SIZE;
    tick.div_euclid(ticks_per_array) * ticks_per_array
}

pub fn tick_array_pda(whirlpool: &Pubkey, start_index: i32, program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            b"tick_array",
            whirlpool.as_ref(),
            start_index.to_string().as_bytes(),
        ],
        program_id,
    )
    .0
}

pub fn oracle_pda(whirlpool: &Pubkey, program_id: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[b"oracle", whirlpool.as_ref()], program_id).0
}

/// The three tick arrays a swap may traverse, in the order the program expects.
/// Selling token A (price falls) walks down; selling token B walks up.
pub fn swap_tick_arrays(
    whirlpool: &Pubkey,
    tick_current: i32,
    tick_spacing: u16,
    a_to_b: bool,
    program_id: &Pubkey,
) -> [Pubkey; 3] {
    let step = tick_spacing as i32 * TICK_ARRAY_SIZE;
    // Swapping up, the program looks one spacing ahead so the boundary tick isn't missed.
    let anchor = if a_to_b {
        tick_current
    } else {
        tick_current + tick_spacing as i32
    };
    let start = tick_array_start_index(anchor, tick_spacing);
    let dir = if a_to_b { -1 } else { 1 };
    [0, 1, 2].map(|i| tick_array_pda(whirlpool, start + dir * i * step, program_id))
}

pub struct OrcaSwapAccounts {
    pub whirlpool: Pubkey,
    pub vault_a: Pubkey,
    pub vault_b: Pubkey,
    pub owner_account_a: Pubkey,
    pub owner_account_b: Pubkey,
    pub authority: Pubkey,
    pub tick_current: i32,
    pub tick_spacing: u16,
}

/// Exact-input swap. `a_to_b = true` sells token A (SOL) for token B (USDC).
pub fn build_orca_swap_instruction(
    a: &OrcaSwapAccounts,
    amount_in: u64,
    min_amount_out: u64,
    a_to_b: bool,
) -> Instruction {
    let program_id = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap();
    let token_program = Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap();
    let [t0, t1, t2] = swap_tick_arrays(
        &a.whirlpool,
        a.tick_current,
        a.tick_spacing,
        a_to_b,
        &program_id,
    );

    let sqrt_price_limit = if a_to_b {
        MIN_SQRT_PRICE_X64
    } else {
        MAX_SQRT_PRICE_X64
    };

    let mut data = Vec::with_capacity(8 + 8 + 8 + 16 + 1 + 1);
    data.extend_from_slice(&ORCA_SWAP_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit.to_le_bytes());
    data.push(1); // amount_specified_is_input
    data.push(a_to_b as u8);

    let accounts = vec![
        AccountMeta::new_readonly(token_program, false),
        AccountMeta::new_readonly(a.authority, true),
        AccountMeta::new(a.whirlpool, false),
        AccountMeta::new(a.owner_account_a, false),
        AccountMeta::new(a.vault_a, false),
        AccountMeta::new(a.owner_account_b, false),
        AccountMeta::new(a.vault_b, false),
        AccountMeta::new(t0, false),
        AccountMeta::new(t1, false),
        AccountMeta::new(t2, false),
        AccountMeta::new_readonly(oracle_pda(&a.whirlpool, &program_id), false),
    ];

    Instruction {
        program_id,
        accounts,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_sdk::hash::hash;

    #[test]
    fn swap_discriminator_is_sha256_prefix() {
        assert_eq!(
            hash(b"global:swap").to_bytes()[..8],
            ORCA_SWAP_DISCRIMINATOR
        );
    }

    #[test]
    fn tick_array_start_handles_negative_ticks() {
        // spacing 4 => 352 ticks per array
        assert_eq!(tick_array_start_index(0, 4), 0);
        assert_eq!(tick_array_start_index(351, 4), 0);
        assert_eq!(tick_array_start_index(352, 4), 352);
        assert_eq!(tick_array_start_index(-1, 4), -352);
        assert_eq!(tick_array_start_index(-21094, 4), -21120);
    }

    #[test]
    fn tick_arrays_walk_in_swap_direction() {
        let pool = Pubkey::new_unique();
        let prog = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap();
        let down = swap_tick_arrays(&pool, -21094, 4, true, &prog);
        assert_eq!(down[0], tick_array_pda(&pool, -21120, &prog));
        assert_eq!(down[1], tick_array_pda(&pool, -21472, &prog));
        assert_eq!(down[2], tick_array_pda(&pool, -21824, &prog));
        let up = swap_tick_arrays(&pool, -21094, 4, false, &prog);
        assert_eq!(up[1], tick_array_pda(&pool, -21120 + 352, &prog));
    }

    #[test]
    fn swap_instruction_shape() {
        let a = OrcaSwapAccounts {
            whirlpool: Pubkey::new_unique(),
            vault_a: Pubkey::new_unique(),
            vault_b: Pubkey::new_unique(),
            owner_account_a: Pubkey::new_unique(),
            owner_account_b: Pubkey::new_unique(),
            authority: Pubkey::new_unique(),
            tick_current: -21094,
            tick_spacing: 4,
        };
        let ix = build_orca_swap_instruction(&a, 1_000, 900, true);
        assert_eq!(ix.accounts.len(), 11);
        assert_eq!(ix.data.len(), 42);
        assert_eq!(ix.data[..8], ORCA_SWAP_DISCRIMINATOR);
        assert_eq!(
            u64::from_le_bytes(ix.data[8..16].try_into().unwrap()),
            1_000
        );
        assert_eq!(u64::from_le_bytes(ix.data[16..24].try_into().unwrap()), 900);
        assert_eq!(
            u128::from_le_bytes(ix.data[24..40].try_into().unwrap()),
            MIN_SQRT_PRICE_X64
        );
        assert_eq!(ix.data[40..], [1, 1]);
        assert!(ix.accounts[1].is_signer);
        let up = build_orca_swap_instruction(&a, 1_000, 900, false);
        assert_eq!(
            u128::from_le_bytes(up.data[24..40].try_into().unwrap()),
            MAX_SQRT_PRICE_X64
        );
        assert_eq!(up.data[41], 0);
    }
}
