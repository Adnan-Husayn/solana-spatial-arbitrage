use bytemuck::{Pod, Zeroable};
use solana_sdk::pubkey::Pubkey;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct AmmInfo {
    pub status: u64,
    pub nonce: u64,
    pub order_num: u64,
    pub depth: u64,
    pub coin_decimals: u64,
    pub pc_decimals: u64,
    pub state: u64,
    pub reset_flag: u64,
    pub min_size: u64,
    pub vol_max_cut_ratio: u64,
    pub amount_wave_ratio: u64,
    pub coin_lot_size: u64,
    pub pc_lot_size: u64,
    pub min_price_multiplier: u64,
    pub max_price_multiplier: u64,
    pub sys_decimal_value: u64,

    pub min_separate_numerator: u64,
    pub min_separate_denominator: u64,
    pub trade_fee_numerator: u64,
    pub trade_fee_denominator: u64,
    pub pnl_numerator: u64,
    pub pnl_denominator: u64,
    pub swap_fee_numerator: u64,
    pub swap_fee_denominator: u64,

    pub need_take_pnl_coin: u64,
    pub need_take_pnl_pc: u64,
    pub total_pnl_pc: u64,
    pub total_pnl_coin: u64,

    /// StateData and other fields between the header and the pubkeys (offsets 224..336).
    pub state_padding: [u64; 14],

    pub pool_coin_token_account: Pubkey,
    pub pool_pc_token_account: Pubkey,
    pub coin_mint_address: Pubkey,
    pub pc_mint_address: Pubkey,
    pub lp_mint_address: Pubkey,
    pub amm_open_orders: Pubkey,
    pub serum_market: Pubkey,
    pub serum_program_id: Pubkey,
    pub amm_target_orders: Pubkey,
    pub pool_withdraw_queue: Pubkey,
    pub pool_temp_lp_token_account: Pubkey,
    pub amm_owner: Pubkey,
    pub pnl_owner: Pubkey,
}

impl AmmInfo {
    pub fn from_bytes(data: &[u8]) -> Option<&Self> {
        if data.len() != 752 {
            return None;
        }
        bytemuck::try_from_bytes(data).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn layout_matches_mainnet() {
        assert_eq!(size_of::<AmmInfo>(), 752);
        assert_eq!(offset_of!(AmmInfo, pool_coin_token_account), 336);
        assert_eq!(offset_of!(AmmInfo, pool_pc_token_account), 368);
        assert_eq!(offset_of!(AmmInfo, amm_open_orders), 496);
        assert_eq!(offset_of!(AmmInfo, serum_market), 528);
        assert_eq!(offset_of!(AmmInfo, amm_target_orders), 592);
    }
}
