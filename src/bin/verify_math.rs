use anyhow::Result;
use std::io::{self, Write};

// PASTE YOUR MATH FUNCTION HERE (Identical to src/math.rs)
pub const FEE_NUMERATOR: u128 = 25;
pub const FEE_DENOMINATOR: u128 = 10000;

pub fn calculate_swap_out(amount_in: u64, reserve_in: u64, reserve_out: u64) -> Result<u64> {
    if amount_in == 0 { return Ok(0); }
    let amount_in_u128 = amount_in as u128;
    let reserve_in_u128 = reserve_in as u128;
    let reserve_out_u128 = reserve_out as u128;
    
    let effective_amount_in = amount_in_u128 * (FEE_DENOMINATOR - FEE_NUMERATOR);
    let numerator = effective_amount_in * reserve_out_u128;
    let denominator = (reserve_in_u128 * FEE_DENOMINATOR) + effective_amount_in;
    
    Ok((numerator / denominator) as u64)
}

fn get_input(prompt: &str) -> u64 {
    print!("{}", prompt);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    // Handle decimals if user pastes "150.5" -> this is a raw tool, expect raw integers (lamports)
    // or we can allow float inputs. Let's ask for Raw Integers to be precise.
    input.trim().parse::<u64>().expect("Please enter a valid u64 integer")
}

fn main() -> Result<()> {
    println!("🧮 MATH VALIDATION HARNESS");
    println!("--------------------------");
    println!("Instructions:");
    println!("1. Open Solscan.io");
    println!("2. Find a recent transaction on the SOL/USDC Pool");
    println!("3. Look at 'Token Balance Change' for the Vaults");
    println!("4. Identify: Input Amount, Pool Reserve Before, Output Amount");
    println!("--------------------------\n");

    let reserve_in = get_input("1. Reserve IN (Liquidity BEFORE trade): ");
    let reserve_out = get_input("2. Reserve OUT (Liquidity BEFORE trade): ");
    let amount_in = get_input("3. Amount IN (How much user put in): ");
    let actual_out = get_input("4. Actual Amount OUT (From Solscan): ");

    println!("\n🤖 Computing...");
    let computed_out = calculate_swap_out(amount_in, reserve_in, reserve_out)?;

    println!("--------------------------");
    println!("Actual On-Chain: {}", actual_out);
    println!("Computed Local:  {}", computed_out);
    let diff = (actual_out as i128 - computed_out as i128).abs();
    println!("Difference:      {}", diff);

    if diff == 0 {
        println!("✅ PERFECT MATCH");
    } else if diff < 1000 {
        println!("⚠️  CLOSE MATCH (Difference likely due to Reserve snapshot timing)");
    } else {
        println!("❌ MISMATCH - Check your logic or Reserve values");
    }

    Ok(())
}