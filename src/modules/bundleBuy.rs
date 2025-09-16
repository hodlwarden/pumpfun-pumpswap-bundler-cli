use anyhow::Result;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::Keypair,
    transaction::{Transaction, VersionedTransaction},
    instruction::{Instruction},
    signer::Signer,
    commitment_config::CommitmentConfig,
    message::v0::Message as TransactionMessage,
    system_instruction,
};
use solana_program::{
    program_error::ProgramError,
    instruction::AccountMeta,
    system_program,
};
use spl_token::instruction as token_instruction;
use spl_associated_token_account::instruction as associated_token_instruction;
use std::str::FromStr;
use reqwest::Client;
use serde_json::json;
use base64;
use bincode::serialize;
use rand::Rng;
use rand::seq::SliceRandom;
use crate::dex::pump::{PumpDex, TRANSFER_FEE_BPS, FEE_DENOMINATOR, TRANSFER_WALLET};
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use std::env;
use num_bigint::BigUint;
use num_traits::{One, Zero, ToPrimitive};


const TIP_ADDRESSES: [&str; 8] = [
    "96gYZGLnJYVFmbjzopPSU6QiEV5fGqZNyN9nmNhvrZU5",
    "HFqU5x63VTqvQss8hp11i4wVV8bD44PvwucfZ2bU7gRe",
    "Cw8CFyM9FkoMi7K7Crf6HNQqf4uEMzpKw6QNghXLvLkY",
    "ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49",
    "DfXygSm4jCyNCybVYYK6DwvWqjKee8pbDmJGcLWNDXjh",
    "ADuUkR4vqLUMWXxW9gh6D6L8pMSawimctcNZ5pGwDcEt",
    "DttWaMuVvTiduZRnguLF7jNxTgiMBZ1hyAumKUiL2KRL",
    "3AVi9Tg9Uo68tJfuvoKvqKNWKkC5wPdSSdeBnizKZ6jT"
];

const BLOCK_ENGINES: [&str; 6] = [
    "https://frankfurt.mainnet.block-engine.jito.wtf",
    "https://amsterdam.mainnet.block-engine.jito.wtf",
    "https://london.mainnet.block-engine.jito.wtf",
    "https://ny.mainnet.block-engine.jito.wtf",
    "https://tokyo.mainnet.block-engine.jito.wtf",
    "https://slc.mainnet.block-engine.jito.wtf"
];

const JITO_UUID: &str = "751f7390-2f50-11f0-858a-6bee29fce9c1";

#[derive(Debug)]
pub enum BundleBuyError {
    RpcError(String),
    TokenError(String),
    TransactionError(String),
    InvalidAmount(String),
    ProgramError(String),
    InvalidProgramId(String),
}

impl std::fmt::Display for BundleBuyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundleBuyError::RpcError(e) => write!(f, "RPC error: {}", e),
            BundleBuyError::TokenError(e) => write!(f, "Token error: {}", e),
            BundleBuyError::TransactionError(e) => write!(f, "Transaction error: {}", e),
            BundleBuyError::InvalidAmount(e) => write!(f, "Invalid amount: {}", e),
            BundleBuyError::ProgramError(e) => write!(f, "Program error: {}", e),
            BundleBuyError::InvalidProgramId(e) => write!(f, "Invalid program ID: {}", e),
        }
    }
}

impl std::error::Error for BundleBuyError {}

impl From<ProgramError> for BundleBuyError {
    fn from(err: ProgramError) -> Self {
        BundleBuyError::ProgramError(err.to_string())
    }
}

pub struct BundleBuy {
    rpc_client: RpcClient,
    token_mint: Pubkey,
    dex: PumpDex,
}

impl BundleBuy {
    pub fn new(
        rpc_url: String,
        token_mint: String,
    ) -> Result<Self, BundleBuyError> {
        let token_mint = Pubkey::from_str(&token_mint)
            .map_err(|e| BundleBuyError::TokenError(format!("Invalid token mint: {}", e)))?;

        Ok(Self {
            rpc_client: RpcClient::new(rpc_url),
            token_mint,
            dex: PumpDex::new(),
        })
    }

    fn get_random_tip_address() -> Pubkey {
        let mut rng = rand::thread_rng();
        let tip_address = TIP_ADDRESSES.choose(&mut rng).unwrap();
        Pubkey::from_str(tip_address).unwrap()
    }

    pub async fn buy_tokens(
        &self,
        wallet_keypairs: Vec<Keypair>,
        sol_amount: f64,
        jito_tip_sol: f64,
    ) -> Result<Vec<(u64, String)>, BundleBuyError> {
        let mut results = Vec::new();
        let num_wallets = wallet_keypairs.len() as u64;
        if num_wallets == 0 {
            results.push((0, "No wallets provided.".to_string()));
            return Ok(results);
        }

        // Convert to BigUint for precise calculations
        let sol_amount_big = BigUint::from((sol_amount * 1_000_000_000.0) as u64);
        let buy_lamports_per_wallet = (&sol_amount_big / BigUint::from(num_wallets)).to_u64()
            .ok_or_else(|| BundleBuyError::InvalidAmount("Failed to convert buy amount to u64".to_string()))?;
        
        let total_spend = buy_lamports_per_wallet * num_wallets;
        let total_spend_big = BigUint::from(total_spend);
        
        // Calculate fee using BigUint
        let mut total_fee = (&total_spend_big * BigUint::from(TRANSFER_FEE_BPS) / BigUint::from(FEE_DENOMINATOR))
            .to_u64()
            .ok_or_else(|| BundleBuyError::InvalidAmount("Failed to convert fee to u64".to_string()))?;
        
        if total_fee < 1_000 {
            total_fee = 1_000;
        }

        let tip_lamports = (jito_tip_sol * 1_000_000_000.0) as u64;

        let first_wallet_pubkey = wallet_keypairs[0].pubkey();
        let mut required_first_wallet = buy_lamports_per_wallet + total_fee + tip_lamports;
        let ata_rent = 2_039_280u64;
        
        // Calculate transfer amount using BigUint
        let transfer_amount_big = (BigUint::from(buy_lamports_per_wallet) * BigUint::from(TRANSFER_FEE_BPS)) / BigUint::from(FEE_DENOMINATOR);
        let mut transfer_amount = transfer_amount_big.to_u64()
            .ok_or_else(|| BundleBuyError::InvalidAmount("Failed to convert transfer amount to u64".to_string()))?;
        
        if transfer_amount < 1_000 {
            transfer_amount = 1_000;
        }

        required_first_wallet += transfer_amount + ata_rent;
        let balance_first_wallet = self.rpc_client.get_balance(&first_wallet_pubkey).unwrap_or(0);
        if balance_first_wallet < required_first_wallet {
            results.push((0, format!("First wallet {} has insufficient lamports: {} available, {} required. Aborting bundle buy.", first_wallet_pubkey, balance_first_wallet, required_first_wallet)));
            return Ok(results);
        }

        let mut insufficient_wallets = Vec::new();
        for wallet in wallet_keypairs.iter().skip(1) {
            let wallet_pubkey = wallet.pubkey();
            let balance = self.rpc_client.get_balance(&wallet_pubkey).unwrap_or(0);
            let required = buy_lamports_per_wallet + transfer_amount + ata_rent;
            if balance < required {
                insufficient_wallets.push((wallet_pubkey, balance, required));
            }
        }

        for (pubkey, balance, required) in &insufficient_wallets {
            results.push((0, format!("Wallet {} has insufficient lamports: {} available, {} required. Skipping.", pubkey, balance, required)));
        }

        let wallet_keypairs: Vec<Keypair> = wallet_keypairs.into_iter()
            .filter(|wallet| {
                let pubkey = wallet.pubkey();
                !insufficient_wallets.iter().any(|(p, _, _)| *p == pubkey)
            })
            .collect();

        if wallet_keypairs.is_empty() {
            results.push((0, "No wallets with sufficient funds for bundle buy.".to_string()));
            return Ok(results);
        }

        let tip_address = Self::get_random_tip_address();
        let jito_tip_ix = system_instruction::transfer(
            &wallet_keypairs[0].pubkey(),
            &tip_address,
            tip_lamports,
        );

        let fee_transfer_ix = system_instruction::transfer(
            &wallet_keypairs[0].pubkey(),
            &Pubkey::from_str(TRANSFER_WALLET).unwrap(),
            total_fee,
        );

        let mut jito_tip_instruction = Some(jito_tip_ix);
        let mut fee_transfer_instruction = Some(fee_transfer_ix);

        let chunk_size = 4; 
        let total_chunks = (wallet_keypairs.len() + chunk_size - 1) / chunk_size;

        let mut all_bundles: Vec<VersionedTransaction> = Vec::new();
        for chunk_index in 0..total_chunks {
            let start = chunk_index * chunk_size;
            let end = std::cmp::min(start + chunk_size, wallet_keypairs.len());
            let mut all_instructions = Vec::new();
            let mut signers = Vec::new();
            for (i, wallet) in wallet_keypairs[start..end].iter().enumerate() {
                let wallet_pubkey = wallet.pubkey();

                let balance = match self.rpc_client.get_balance(&wallet_pubkey) {
                    Ok(bal) => bal,
                    Err(e) => {
                        results.push((0, format!("Wallet {}: failed to fetch balance ({}). Skipping.", wallet_pubkey, e)));
                        continue;
                    }
                };

                let buy_amount_after_transfer = buy_lamports_per_wallet - transfer_amount;
                let mut required = buy_lamports_per_wallet + transfer_amount + ata_rent;

                if balance < required {
                    results.push((0, format!("Wallet {} has insufficient lamports: {} available, {} required. Skipping.", wallet_pubkey, balance, required)));
                    continue;
                }
                
                let wallet_ata = spl_associated_token_account::get_associated_token_address(
                    &wallet_pubkey,
                    &self.token_mint,
                );

                let (bonding_curve, _) = self.dex.get_bonding_curve(&self.token_mint);
                let a_bonding_curve = spl_associated_token_account::get_associated_token_address(
                    &bonding_curve,
                    &self.token_mint,
                );
                let curve_info = self.rpc_client
                    .get_account(&bonding_curve)
                    .map_err(|e| BundleBuyError::RpcError(e.to_string()))?;
                let creator_pubkey = Pubkey::try_from(&curve_info.data[49..81])
                    .map_err(|e| BundleBuyError::TokenError(format!("Failed to parse creator pubkey: {}", e)))?;
                let (creator_vault, _) = self.dex.get_creator_vault(&creator_pubkey);
                let virtual_token_reserves = u64::from_le_bytes(curve_info.data[8..16].try_into().unwrap());
                let virtual_sol_reserves = u64::from_le_bytes(curve_info.data[16..24].try_into().unwrap());

                let create_ata_ix = associated_token_instruction::create_associated_token_account_idempotent(
                    &wallet_pubkey,
                    &wallet_pubkey,
                    &self.token_mint,
                    &spl_token::id(),
                );

                let (tokens_to_receive, _, _) = self.dex.get_amount_out(
                    buy_amount_after_transfer,
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
                buy_instruction_data[16..24].copy_from_slice(&buy_amount_after_transfer.to_le_bytes());

                let buy_instruction = Instruction {
                    program_id: self.dex.program_id,
                    accounts: vec![
                        AccountMeta::new_readonly(self.dex.global, false),
                        AccountMeta::new(self.dex.fee_recipient, false),
                        AccountMeta::new(self.token_mint, false),
                        AccountMeta::new(bonding_curve, false),
                        AccountMeta::new(a_bonding_curve, false),
                        AccountMeta::new(wallet_ata, false),
                        AccountMeta::new(wallet_pubkey, true),
                        AccountMeta::new_readonly(system_program::id(), false),
                        AccountMeta::new_readonly(spl_token::id(), false),
                        AccountMeta::new(creator_vault, false),
                        AccountMeta::new_readonly(self.dex.event_authority, false),
                        AccountMeta::new_readonly(self.dex.program_id, false),
                    ],
                    data: buy_instruction_data,
                };

                all_instructions.push(create_ata_ix);
                all_instructions.push(buy_instruction);
                signers.push(wallet);
            }

            if chunk_index == 0 {
                if let Some(tip_ix) = jito_tip_instruction.take() {
                    all_instructions.insert(0, tip_ix);
                }
                if let Some(fee_ix) = fee_transfer_instruction.take() {
                    all_instructions.insert(0, fee_ix);
                }
            }

            let mut retry_count = 0;
            let max_retries = 32;
            let mut transaction = None;

            while retry_count < max_retries {
                let recent_blockhash = self.rpc_client
                    .get_latest_blockhash()
                    .map_err(|e| BundleBuyError::RpcError(e.to_string()))?;

                let message = TransactionMessage::try_compile(
                    &wallet_keypairs[start].pubkey(),
                    &all_instructions,
                    &[],
                    recent_blockhash,
                ).map_err(|e| BundleBuyError::TransactionError(format!("Failed to compile message: {}", e)))?;

                let tx = VersionedTransaction::try_new(
                    solana_sdk::message::VersionedMessage::V0(message),
                    &signers
                ).map_err(|e| BundleBuyError::TransactionError(format!("Failed to create versioned transaction: {}", e)))?;

                transaction = Some(tx);
                break;
            }

            if transaction.is_none() {
                return Err(BundleBuyError::TransactionError("Failed to create valid transaction after maximum retries".to_string()));
            }

            all_bundles.push(transaction.unwrap());
        }

        if !all_bundles.is_empty() {
            let bundle_url = Self::send_jito_bundle(all_bundles.clone(), None)
                .await
                .map_err(|e| BundleBuyError::TransactionError(format!("Failed to send bundle to Jito: {}", e)))?;
            
            results.push((0, format!("Bundle sent to Jito. Explorer URL: {}", bundle_url)));
        }

        Ok(results)
    }

    async fn send_jito_bundle(
        txs: Vec<VersionedTransaction>,
        jito_uuid: Option<&str>,
    ) -> Result<String, anyhow::Error> {
        let client = Client::new();
        let send_to_all = env::var("SEND_TO_ALL").unwrap_or_else(|_| "true".to_string()) == "true";

        let bundle_base64: Vec<String> = txs.iter()
            .map(|tx| {
                let serialized = bincode::serialize(tx)
                    .map_err(|e| anyhow::anyhow!("Failed to serialize transaction: {}", e))?;
                Ok::<String, anyhow::Error>(base64::engine::general_purpose::STANDARD.encode(serialized))
            })
            .collect::<Result<Vec<String>, anyhow::Error>>()?;

        let bundle_request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendBundle",
            "params": [
                bundle_base64,
                {
                    "encoding": "base64"
                }
            ]
        });

        if send_to_all {
            let mut handles = Vec::new();
            let mut results = Vec::new();
            
            for engine in BLOCK_ENGINES.iter() {
                let client = client.clone();
                let engine = engine.to_string();
                let bundle_request = bundle_request.clone();
                let jito_uuid = jito_uuid.map(|u| format!("?uuid={}", u)).unwrap_or_default();
                
                let handle = tokio::spawn(async move {
                    match client
                        .post(format!("{}/api/v1/bundles{}", engine, jito_uuid))
                        .json(&bundle_request)
                        .send()
                        .await
                    {
                        Ok(res) => {
                            let status = res.status();
                            match res.text().await {
                                Ok(text) => {
                                    if text.is_empty() {
                                        return Err(format!("Empty response from {}", engine));
                                    }
                                    match serde_json::from_str::<serde_json::Value>(&text) {
                                        Ok(body) => {
                                            if status.is_success() {
                                                if let Some(bundle_id) = body.get("result") {
                                                    let bundle_id = bundle_id.to_string().trim_matches('"').to_string();
                                                    let engine_name = engine.split('.').nth(0).unwrap_or("unknown");
                                                    Ok(format!("[{}] {}", engine_name, bundle_id))
                                                } else {
                                                    Err(format!("No bundle ID from {}", engine))
                                                }
                                            } else {
                                                let error = body.get("error")
                                                    .and_then(|e| e.get("message"))
                                                    .and_then(|m| m.as_str())
                                                    .unwrap_or("Unknown error");
                                                Err(format!("Error from {}: {}", engine, error))
                                            }
                                        }
                                        Err(e) => {
                                            Err(format!("Parse error from {}: {}", engine, e))
                                        }
                                    }
                                }
                                Err(e) => {
                                    Err(format!("Read error from {}: {}", engine, e))
                                }
                            }
                        }
                        Err(e) => {
                            Err(format!("Send error to {}: {}", engine, e))
                        }
                    }
                });
                handles.push(handle);
            }

            for handle in handles {
                match handle.await {
                    Ok(result) => {
                        match result {
                            Ok(bundle_id) => results.push(bundle_id),
                            Err(e) => println!("Error: {}", e),
                        }
                    }
                    Err(e) => println!("Error: {}", e),
                }
            }

            if !results.is_empty() {
                Ok(results.join("\n"))
            } else {
                Err(anyhow::anyhow!("Failed to send bundle to any block engine"))
            }
        } else {
            let block_engine = env::var("BLOCK_ENGINE").map_err(|_| anyhow::anyhow!("BLOCK_ENGINE must be set"))?;
            let jito_uuid = jito_uuid.map(|u| format!("?uuid={}", u)).unwrap_or_default();
            
            let res = client
                .post(format!("{}/api/v1/bundles{}", block_engine, jito_uuid))
                .json(&bundle_request)
                .send()
                .await?;

            let status = res.status();
            let body = res.json::<serde_json::Value>().await?;

            if status.is_success() {
                if let Some(bundle_id) = body.get("result") {
                    let bundle_id = bundle_id.to_string().trim_matches('"').to_string();
                    let explorer_url = format!("https://explorer.jito.wtf/bundle/{}", bundle_id);
                    Ok(explorer_url)
                } else {
                    Ok("Bundle sent successfully but no ID returned".to_string())
                }
            } else {
                let error = body.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("Unknown error");
                Err(anyhow::anyhow!("Error sending bundle: {}", error))
            }
        }
    }
}
