use solana_client::nonblocking::rpc_client;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    message::Message,
    pubkey::Pubkey,
    signer::Signer,
    system_program,
    transaction::VersionedTransaction,
    message::v0::Message as TransactionMessage,
    message::VersionedMessage,
};
use std::str::FromStr;
use crate::dex::pump::PumpDex;
use anyhow::{anyhow, Result};
use std::io::{self, Write};
use std::time::Duration;
use tokio::time::sleep;
use std::path::Path;
use std::fs;
use solana_sdk::signature::Keypair;
use spl_associated_token_account::get_associated_token_address;
use bs58;
use spl_token::{
    instruction as token_instruction,
    state::Account as TokenAccount,
};
use spl_associated_token_account::{
    instruction::create_associated_token_account_idempotent,
};
use serde_json;

pub async fn dev_sell_token(client: &RpcClient, dex: &PumpDex) -> Result<()> {
    use std::env;
    dotenv::dotenv().ok();
    print!("Enter token mint address --> ");
    io::stdout().flush()?;
    let mut mint = String::new();
    io::stdin().read_line(&mut mint)?;
    let mint = mint.trim();
    let token_address = Pubkey::from_str(mint)?;

    // Load DEV keypair from .env
    let dev_privkey = env::var("DEV").map_err(|_| anyhow!("DEV not set in .env file"))?;
    let dev_bytes = bs58::decode(&dev_privkey).into_vec()?;
    let dev_keypair = Keypair::from_bytes(&dev_bytes)?;
    let dev_pubkey = dev_keypair.pubkey();

    // Get ATA
    let user_ata = get_associated_token_address(&dev_pubkey, &token_address);
    let account = client.get_token_account_balance(&user_ata)?;
    let balance = account.amount.parse::<u64>()?;
    if balance == 0 {
        return Err(anyhow!("DEV wallet has no token balance"));
    }
    let sell_amount = balance;

    // Get bonding curve and vaults
    let (bonding_curve, _) = Pubkey::find_program_address(
        &[b"bonding-curve", token_address.as_ref()],
        &dex.program_id,
    );
    let curve_info = client.get_account(&bonding_curve)?;
    let creator_pubkey = Pubkey::try_from(&curve_info.data[49..81])?;
    let (creator_vault, _) = Pubkey::find_program_address(
        &[b"creator-vault", creator_pubkey.as_ref()],
        &dex.program_id,
    );
    let a_bonding_curve = get_associated_token_address(&bonding_curve, &token_address);
    let reserve_a = u64::from_le_bytes(curve_info.data.get(81..89).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
    let reserve_b = u64::from_le_bytes(curve_info.data.get(89..97).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
    let (sol_to_receive, _, _) = dex.get_amount_out(
        sell_amount,
        reserve_a,
        reserve_b,
    );
    let mut fee_amount = (sol_to_receive * 100) / 10000;
    if fee_amount < 1_000 {
        fee_amount = 1_000;
    }
    let fee_recipient = Pubkey::from_str("FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT")?;

    // Build instructions
    let sell_instruction = dex.create_sell_instruction(
        &token_address,
        &bonding_curve,
        &a_bonding_curve,
        &user_ata,
        &dev_pubkey,
        &creator_vault,
        sell_amount.into(),
    );
    let fee_instruction = solana_sdk::system_instruction::transfer(
        &dev_pubkey,
        &fee_recipient,
        fee_amount,
    );
    let close_ata_ix = spl_token::instruction::close_account(
        &spl_token::id(),
        &user_ata,
        &dev_pubkey,
        &dev_pubkey,
        &[],
    )?;

    let blockhash = client.get_latest_blockhash()?;
    let message = TransactionMessage::try_compile(
        &dev_pubkey,
        &[sell_instruction, fee_instruction, close_ata_ix],
        &[],
        blockhash,
    )?;
    let transaction = VersionedTransaction::try_new(
        VersionedMessage::V0(message),
        &[&dev_keypair],
    )?;

    // let sim = client.simulate_transaction(&transaction);
    // println!("{:?}", sim);
    
    let signature = client.send_transaction_with_config(
        &transaction,
        solana_client::rpc_config::RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: Some(CommitmentConfig::processed().commitment),
            encoding: None,
            max_retries: Some(5),
            min_context_slot: None
        }
    )?;
    println!("Solscan: https://solscan.io/tx/{}", signature);
    Ok(())
}
