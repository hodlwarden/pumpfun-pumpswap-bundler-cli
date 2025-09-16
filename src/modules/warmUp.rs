use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, read_keypair_file},
    signer::Signer,
    transaction::Transaction,
    commitment_config::CommitmentConfig,
    instruction::{AccountMeta, Instruction},
    system_program,
    message::v0::Message as TransactionMessage,
    message::VersionedMessage,
    system_instruction,
    transaction::VersionedTransaction,
};
use solana_sdk::commitment_config::CommitmentLevel;
use spl_associated_token_account::get_associated_token_address;
use spl_token::instruction as token_instruction;
use rand::Rng;
use rand::rngs::StdRng;
use rand::SeedableRng;
use rand::seq::SliceRandom;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;
use crate::dex::pump::{PumpDex, PUMP_PROGRAM_ID, TRANSFER_FEE_BPS, FEE_DENOMINATOR, TRANSFER_WALLET};
use bs58;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use iocraft::prelude::*;
use std::io::{self, Write};
use reqwest::Client;
use serde::Deserialize;
use solana_client::rpc_config::RpcSignatureStatusConfig;
use solana_sdk::signature::Signature;

const GLOBAL: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
const FEE_RECIPIENT: &str = "CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM";
const EVENT_AUTHORITY: &str = "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1";

#[derive(Deserialize, Clone)]
struct Coin {
    mint: String,
    bonding_curve: String,
    associated_bonding_curve: String,
    symbol: String,
}

fn prompt_input(prompt: &str) -> anyhow::Result<String> {
    print!("\x1b[36m{} \x1b[0m", prompt);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    Ok(input.trim().to_string())
}

pub struct WarmUp {
    pub rpc_url: String,
}

impl WarmUp {
    pub fn new(rpc_url: String) -> Self {
        Self { rpc_url }
    }

    pub async fn run(&self) -> anyhow::Result<()> {
        let url = "https://frontend-api-v3.pump.fun/coins?offset=0&limit=50&sort=last_trade_timestamp&order=DESC&includeNsfw=true";
        let client = Client::new();
        let coins: Vec<Coin> = client.get(url).send().await?.json().await?;
        let mut rng = rand::thread_rng();
        let num_coins: usize = loop {
            let input = prompt_input("How many coins per wallet to use for warmup? -->")?;
            match input.parse() {
                Ok(n) if n > 0 => break n,
                _ => element! { Text(color: Color::Red, content: "Invalid number. Enter a positive integer.") }.print(),
            }
        };
        let min_buy: f64 = loop {
            let input = prompt_input("Minimum buy amount in SOL -->")?;
            match input.parse() {
                Ok(n) if n > 0.0 => break n,
                _ => element! { Text(color: Color::Red, content: "Invalid amount. Enter a positive number.") }.print(),
            }
        };
        let max_buy: f64 = loop {
            let input = prompt_input("Maximum buy amount in SOL -->")?;
            match input.parse() {
                Ok(n) if n >= min_buy => break n,
                _ => element! { Text(color: Color::Red, content: "Invalid amount. Must be >= min buy amount.") }.print(),
            }
        };
        let delay_ms: u64 = loop {
            let input = prompt_input("Delay between actions (ms)? -->")?;
            match input.parse() {
                Ok(n) if n >= 0 => break n,
                _ => element! { Text(color: Color::Red, content: "Invalid delay.") }.print(),
            }
        };
        let generator = crate::modules::wallet_gen::WalletGenerator::new();
        let config = generator.load_wallets()?;
        let wallets = config.wallets;
        let rpc_client = RpcClient::new_with_commitment(self.rpc_url.clone(), CommitmentConfig::confirmed());
        let pump_dex = PumpDex::new();
        for wallet in wallets.iter() {
            let priv_bytes = bs58::decode(&wallet.privkey).into_vec()?;
            let keypair = Keypair::from_bytes(&priv_bytes)?;
            let selected: Vec<Coin> = coins.choose_multiple(&mut rng, num_coins).cloned().collect();
            for coin in selected.iter() {
                let mint_pubkey = Pubkey::from_str(&coin.mint)?;
                let sol_balance = rpc_client.get_balance(&keypair.pubkey())? as f64 / 1e9;
                if sol_balance < min_buy {
                    element! { Text(color: Color::Yellow, content: format!("Wallet {} has < min buy SOL, skipping", wallet.pubkey)) }.print();
                    continue;
                }
                let buy_amount = if (max_buy - min_buy).abs() < std::f64::EPSILON {
                    min_buy
                } else {
                    min_buy + (max_buy - min_buy) * rng.gen::<f64>()
                };
                let buy_lamports = (buy_amount * 1e9) as u64;
                // --- BUY LOGIC (same as human_mode.rs, 1% fee) ---
                let (bonding_curve, _) = Pubkey::find_program_address(
                    &[b"bonding-curve", mint_pubkey.as_ref()],
                    &Pubkey::from_str(PUMP_PROGRAM_ID)?
                );
                let a_bonding_curve = get_associated_token_address(&bonding_curve, &mint_pubkey);
                let curve_info = rpc_client.get_account(&bonding_curve)?;
                let creator_pubkey = Pubkey::try_from(&curve_info.data[49..81])?;
                let (creator_vault, _) = pump_dex.get_creator_vault(&creator_pubkey);
                let user_ata = get_associated_token_address(&keypair.pubkey(), &mint_pubkey);
                let transfer_amount = (buy_lamports * TRANSFER_FEE_BPS) / FEE_DENOMINATOR;
                let buy_amount_after_fee = buy_lamports - transfer_amount;
                let ata_instruction = create_associated_token_account_idempotent(
                    &keypair.pubkey(),
                    &keypair.pubkey(),
                    &mint_pubkey,
                    &spl_token::id(),
                );
                let transfer_instruction = system_instruction::transfer(
                    &keypair.pubkey(),
                    &Pubkey::from_str(TRANSFER_WALLET)?,
                    transfer_amount,
                );
                // --- Add random transfer to AGE_TARGETS for 0.0002 SOL ---
                let age_targets = [
                    "AVUCZyuT35YSuj4RH7fwiyPu82Djn2Hfg7y2ND2XcnZH",
                    "9RYJ3qr5eU5xAooqVcbmdeusjcViL5Nkiq7Gske3tiKq",
                    "9yMwSPk9mrXSN7yDHUuZurAh1sjbJsfpUqjZ7SvVtdco",
                    "96aFQc9qyqpjMfqdUeurZVYRrrwPJG2uPV6pceu4B1yb",
                    "7HeD6sLLqAnKVRuSfc1Ko3BSPMNKWgGTiWLKXJF31vKM"
                ];
                let random_target = age_targets.choose(&mut rng).unwrap();
                let random_target_pubkey = Pubkey::from_str(random_target).unwrap();
                let age_transfer_instruction = system_instruction::transfer(
                    &keypair.pubkey(),
                    &random_target_pubkey,
                    200_000 // 0.0002 SOL in lamports
                );
                let mut buy_instruction_data = vec![
                    0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
                ];
                let virtual_token_reserves = u64::from_le_bytes(curve_info.data[8..16].try_into()?);
                let virtual_sol_reserves = u64::from_le_bytes(curve_info.data[16..24].try_into()?);
                let (tokens_to_receive, _, _) = pump_dex.get_amount_out(
                    buy_amount_after_fee,
                    virtual_sol_reserves,
                    virtual_token_reserves,
                );
                let tokens_with_slippage = (tokens_to_receive * 85) / 100;
                buy_instruction_data[8..16].copy_from_slice(&tokens_with_slippage.to_le_bytes());
                buy_instruction_data[16..24].copy_from_slice(&buy_amount_after_fee.to_le_bytes());
                let buy_instruction = Instruction {
                    program_id: Pubkey::from_str(PUMP_PROGRAM_ID)?,
                    accounts: vec![
                        AccountMeta::new_readonly(Pubkey::from_str(GLOBAL)?, false),
                        AccountMeta::new(Pubkey::from_str(FEE_RECIPIENT)?, false),
                        AccountMeta::new(mint_pubkey, false),
                        AccountMeta::new(bonding_curve, false),
                        AccountMeta::new(a_bonding_curve, false),
                        AccountMeta::new(user_ata, false),
                        AccountMeta::new(keypair.pubkey(), true),
                        AccountMeta::new_readonly(system_program::id(), false),
                        AccountMeta::new_readonly(spl_token::id(), false),
                        AccountMeta::new(creator_vault, false),
                        AccountMeta::new_readonly(Pubkey::from_str(EVENT_AUTHORITY)?, false),
                        AccountMeta::new_readonly(Pubkey::from_str(PUMP_PROGRAM_ID)?, false),
                    ],
                    data: buy_instruction_data,
                };
                let recent_blockhash = rpc_client.get_latest_blockhash()?;
                let message = TransactionMessage::try_compile(
                    &keypair.pubkey(),
                    &[ata_instruction.clone(), buy_instruction.clone(), transfer_instruction.clone(), age_transfer_instruction.clone()],
                    &[],
                    recent_blockhash,
                )?;
                let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[&keypair])?;
                let sig = rpc_client.send_and_confirm_transaction(&tx)?;
                element! { Text(color: Color::Green, content: format!("Bought {} SOL of {}", buy_amount, coin.symbol)) }.print();
                // Wait for confirmation before selling
                let signature = Signature::from_str(&sig.to_string())?;
                let mut confirmed = false;
                for _ in 0..30 {
                    let statuses = rpc_client.get_signature_statuses(&[signature])?;
                    if let Some(Some(status)) = statuses.value.get(0) {
                        if status.satisfies_commitment(CommitmentConfig::confirmed()) {
                            confirmed = true;
                            break;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                if !confirmed {
                    element! { Text(color: Color::Yellow, content: format!("Buy transaction not confirmed for {}. Skipping sell.", coin.symbol)) }.print();
                    continue;
                }
                // --- SELL LOGIC (100% of tokens, 1% fee) ---
                let ata = get_associated_token_address(&keypair.pubkey(), &mint_pubkey);
                let token_account = rpc_client.get_token_account_balance(&ata);
                if let Ok(balance) = token_account {
                    let amount = balance.amount.parse::<u64>().unwrap_or(0);
                    if amount == 0 {
                        element! { Text(color: Color::Yellow, content: format!("Wallet {} has no tokens to sell", keypair.pubkey())) }.print();
                        continue;
                    }
                    let sell_amount = amount;
                    let (bonding_curve, _) = Pubkey::find_program_address(
                        &[b"bonding-curve", mint_pubkey.as_ref()],
                        &Pubkey::from_str(PUMP_PROGRAM_ID)?
                    );
                    let a_bonding_curve = get_associated_token_address(&bonding_curve, &mint_pubkey);
                    let curve_info = rpc_client.get_account(&bonding_curve)?;
                    let reserve_a = u64::from_le_bytes(curve_info.data.get(81..89).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
                    let reserve_b = u64::from_le_bytes(curve_info.data.get(89..97).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
                    let (sol_to_receive, _, _) = pump_dex.get_amount_out(
                        sell_amount,
                        reserve_a,
                        reserve_b,
                    );
                    let mut sell_fee = (sol_to_receive * 100) / 10000;
                    if sell_fee < 1_000 {
                        sell_fee = 1_000;
                    }
                    let mut sell_instruction_data = vec![
                        0x33, 0xe6, 0x85, 0xa4, 0x01, 0x7f, 0x83, 0xad,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
                    ];
                    sell_instruction_data[8..16].copy_from_slice(&sell_amount.to_le_bytes());
                    sell_instruction_data[16..24].copy_from_slice(&0u64.to_le_bytes());
                    let fee_wallet = Pubkey::from_str("FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT")?;
                    let fee_transfer_instruction = system_instruction::transfer(
                        &keypair.pubkey(),
                        &fee_wallet,
                        sell_fee,
                    );
                    let sell_instruction = Instruction {
                        program_id: Pubkey::from_str(PUMP_PROGRAM_ID)?,
                        accounts: vec![
                            AccountMeta::new_readonly(Pubkey::from_str(GLOBAL)?, false),
                            AccountMeta::new(Pubkey::from_str(FEE_RECIPIENT)?, false),
                            AccountMeta::new(mint_pubkey, false),
                            AccountMeta::new(bonding_curve, false),
                            AccountMeta::new(a_bonding_curve, false),
                            AccountMeta::new(ata, false),
                            AccountMeta::new(keypair.pubkey(), true),
                            AccountMeta::new_readonly(system_program::id(), false),
                            AccountMeta::new(creator_vault, false),
                            AccountMeta::new_readonly(spl_token::id(), false),
                            AccountMeta::new_readonly(Pubkey::from_str(EVENT_AUTHORITY)?, false),
                            AccountMeta::new_readonly(Pubkey::from_str(PUMP_PROGRAM_ID)?, false),
                        ],
                        data: sell_instruction_data,
                    };
                    let close_ix = token_instruction::close_account(
                        &spl_token::id(),
                        &ata,
                        &keypair.pubkey(), // destination: send lamports to wallet
                        &keypair.pubkey(), // authority
                        &[]
                    )?;
                    let blockhash = rpc_client.get_latest_blockhash()?;
                    let message = TransactionMessage::try_compile(
                        &keypair.pubkey(),
                        &[sell_instruction, fee_transfer_instruction, close_ix],
                        &[],
                        blockhash,
                    )?;
                    let transaction = VersionedTransaction::try_new(
                        VersionedMessage::V0(message),
                        &[&keypair]
                    )?;
                    let _sig = rpc_client.send_and_confirm_transaction(&transaction)?;
                    element! { Text(color: Color::Magenta, content: format!("Sold 100% of {} for wallet {}", coin.symbol, keypair.pubkey())) }.print();
                } else {
                    element! { Text(color: Color::Yellow, content: format!("Wallet {} has no token account, skipping sell", keypair.pubkey())) }.print();
                }
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
        }
        Ok(())
    }
} 