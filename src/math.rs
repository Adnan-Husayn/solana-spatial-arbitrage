use anyhow::{Result, anyhow};

pub const FEE_NUMERATOR: u128 = 25;
pub const FEE_DENOMINATOR: u128 = 10000;

pub fn calculate_swap_out(amount_in: u64, reserve_in: u64, reserve_out: u64) -> Result<u64> {
    if amount_in == 0 {
        return Ok(0);
    }

    if reserve_in == 0 || reserve_out == 0 {
        return Err(anyhow!("Liquidity reserves cannot be zero"));
    }

    let amount_in_u128 = amount_in as u128;
    let reserve_in_u128 = reserve_in as u128;
    let reserve_out_u128 = reserve_out as u128;

    let effective_amount_in = amount_in_u128
        .checked_mul(FEE_DENOMINATOR - FEE_NUMERATOR)
        .ok_or(anyhow!("Math Overflow: Effective Input"))?;

    let numerator = effective_amount_in
        .checked_mul(reserve_out_u128)
        .ok_or(anyhow!("Math Overflow: Numerator"))?;

    let den_part_1 = reserve_in_u128
        .checked_mul(FEE_DENOMINATOR)
        .ok_or(anyhow!("Math Overflow: Denom Part 1"))?;

    let denominator = den_part_1
        .checked_add(effective_amount_in)
        .ok_or(anyhow!("Math Overflow: Denom Total"))?;

    let amount_out = numerator
        .checked_div(denominator)
        .ok_or(anyhow!("Math Division Error"))?;

    if amount_out > u64::MAX as u128 {
        return Err(anyhow!("Math Overflow: Result too large for u64"));
    }

    Ok(amount_out as u64)
}
