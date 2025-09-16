use anyhow::Result;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::Keypair,
    transaction::{Transaction, VersionedTransaction},
    instruction::{Instruction, AccountMeta},
    signer::Signer,
    commitment_config::CommitmentConfig,
    message::v0::Message as TransactionMessage,
    system_instruction,
};
use solana_program::{
    program_error::ProgramError,
    system_program,
};
use spl_token::instruction as token_instruction;
use spl_associated_token_account::instruction as associated_token_instruction;
use std::str::FromStr;
use solana_client::rpc_request::TokenAccountsFilter;
use std::collections::HashSet;
use reqwest::Client;
use serde_json::json;
use bs58;
use bincode::serialize;
use rand::Rng;
use rand::seq::SliceRandom;
use base64;
use crate::dex::pump::{PumpDex, PUMP_PROGRAM_ID, TRANSFER_FEE_BPS, FEE_DENOMINATOR, TRANSFER_WALLET};
use std::env;
use dotenv::dotenv;
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

#[derive(Debug)]
pub enum DevDumpError {
    RpcError(String),
    TokenError(String),
    TransactionError(String),
    InvalidAmount(String),
    ProgramError(String),
    InvalidProgramId(String),
}

impl std::fmt::Display for DevDumpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DevDumpError::RpcError(e) => write!(f, "RPC error: {}", e),
            DevDumpError::TokenError(e) => write!(f, "Token error: {}", e),
            DevDumpError::TransactionError(e) => write!(f, "Transaction error: {}", e),
            DevDumpError::InvalidAmount(e) => write!(f, "Invalid amount: {}", e),
            DevDumpError::ProgramError(e) => write!(f, "Program error: {}", e),
            DevDumpError::InvalidProgramId(e) => write!(f, "Invalid program ID: {}", e),
        }
    }
}

impl std::error::Error for DevDumpError {}

impl From<ProgramError> for DevDumpError {
    fn from(err: ProgramError) -> Self {
        DevDumpError::ProgramError(err.to_string())
    }
}

pub struct DevDump {
    rpc_client: RpcClient,
    funder_keypair: Keypair,
    token_mint: Pubkey,
    dump_percentage: u8,
    jito_tip_sol: f64,
    dex: PumpDex,
}

impl DevDump {
    fn calc_sell_fee(&self, sol_to_receive: u64) -> u64 {
        if sol_to_receive > 0 {
            let calculated_fee = (sol_to_receive * 100) / 10000;
            if calculated_fee < 1_000 {
                1_000
            } else {
                calculated_fee
            }
        } else {
            1_000
        }
    }

    fn get_payer_from_env() -> Result<Keypair, DevDumpError> {
        dotenv().ok();
        let payer_privkey = env::var("DEV")
            .map_err(|_| DevDumpError::TokenError("DEV not set in .env file".to_string()))?;
        let bytes = bs58::decode(&payer_privkey)
            .into_vec()
            .map_err(|e| DevDumpError::TokenError(format!("Failed to decode payer private key: {}", e)))?;
        Keypair::from_bytes(&bytes)
            .map_err(|e| DevDumpError::TokenError(format!("Failed to create keypair: {}", e)))
    }

    pub fn new(
        rpc_url: String,
        funder_keypair: Keypair,
        token_mint: String,
        dump_percentage: u8,
        jito_tip_sol: f64,
    ) -> Result<Self, DevDumpError> {
        if dump_percentage > 100 {
            return Err(DevDumpError::InvalidAmount("Dump percentage cannot exceed 100".to_string()));
        }

        let token_mint = Pubkey::from_str(&token_mint)
            .map_err(|e| DevDumpError::TokenError(format!("Invalid token mint: {}", e)))?;

        let funder_keypair = Self::get_payer_from_env()?;

        Ok(Self {
            rpc_client: RpcClient::new(rpc_url),
            funder_keypair,
            token_mint,
            dump_percentage,
            jito_tip_sol,
            dex: PumpDex::new(),
        })
    }

    pub fn get_token_accounts(&self, wallet_pubkey: &Pubkey) -> Result<Vec<(Pubkey, u64)>, DevDumpError> {
        match self.rpc_client.get_account(wallet_pubkey) {
            Ok(_) => {},
            Err(_) => return Ok(Vec::new()),
        }

        let token_accounts = self.rpc_client.get_token_accounts_by_owner_with_commitment(
            wallet_pubkey,
            TokenAccountsFilter::ProgramId(spl_token::id()),
            CommitmentConfig::confirmed(),
        ).map_err(|e| DevDumpError::RpcError(e.to_string()))?;
        
        let mut accounts = Vec::new();
        for account in token_accounts.value {
            match &account.account.data {
                solana_account_decoder::UiAccountData::Json(parsed) => {
                    if let Some(token_info) = parsed.parsed.get("info") {
                        let mint_str = token_info.get("mint").and_then(|m| m.as_str()).unwrap_or("");
                        let amount_str = token_info.get("tokenAmount")
                            .and_then(|ta| ta.get("amount"))
                            .and_then(|a| a.as_str())
                            .unwrap_or("0");
                        let amount = amount_str.parse::<u64>().unwrap_or(0);
                        
                        if mint_str == self.token_mint.to_string() {
                            let pubkey = Pubkey::from_str(&account.pubkey)
                                .map_err(|e| DevDumpError::TokenError(format!("Invalid pubkey: {}", e)))?;
                            accounts.push((pubkey, amount));
                        }
                    }
                }
                _ => {}
            }
        }
        
        Ok(accounts)
    }

    fn get_random_tip_address() -> Pubkey {
        let mut rng = rand::thread_rng();
        let tip_address = TIP_ADDRESSES.choose(&mut rng).unwrap();
        Pubkey::from_str(tip_address).unwrap()
    }

    fn verify_token_balance(&self, token_account: &Pubkey, expected_balance: u64) -> Result<u64, DevDumpError> {
        let balance = self.rpc_client
            .get_token_account_balance(token_account)
            .map_err(|e| DevDumpError::RpcError(e.to_string()))?;
        
        let actual_balance = balance.amount.parse::<u64>()
            .map_err(|e| DevDumpError::TokenError(format!("Failed to parse token amount: {}", e)))?;
        
        Ok(actual_balance)
    }

    pub async fn dump_tokens(
        &self,
        wallet_keypairs: Vec<Keypair>,
        jito_uuid: &str,
        tip_lamports: u64,
    ) -> Result<Vec<(u64, String)>, DevDumpError> {
        let mut results = Vec::new();
        let funder_pubkey = self.funder_keypair.pubkey();
        let funder_token_account = spl_associated_token_account::get_associated_token_address(
            &funder_pubkey,
            &self.token_mint,
        );

        let create_ata_ix = spl_associated_token_account::instruction::create_associated_token_account(
            &funder_pubkey,
            &funder_pubkey,
            &self.token_mint,
            &spl_token::id(),
        );

        let recent_blockhash = self.rpc_client
            .get_latest_blockhash()
            .map_err(|e| DevDumpError::RpcError(e.to_string()))?;

        let create_ata_tx = Transaction::new_signed_with_payer(
            &[create_ata_ix],
            Some(&funder_pubkey),
            &[&self.funder_keypair],
            recent_blockhash,
        );

        let _ = self.rpc_client.send_and_confirm_transaction(&create_ata_tx);

        let mut all_transfers: Vec<(Pubkey, Pubkey, u64, &Keypair)> = Vec::new();
        let mut processed_accounts = HashSet::new();

        for wallet in &wallet_keypairs {
            let wallet_pubkey = wallet.pubkey();
            let token_accounts = self.get_token_accounts(&wallet_pubkey)?;
            for (token_account, balance) in token_accounts {
                if !processed_accounts.contains(&token_account) && balance > 0 {
                    let balance_big = BigUint::from(balance);
                    let transfer_amount = if self.dump_percentage == 100 {
                        ((&balance_big * BigUint::from(99u64)) / BigUint::from(100u64)).to_u64().unwrap_or(0)
                    } else {
                        ((&balance_big * BigUint::from(self.dump_percentage as u64)) / BigUint::from(100u64)).to_u64().unwrap_or(0)
                    };
                    
                    if transfer_amount > 0 {
                        all_transfers.push((token_account, wallet_pubkey, transfer_amount, wallet));
                        processed_accounts.insert(token_account);
                }
            }
            }
        }

        let payer_token_accounts = self.get_token_accounts(&self.funder_keypair.pubkey())?;
        let mut payer_sell_amount = BigUint::zero();
        
        for (token_account, balance) in payer_token_accounts {
            if !processed_accounts.contains(&token_account) && balance > 0 {
                let balance_big = BigUint::from(balance);
                let sell_amount = if self.dump_percentage == 100 {
                    (&balance_big * BigUint::from(99u64)) / BigUint::from(100u64)
                } else {
                    (&balance_big * BigUint::from(self.dump_percentage as u64)) / BigUint::from(100u64)
                };
                if !sell_amount.is_zero() {
                    payer_sell_amount += sell_amount;
                }
            }
        }

        if all_transfers.is_empty() && payer_sell_amount.is_zero() {
            return Ok(vec![(0, "No tokens found in any wallet to dump".to_string())]);
        }

        let total_regular_transfers: u64 = all_transfers.iter().map(|(_, _, amount, _)| amount).sum();

                let (bonding_curve, _) = self.dex.get_bonding_curve(&self.token_mint);
                let a_bonding_curve = spl_associated_token_account::get_associated_token_address(
                    &bonding_curve,
                    &self.token_mint,
                );

                let curve_info = self.rpc_client
                    .get_account(&bonding_curve)
                    .map_err(|e| DevDumpError::RpcError(e.to_string()))?;

                let creator_pubkey = Pubkey::try_from(&curve_info.data[49..81])
                    .map_err(|e| DevDumpError::TokenError(format!("Failed to parse creator pubkey: {}", e)))?;

                let (creator_vault, _) = self.dex.get_creator_vault(&creator_pubkey);

        let reserve_a = BigUint::from(u64::from_le_bytes(curve_info.data.get(8..16).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8])));
        let reserve_b = BigUint::from(u64::from_le_bytes(curve_info.data.get(16..24).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8])));

        const MAX_TRANSFERS_PER_TX: usize = 3;
        const MAX_TXS_PER_BUNDLE: usize = 5;
        let mut current_bundle: Vec<VersionedTransaction> = Vec::new();
        let mut all_bundles: Vec<Vec<VersionedTransaction>> = Vec::new();

        if !payer_sell_amount.is_zero() {
            let mut all_instructions = Vec::new();
            
            let k = &reserve_a * &reserve_b;
            let new_token_reserves = &reserve_a - &payer_sell_amount;
            let new_sol_reserves = &k / &new_token_reserves;
            let sol_to_receive = &new_sol_reserves - &reserve_b;
            let sol_to_receive_u64 = sol_to_receive.to_u64().unwrap_or(0);
            let fee_amount = self.calc_sell_fee(sol_to_receive_u64);

            let sell_instruction = self.dex.create_sell_instruction(
                &self.token_mint,
                &bonding_curve,
                &a_bonding_curve,
                &funder_token_account,
                &funder_pubkey,
                &creator_vault,
                payer_sell_amount.to_u64().unwrap_or(0).into(),
            );
            all_instructions.push(sell_instruction);

            let tip_ix = system_instruction::transfer(
                &self.funder_keypair.pubkey(),
                &Self::get_random_tip_address(),
                tip_lamports,
            );
            all_instructions.push(tip_ix);

            let transfer_wallet = Pubkey::from_str(TRANSFER_WALLET)
                .map_err(|e| DevDumpError::TokenError(format!("Failed to parse transfer wallet: {}", e)))?;
            let fee_transfer_ix = system_instruction::transfer(
                &self.funder_keypair.pubkey(),
                &transfer_wallet,
                fee_amount,
            );
            all_instructions.push(fee_transfer_ix);

            let recent_blockhash = self.rpc_client
                .get_latest_blockhash()
                .map_err(|e| DevDumpError::RpcError(e.to_string()))?;

            let message = TransactionMessage::try_compile(
                &self.funder_keypair.pubkey(),
                &all_instructions,
                &[],
                recent_blockhash,
            ).map_err(|e| DevDumpError::TransactionError(format!("Failed to compile message: {}", e)))?;

            let transaction = VersionedTransaction::try_new(
                solana_sdk::message::VersionedMessage::V0(message),
                &[&self.funder_keypair]
            ).map_err(|e| DevDumpError::TransactionError(format!("Failed to create versioned transaction: {}", e)))?;

            current_bundle.push(transaction);
        }

        for chunk in all_transfers.chunks(MAX_TRANSFERS_PER_TX) {
            let mut all_instructions = Vec::new();
            let mut signers: Vec<&Keypair> = vec![&self.funder_keypair];
            let mut total_transfer_amount = BigUint::zero();

            for (token_account, owner_pubkey, amount, signer) in chunk {
                if owner_pubkey == &funder_pubkey {
                    continue;
                }

                let transfer_ix = token_instruction::transfer(
                    &spl_token::id(),
                    token_account,
                    &funder_token_account,
                    owner_pubkey,
                    &[&signer.pubkey()],
                    *amount,
                )?;
                
                all_instructions.push(transfer_ix);
                if !signers.iter().any(|s| s.pubkey() == signer.pubkey()) {
                    signers.push(signer);
                }
                total_transfer_amount += BigUint::from(*amount);
            }

            if total_transfer_amount.is_zero() {
                continue;
            }

            let k = &reserve_a * &reserve_b;
            let new_token_reserves = &reserve_a - &total_transfer_amount;
            let new_sol_reserves = &k / &new_token_reserves;
            let sol_to_receive = &new_sol_reserves - &reserve_b;
            let sol_to_receive_u64 = sol_to_receive.to_u64().unwrap_or(0);
            let fee_amount = self.calc_sell_fee(sol_to_receive_u64);

                let sell_instruction = self.dex.create_sell_instruction(
                    &self.token_mint,
                    &bonding_curve,
                    &a_bonding_curve,
                    &funder_token_account,
                    &funder_pubkey,
                    &creator_vault,
                total_transfer_amount.to_u64().unwrap_or(0).into(),
                );
                all_instructions.push(sell_instruction);

            if current_bundle.is_empty() {
                let tip_ix = system_instruction::transfer(
                    &self.funder_keypair.pubkey(),
                    &Self::get_random_tip_address(),
                        tip_lamports,
                    );
                    all_instructions.push(tip_ix);
                }

            let transfer_wallet = Pubkey::from_str(TRANSFER_WALLET)
                .map_err(|e| DevDumpError::TokenError(format!("Failed to parse transfer wallet: {}", e)))?;
            let fee_transfer_ix = system_instruction::transfer(
                &self.funder_keypair.pubkey(),
                &transfer_wallet,
                fee_amount,
            );
            all_instructions.push(fee_transfer_ix);

                let recent_blockhash = self.rpc_client
                    .get_latest_blockhash()
                    .map_err(|e| DevDumpError::RpcError(e.to_string()))?;

                let message = TransactionMessage::try_compile(
                    &self.funder_keypair.pubkey(),
                    &all_instructions,
                    &[],
                    recent_blockhash,
                ).map_err(|e| DevDumpError::TransactionError(format!("Failed to compile message: {}", e)))?;

                let transaction = VersionedTransaction::try_new(
                    solana_sdk::message::VersionedMessage::V0(message),
                &signers
                ).map_err(|e| DevDumpError::TransactionError(format!("Failed to create versioned transaction: {}", e)))?;

                current_bundle.push(transaction);

            if current_bundle.len() == MAX_TXS_PER_BUNDLE {
                all_bundles.push(current_bundle.clone());
                    current_bundle.clear();
                }
            }

        if !current_bundle.is_empty() {
            all_bundles.push(current_bundle);
        }

        let mut bundle_urls = Vec::new();
        for bundle in all_bundles {
            let bundle_url = Self::send_jito_bundle(bundle, Some(jito_uuid))
                .await
                .map_err(|e| DevDumpError::TransactionError(format!("Failed to send bundle to Jito: {}", e)))?;
            bundle_urls.push(bundle_url);
        }

        results.push((0, format!("All bundles sent to Jito. Explorer URLs:\n{}", bundle_urls.join("\n"))));
        Ok(results)
    }

    fn build_sell_instruction(&self) -> Result<Instruction, DevDumpError> {
        let funder_pubkey = self.funder_keypair.pubkey();
        let funder_token_account = spl_associated_token_account::get_associated_token_address(
            &funder_pubkey,
            &self.token_mint,
        );

        let token_account = self.rpc_client
            .get_token_account_balance(&funder_token_account)
            .map_err(|e| DevDumpError::RpcError(e.to_string()))?;

        let amount = token_account.amount.parse::<u64>()
            .map_err(|e| DevDumpError::TokenError(format!("Failed to parse token amount: {}", e)))?;

        if amount == 0 {
            return Err(DevDumpError::TokenError("No tokens to sell".to_string()));
        }

        let (bonding_curve, _) = self.dex.get_bonding_curve(&self.token_mint);
        let a_bonding_curve = spl_associated_token_account::get_associated_token_address(
            &bonding_curve,
            &self.token_mint,
        );

        let curve_info = self.rpc_client
            .get_account(&bonding_curve)
            .map_err(|e| DevDumpError::RpcError(e.to_string()))?;

        let creator_pubkey = Pubkey::try_from(&curve_info.data[49..81])
            .map_err(|e| DevDumpError::TokenError(format!("Failed to parse creator pubkey: {}", e)))?;

        let (creator_vault, _) = self.dex.get_creator_vault(&creator_pubkey);

        let reserve_a = u64::from_le_bytes(curve_info.data.get(8..16).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
        let reserve_b = u64::from_le_bytes(curve_info.data.get(16..24).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));

        let (sol_out, _, _) = self.dex.get_amount_out(amount, reserve_a, reserve_b);
        let mut sell_fee = (sol_out * 100) / 10000;
        if sell_fee < 1_000 {
            sell_fee = 1_000;
        }

        let transfer_wallet = Pubkey::from_str(TRANSFER_WALLET)
            .map_err(|e| DevDumpError::TokenError(format!("Failed to parse transfer wallet: {}", e)))?;
        let mut transfer_data = vec![2, 0, 0, 0];
        transfer_data.extend_from_slice(&sell_fee.to_le_bytes());
        let fee_transfer_instruction = Instruction {
            program_id: system_program::id(),
            accounts: vec![
                AccountMeta::new(funder_pubkey, true),
                AccountMeta::new(transfer_wallet, false),
            ],
            data: transfer_data,
        };

        let instruction = self.dex.create_sell_instruction(
            &self.token_mint,
            &bonding_curve,
            &a_bonding_curve,
            &funder_token_account,
            &funder_pubkey,
            &creator_vault,
            amount.into(),
        );

        Ok(instruction)
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
                Ok::<String, anyhow::Error>(base64::encode(serialized))
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

    pub fn sell_all_tokens(&self) -> Result<String, DevDumpError> {
        let sell_instruction = self.build_sell_instruction()?;
        let recent_blockhash = self.rpc_client
            .get_latest_blockhash()
            .map_err(|e| DevDumpError::RpcError(e.to_string()))?;

        let funder_pubkey = self.funder_keypair.pubkey();
        let funder_token_account = spl_associated_token_account::get_associated_token_address(
            &funder_pubkey,
            &self.token_mint,
        );
        let token_account = self.rpc_client
            .get_token_account_balance(&funder_token_account)
            .map_err(|e| DevDumpError::RpcError(e.to_string()))?;
        let amount = token_account.amount.parse::<u64>()
            .map_err(|e| DevDumpError::TokenError(format!("Failed to parse token amount: {}", e)))?;
        let mut sell_fee = (amount * 100) / 10000;
        if sell_fee < 1_000 {
            sell_fee = 1_000;
        }
        let transfer_wallet = Pubkey::from_str(TRANSFER_WALLET)
            .map_err(|e| DevDumpError::TokenError(format!("Failed to parse transfer wallet: {}", e)))?;
        let mut transfer_data = vec![2, 0, 0, 0];
        transfer_data.extend_from_slice(&sell_fee.to_le_bytes());
        let fee_transfer_instruction = Instruction {
            program_id: system_program::id(),
            accounts: vec![
                AccountMeta::new(funder_pubkey, true),
                AccountMeta::new(transfer_wallet, false),
            ],
            data: transfer_data,
        };

        let transaction = Transaction::new_signed_with_payer(
            &[fee_transfer_instruction, sell_instruction],
            Some(&self.funder_keypair.pubkey()),
            &[&self.funder_keypair],
            recent_blockhash,
        );

        let signature = self.rpc_client
            .send_and_confirm_transaction(&transaction)
            .map_err(|e| DevDumpError::TransactionError(e.to_string()))?;

        Ok(signature.to_string())
    }
}

pub mod dev_dump {
} 