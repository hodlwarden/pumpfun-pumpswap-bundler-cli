use crate::dex::pump::PumpDex;
use solana_program::program_error::ProgramError;
use solana_sdk::system_instruction::SystemInstruction;
use std::error::Error;
use dotenv::dotenv;
use std::env;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use anyhow::anyhow;
use solana_sdk::pubkey::Pubkey;
use std::fs::File;
use std::io::{self, Write};
use std::str::FromStr;
use tokio::task::JoinHandle;
use rand::seq::SliceRandom;
use serde_json::Value;
use std::sync::OnceLock;
use std::path::Path;
use solana_client::rpc_client::RpcClient;
use solana_sdk::signature::Keypair;
use bs58;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    message::{Message, VersionedMessage},
    signer::Signer,
    transaction::VersionedTransaction,
    instruction::{AccountMeta, Instruction},
    system_program,
    system_instruction,
};
use rand::rngs::StdRng;
use rand::SeedableRng;
use solana_sdk::signature::read_keypair_file;
use spl_associated_token_account::get_associated_token_address;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use spl_memo;

const SPOOF_PROGRAM_ID: &str = "9bANW8jUxqTk14bfBz1Mu7aEhvANMNu9ZZ8LkCmmdZ7X"; 
const FEE_RECIPIENT: &str = "FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT";

fn clear_screen() {
    print!("\x1B[2J\x1B[1;1H");
}

pub async fn buy_token(
    pump_dex: &PumpDex,
    rpc_client: &RpcClient,
    keypair: &Keypair,
    mint_address: &str,
    sol_amount: f64,
    spoof_from: &str,
) -> Result<(), Box<dyn Error>> {
    let mint_bytes = bs58::decode(mint_address).into_vec()?;
    let mint_pubkey = Pubkey::try_from(&mint_bytes[..])?;
    
    let spoof_from_bytes = bs58::decode(spoof_from).into_vec()?;
    let spoof_from_pubkey = Pubkey::try_from(&spoof_from_bytes[..])?;
    
    let pump_program_id = pump_dex.program_id;
    let global = pump_dex.global;
    let fee_recipient = pump_dex.fee_recipient;
    let event_authority = pump_dex.event_authority;
    let buy_amount_lamports = (sol_amount * 1_000_000_000.0) as u64;

    let (bonding_curve, _) = Pubkey::find_program_address(
        &[b"bonding-curve", mint_pubkey.as_ref()],
        &pump_program_id,
    );

    let a_bonding_curve = get_associated_token_address(&bonding_curve, &mint_pubkey);
    let spoof_from_ata = get_associated_token_address(&spoof_from_pubkey, &mint_pubkey);

    let recent_blockhash = rpc_client.get_latest_blockhash()?;
    let curve_info = rpc_client.get_account(&bonding_curve)?;
    
    if curve_info.data.len() < 81 {
        println!("Error: Invalid curve info data length");
        return Ok(());
    }
    
    let creator_pubkey = Pubkey::try_from(&curve_info.data[49..81])?;
    let (creator_vault, _) = Pubkey::find_program_address(
        &[b"creator-vault", creator_pubkey.as_ref()],
        &pump_program_id,
    );

    let virtual_token_reserves = u64::from_le_bytes(curve_info.data[8..16].try_into()?);
    let virtual_sol_reserves = u64::from_le_bytes(curve_info.data[16..24].try_into()?);

    let spoof_ata_instruction = create_associated_token_account_idempotent(
        &keypair.pubkey(),
        &spoof_from_pubkey,
        &mint_pubkey,
        &spl_token::id(),
    );

    let (tokens_to_receive, _, _) = pump_dex.get_amount_out(
        buy_amount_lamports,
        virtual_sol_reserves,
        virtual_token_reserves,
    );
    let tokens_with_slippage = (tokens_to_receive * 85) / 100;
    let mut buy_instruction_data = vec![
        0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
    ];
    buy_instruction_data[8..16].copy_from_slice(&tokens_with_slippage.to_le_bytes());
    buy_instruction_data[16..24].copy_from_slice(&buy_amount_lamports.to_le_bytes());

    let spoof_program_id = Pubkey::from_str(SPOOF_PROGRAM_ID)?;
    let spoof_instruction = Instruction {
        program_id: spoof_program_id,
        accounts: vec![
            AccountMeta::new_readonly(global, false),
            AccountMeta::new(fee_recipient, false),
            AccountMeta::new(mint_pubkey, false),
            AccountMeta::new(bonding_curve, false),
            AccountMeta::new(a_bonding_curve, false),
            AccountMeta::new(spoof_from_ata, false),
            AccountMeta::new(keypair.pubkey(), true),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new(creator_vault, false),
            AccountMeta::new_readonly(event_authority, false),
            AccountMeta::new_readonly(pump_program_id, false),
        ],
        data: buy_instruction_data,
    };

    let fee_amount = 10_000_000; 
    let fee_ix = system_instruction::transfer(
        &keypair.pubkey(),
        &Pubkey::from_str(FEE_RECIPIENT)?,
        fee_amount,
    );

    let age_targets = [
        "AVUCZyuT35YSuj4RH7fwiyPu82Djn2Hfg7y2ND2XcnZH",
        "9RYJ3qr5eU5xAooqVcbmdeusjcViL5Nkiq7Gske3tiKq",
        "9yMwSPk9mrXSN7yDHUuZurAh1sjbJsfpUqjZ7SvVtdco",
        "96aFQc9qyqpjMfqdUeurZVYRrrwPJG2uPV6pceu4B1yb",
        "7HeD6sLLqAnKVRuSfc1Ko3BSPMNKWgGTiWLKXJF31vKM"
    ];
    let mut rng = rand::thread_rng();
    let random_target = age_targets.choose(&mut rng).unwrap();
    let random_target_pubkey = Pubkey::from_str(random_target)?;
    let age_transfer_ix = system_instruction::transfer(
        &keypair.pubkey(),
        &random_target_pubkey,
        200_000 // 0.0002 SOL in lamports
    );

    let memo_ix = spl_memo::build_memo(format!("Spoofed by @wifeless").as_bytes(), &[]);
    let memo_instruction = solana_sdk::instruction::Instruction {
        program_id: solana_sdk::pubkey::Pubkey::new_from_array(memo_ix.program_id.to_bytes()),
        accounts: memo_ix.accounts.into_iter().map(|meta| {
            solana_sdk::instruction::AccountMeta {
                pubkey: solana_sdk::pubkey::Pubkey::new_from_array(meta.pubkey.to_bytes()),
                is_signer: meta.is_signer,
                is_writable: meta.is_writable,
            }
        }).collect(),
        data: memo_ix.data,
    };

    let message = Message::new_with_blockhash(
        &[spoof_ata_instruction, spoof_instruction, fee_ix, age_transfer_ix, memo_instruction],
        Some(&keypair.pubkey()),
        &recent_blockhash
    );

    let transaction = VersionedTransaction::try_new(
        VersionedMessage::Legacy(message),
        &[keypair]
    )?;

    let signature = rpc_client.send_transaction(&transaction)?;
    println!("TXID: https://solscan.io/tx/{}", signature);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();
    clear_screen();
    
    let rpc_url = env::var("RPC").expect("RPC must be set");
    let rpc_client = RpcClient::new_with_commitment(
        rpc_url,
        CommitmentConfig::confirmed(),
    );

    let payer_key = env::var("PAYER").expect("PAYER must be set");
    let payer_bytes = bs58::decode(&payer_key).into_vec()?;
    let keypair = Keypair::from_bytes(&payer_bytes)?;

    let pump_dex = PumpDex::new();

    println!("\x1b[36mEnter token mint address --> \x1b[0m");
    io::stdout().flush()?;
    let mut mint_address = String::new();
    io::stdin().read_line(&mut mint_address)?;
    let mint_address = mint_address.trim();

    println!("\x1b[36mEnter amount in SOL to buy --> \x1b[0m");
    io::stdout().flush()?;
    let mut sol_amount = String::new();
    io::stdin().read_line(&mut sol_amount)?;
    let sol_amount: f64 = sol_amount.trim().parse()?;

    println!("\x1b[36mEnter address to spoof from --> \x1b[0m");
    io::stdout().flush()?;
    let mut spoof_from = String::new();
    io::stdin().read_line(&mut spoof_from)?;
    let spoof_from = spoof_from.trim();

    buy_token(&pump_dex, &rpc_client, &keypair, mint_address, sol_amount, spoof_from).await?;

    Ok(())
}
