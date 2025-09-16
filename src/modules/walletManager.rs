use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    message::v0::Message as TransactionMessage,
    message::VersionedMessage,
    pubkey::Pubkey,
    signature::{Keypair, Signature},
    signer::Signer,
    system_instruction,
    transaction::VersionedTransaction,
    address_lookup_table_account::AddressLookupTableAccount,
};
use spl_token::{
    instruction as token_instruction,
    state::Account as TokenAccount,
};
use std::sync::Arc;
use tokio::sync::Mutex;
use anyhow::{Result, anyhow};
use rand::Rng;
use std::collections::HashMap;
use dotenv::dotenv;
use std::env;
use std::str::FromStr;
use serde::{Serialize, Deserialize};
use bs58;
use base64;
use solana_account_decoder::UiAccountData;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_program::program_pack::Pack;
use crate::modules::wallet_gen::WalletGenerator;
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde_json;
use spl_associated_token_account;
use solana_address_lookup_table_program::{
    instruction::{deactivate_lookup_table, close_lookup_table},
    state::AddressLookupTable,
};
use solana_client::rpc_config::{RpcProgramAccountsConfig, RpcAccountInfoConfig, RpcSendTransactionConfig};
use solana_client::rpc_filter::RpcFilterType;
use solana_client::rpc_response::RpcKeyedAccount;
use solana_account_decoder::UiAccountEncoding;
use std::thread;
use std::time::Duration;
use std::io;

const BLACK_BG: &str = "\x1b[40m";
const GREEN: &str = "\x1b[32m";
const BRIGHT_CYAN: &str = "\x1b[96m";
const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const BRIGHT_YELLOW: &str = "\x1b[93m";
const MAGENTA: &str = "\x1b[35m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Wallet {
    pub pubkey: String,
    pub privkey: String,
    pub balance: f64,
}

#[derive(Debug, Serialize, Deserialize)]
struct MakerWallet {
    pubkey: String,
    private_key: String,
    token_mint: String,
    ata: String,
}

pub struct WalletManager {
    rpc_client: RpcClient,
    wallets: Arc<Mutex<Vec<Wallet>>>,
    payer: Keypair,
}

impl WalletManager {
    fn get_payer_from_env() -> Result<Keypair> {
        dotenv().ok();
        let payer_privkey = env::var("PAYER").map_err(|_| anyhow!("Error: PAYER not set in .env file"))?;
        let bytes = bs58::decode(&payer_privkey).into_vec()?;
        Keypair::from_bytes(&bytes).map_err(|e| anyhow!("Error: Failed to create keypair: {}", e))
    }

    pub async fn new(rpc_url: String) -> Result<Self> {
        let rpc_client = RpcClient::new_with_timeout(
            rpc_url,
            Duration::from_secs(30)
        );
        let payer = Self::get_payer_from_env()?;
        let wallets = Arc::new(Mutex::new(Vec::new()));
        
        Ok(Self {
            rpc_client,
            wallets,
            payer,
        })
    }

    pub async fn get_wallets(&self) -> Result<Vec<Wallet>> {
        let wallets = self.wallets.lock().await;
        Ok(wallets.clone())
    }

    pub async fn import_wallet(&self, privkey: &str) -> Result<()> {
        let bytes = bs58::decode(privkey).into_vec()?;
        let keypair = Keypair::from_bytes(&bytes)?;
        let pubkey = keypair.pubkey().to_string();

        let mut wallets = self.wallets.lock().await;
        if !wallets.iter().any(|w| w.pubkey == pubkey) {
            wallets.push(Wallet {
                pubkey,
                privkey: privkey.to_string(),
                balance: 0.0,
            });
        }
        Ok(())
    }

    pub async fn backup_wallets(&self) -> Result<String> {
        let wallets = self.wallets.lock().await;
        let mut result = String::new();
        for (i, wallet) in wallets.iter().enumerate() {
            result.push_str(&format!("{}. Public Key: {}\n", i + 1, wallet.pubkey));
            result.push_str(&format!("   Private Key: {}\n", wallet.privkey));
        }
        Ok(result)
    }

    pub async fn get_balances_string(&self) -> Result<String> {
        let mut result = String::new();
        
        let header = format!("{}{}{}=== Wallet Address | SOL Balance | Token Balance ==={}{}", 
            BLACK_BG, GREEN, "=".repeat(20), "=".repeat(20), RESET);
        result.push_str(&format!("{}\n", header));
        
        if let Ok(contents) = std::fs::read_to_string("wallets/wallets.json") {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(wallets) = data["wallets"].as_array() {
                    result.push_str(&format!("\n{}{}=== Main Wallets ==={}\n", BLACK_BG, BRIGHT_CYAN, RESET));
                    for wallet in wallets {
                        if let (Some(pubkey), Some(_)) = (wallet["pubkey"].as_str(), wallet["privkey"].as_str()) {
                            match Pubkey::from_str(pubkey) {
                                Ok(pubkey) => {
                                    let sol_balance = match self.rpc_client.get_balance(&pubkey) {
                                        Ok(balance) => balance,
                                        Err(e) => {
                                            result.push_str(&format!("{}{}{} | {}Error getting SOL balance: {}\n",
                                                CYAN, pubkey, RESET,
                                                RED, e));
                                            continue;
                                        }
                                    };

                                    let token_accounts = match self.rpc_client.get_token_accounts_by_owner(
                                        &pubkey,
                                        TokenAccountsFilter::ProgramId(spl_token::id()),
                                    ) {
                                        Ok(accounts) => accounts,
                                        Err(e) => {
                                            result.push_str(&format!("{}{}{} | {}{:.4} SOL{} | {}Error getting token accounts: {}\n",
                                                CYAN, pubkey, RESET,
                                                YELLOW, sol_balance as f64 / 1e9, RESET,
                                                RED, e));
                                            continue;
                                        }
                                    };

                                    let mut total_token_balance: f64 = 0.0;
                                    let mut token_details = Vec::new();
                                    
                                    for account in token_accounts {
                                        if let Ok(account_pubkey) = Pubkey::from_str(&account.pubkey.to_string()) {
                                            if let Ok(balance) = self.rpc_client.get_token_account_balance(&account_pubkey) {
                                                if let Some(amount) = balance.ui_amount {
                                                    if amount > 0.0 {
                                                        total_token_balance += amount;
                                                        if let UiAccountData::Binary(data, _) = &account.account.data {
                                                            if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(data) {
                                                                if let Ok(parsed_account) = TokenAccount::unpack(&decoded) {
                                                                    token_details.push(format!("{:.4} (Mint: {})", 
                                                                        amount, 
                                                                        parsed_account.mint));
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    if !token_details.is_empty() {
                                        result.push_str(&format!("{}{}{} | {}{:.4} SOL{} | {}{:.4} Tokens{}\n",
                                            CYAN, pubkey, RESET,
                                            YELLOW, sol_balance as f64 / 1e9, RESET,
                                            MAGENTA, total_token_balance, RESET));
                                        for detail in token_details {
                                            result.push_str(&format!("    {}{}{}\n", 
                                                MAGENTA, detail, RESET));
                                        }
                                    } else {
                                        result.push_str(&format!("{}{}{} | {}{:.4} SOL{} | {}{:.4} Tokens{}\n",
                                            CYAN, pubkey, RESET,
                                            YELLOW, sol_balance as f64 / 1e9, RESET,
                                            MAGENTA, total_token_balance, RESET));
                                    }
                                }
                                Err(e) => {
                                    result.push_str(&format!("{}{}{} | {}Error: {}\n",
                                        CYAN, pubkey, RESET,
                                        RED, e.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Ok(contents) = std::fs::read_to_string("wallets/makers.json") {
            if let Ok(wallets) = serde_json::from_str::<Vec<MakerWallet>>(&contents) {
                result.push_str(&format!("\n{}{}=== Maker Wallets ==={}\n", BLACK_BG, BRIGHT_CYAN, RESET));
                for wallet in wallets {
                    match Pubkey::from_str(&wallet.pubkey) {
                        Ok(pubkey) => {
                            let sol_balance = match self.rpc_client.get_balance(&pubkey) {
                                Ok(balance) => balance,
                                Err(e) => {
                                    result.push_str(&format!("{}{}{} | {}Error getting SOL balance: {}\n",
                                        CYAN, pubkey, RESET,
                                        RED, e));
                                    continue;
                                }
                            };

                            let token_accounts = match self.rpc_client.get_token_accounts_by_owner(
                                &pubkey,
                                TokenAccountsFilter::ProgramId(spl_token::id()),
                            ) {
                                Ok(accounts) => accounts,
                                Err(e) => {
                                    result.push_str(&format!("{}{}{} | {}{:.4} SOL{} | {}Error getting token accounts: {}\n",
                                        CYAN, pubkey, RESET,
                                        YELLOW, sol_balance as f64 / 1e9, RESET,
                                        RED, e));
                                    continue;
                                }
                            };

                            let mut total_token_balance: f64 = 0.0;
                            let mut token_details = Vec::new();
                            
                            for account in token_accounts {
                                if let Ok(account_pubkey) = Pubkey::from_str(&account.pubkey.to_string()) {
                                    if let Ok(balance) = self.rpc_client.get_token_account_balance(&account_pubkey) {
                                        if let Some(amount) = balance.ui_amount {
                                            if amount > 0.0 {
                                                total_token_balance += amount;
                                                if let UiAccountData::Binary(data, _) = &account.account.data {
                                                    if let Ok(decoded) = base64::engine::general_purpose::STANDARD.decode(data) {
                                                        if let Ok(parsed_account) = TokenAccount::unpack(&decoded) {
                                                            token_details.push(format!("{:.4} (Mint: {})", 
                                                                amount, 
                                                                parsed_account.mint));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            if !token_details.is_empty() {
                                result.push_str(&format!("{}{}{} | {}{:.4} SOL{} | {}{:.4} Tokens{}\n",
                                    CYAN, pubkey, RESET,
                                    YELLOW, sol_balance as f64 / 1e9, RESET,
                                    MAGENTA, total_token_balance, RESET));
                                for detail in token_details {
                                    result.push_str(&format!("    {}{}{}\n", 
                                        MAGENTA, detail, RESET));
                                }
                            } else {
                                result.push_str(&format!("{}{}{} | {}{:.4} SOL{} | {}{:.4} Tokens{}\n",
                                    CYAN, pubkey, RESET,
                                    YELLOW, sol_balance as f64 / 1e9, RESET,
                                    MAGENTA, total_token_balance, RESET));
                            }
                        }
                        Err(e) => {
                            result.push_str(&format!("{}{}{} | {}Error: {}\n",
                                CYAN, wallet.pubkey, RESET,
                                RED, e.to_string()));
                        }
                    }
                }
            }
        }
        
        Ok(result)
    }

    pub async fn fund_wallets(&self, amount_lamports: u64) -> Result<String> {
        let contents = std::fs::read_to_string("wallets/wallets.json")
            .map_err(|_| anyhow!("Error: Failed to read wallets.json"))?;
        
        let data = serde_json::from_str::<serde_json::Value>(&contents)
            .map_err(|_| anyhow!("Error: Failed to parse wallets.json"))?;
        
        let wallets = data["wallets"].as_array()
            .ok_or_else(|| anyhow!("Error: No wallets found"))?;
        
        if wallets.is_empty() {
            return Ok("Error: No wallets available".to_string());
        }

        let mut instructions = Vec::new();
        let total_amount = amount_lamports * wallets.len() as u64;
        
        for wallet in wallets {
            if let (Some(pubkey), Some(_)) = (wallet["pubkey"].as_str(), wallet["privkey"].as_str()) {
                let to_pubkey = Pubkey::from_str(pubkey)?;
                let instruction = system_instruction::transfer(
                    &self.payer.pubkey(),
                    &to_pubkey,
                    amount_lamports,
                );
                instructions.push(instruction);
            }
        }

        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let message = TransactionMessage::try_compile(
            &self.payer.pubkey(),
            &instructions,
            &[],
            recent_blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[&self.payer]
        )?;

        match self.rpc_client.send_transaction(&transaction) {
            Ok(signature) => {
                let total_sol = total_amount as f64 / 1e9;
                let amount_sol = amount_lamports as f64 / 1e9;
                Ok(format!(
                    "TXID: {}\nAmount: {:.3} SOL\nTotal: {:.3} SOL",
                    signature,
                    amount_sol,
                    total_sol
                ))
            }
            Err(e) => Err(anyhow!("Error: {}", e)),
        }
    }

    pub async fn fund_wallets_range(&self, min_amount: u64, max_amount: u64) -> Result<String> {
        let contents = std::fs::read_to_string("wallets/wallets.json")
            .map_err(|_| anyhow!("Error: Failed to read wallets.json"))?;
        
        let data = serde_json::from_str::<serde_json::Value>(&contents)
            .map_err(|_| anyhow!("Error: Failed to parse wallets.json"))?;
        
        let wallets = data["wallets"].as_array()
            .ok_or_else(|| anyhow!("Error: No wallets found"))?;
        
        if wallets.is_empty() {
            return Ok("Error: No wallets available".to_string());
        }

        let mut rng = StdRng::from_entropy();
        let mut instructions = Vec::new();
        let mut total_amount = 0u64;
        let mut wallet_pubkeys = Vec::new();
        let mut amounts = Vec::new();

        for wallet in wallets {
            if let (Some(pubkey), Some(_)) = (wallet["pubkey"].as_str(), wallet["privkey"].as_str()) {
                let amount = rng.gen_range(min_amount, max_amount + 1);
                total_amount += amount;
                let to_pubkey = Pubkey::from_str(pubkey)?;
                wallet_pubkeys.push(to_pubkey);
                amounts.push(amount);
                instructions.push(system_instruction::transfer(
                    &self.payer.pubkey(),
                    &to_pubkey,
                    amount,
                ));
            }
        }

        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let message = TransactionMessage::try_compile(
            &self.payer.pubkey(),
            &instructions,
            &[],
            recent_blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[&self.payer],
        )?;

        match self.rpc_client.send_transaction(&transaction) {
            Ok(signature) => {
                let total_sol = total_amount as f64 / 1e9;
                let mut result = format!("TXID: {}\nTotal: {:.6} SOL", signature, total_sol);
                for (i, (pubkey, amount)) in wallet_pubkeys.iter().zip(amounts.iter()).enumerate() {
                    result.push_str(&format!("\n{}. {}: {:.6} SOL", i + 1, pubkey, *amount as f64 / 1e9));
                }
                Ok(result)
            }
            Err(e) => Err(anyhow!("Error: {}", e)),
        }
    }

    pub async fn fund_wallets_with_payer(&self, amount_lamports: u64, payer: &Keypair) -> Result<String> {
        let mut result = String::new();
        let wallets = self.wallets.lock().await;
        
        for (i, wallet) in wallets.iter().enumerate() {
            let to_pubkey = Pubkey::from_str(&wallet.pubkey)?;
            let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
            
            let message = TransactionMessage::try_compile(
                &payer.pubkey(),
                &[system_instruction::transfer(
                    &payer.pubkey(),
                    &to_pubkey,
                    amount_lamports,
                )],
                &[],
                recent_blockhash,
            )?;
            
            let transaction = VersionedTransaction::try_new(
                VersionedMessage::V0(message),
                &[payer],
            )?;
            
            match self.rpc_client.send_transaction(&transaction) {
                Ok(signature) => {
                    result.push_str(&format!("TXID: {}\n", signature));
                }
                Err(e) => {
                    result.push_str(&format!("Error: {}\n", e));
                }
            }
        }
        
        Ok(result)
    }

    pub async fn fund_wallets_range_with_payer(&self, min_amount: u64, max_amount: u64, payer: &Keypair) -> Result<String> {
        let mut rng = rand::thread_rng();
        let wallets = self.wallets.lock().await;
        let mut instructions = Vec::new();
        let mut total_amount = 0u64;
        let mut result = String::new();

        for wallet in wallets.iter() {
            let amount = rng.gen_range(min_amount, max_amount + 1);
            total_amount += amount;
            let to_pubkey = Pubkey::from_str(&wallet.pubkey)?;
            let instruction = system_instruction::transfer(
                &payer.pubkey(),
                &to_pubkey,
                amount,
            );
            instructions.push(instruction);
        }

        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let message = TransactionMessage::try_compile(
            &payer.pubkey(),
            &instructions,
            &[],
            recent_blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[payer]
        )?;

        match self.rpc_client.send_transaction(&transaction) {
            Ok(signature) => {
                let total_sol = total_amount as f64 / 1e9;
                result.push_str(&format!("TXID: {}\nTotal: {:.6} SOL", signature, total_sol));
                Ok(result)
            }
            Err(e) => Err(anyhow!("Error: {}", e)),
        }
    }

    pub async fn withdraw_from_all(&self) -> Result<Vec<String>> {
        let mut result = Vec::new();
        let mut total_withdrawn = 0u64;
        const MAX_WALLETS_PER_TX: usize = 4;
        let mut signatures = Vec::new();
        let mut wallets_with_balance = Vec::new();
        let mut processed_pubkeys = std::collections::HashSet::new();
        let wsol_mint = Pubkey::from_str("So11111111111111111111111111111111111111112")?;

        if let Ok(contents) = std::fs::read_to_string("wallets/wallets.json") {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(wallets) = data["wallets"].as_array() {
                    for wallet in wallets {
                        if let (Some(pubkey), Some(privkey)) = (wallet["pubkey"].as_str(), wallet["privkey"].as_str()) {
                            if !processed_pubkeys.contains(pubkey) {
                                let from_pubkey = Pubkey::from_str(pubkey)?;
                                let balance = self.rpc_client.get_balance(&from_pubkey)?;
                                
                                let wsol_ata = spl_associated_token_account::get_associated_token_address(&from_pubkey, &wsol_mint);
                                let wsol_balance = match self.rpc_client.get_token_account_balance(&wsol_ata) {
                                    Ok(balance) => (balance.ui_amount.unwrap_or(0.0) * 1e9) as u64,
                                    Err(_) => 0,
                                };

                                if balance > 0 || wsol_balance > 0 {
                                    let from_keypair = Keypair::from_bytes(&bs58::decode(privkey).into_vec()?)?;
                                    wallets_with_balance.push((from_pubkey, from_keypair, balance, wsol_balance, wsol_ata));
                                    result.push(format!("TXID: Found balance in wallet {}: {:.3} SOL, {:.3} WSOL", 
                                        pubkey, balance as f64 / 1e9, wsol_balance as f64 / 1e9));
                                    processed_pubkeys.insert(pubkey.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Ok(contents) = std::fs::read_to_string("wallets/makers.json") {
            if let Ok(wallets) = serde_json::from_str::<Vec<MakerWallet>>(&contents) {
                for wallet in wallets {
                    if !processed_pubkeys.contains(&wallet.pubkey) {
                        let from_pubkey = Pubkey::from_str(&wallet.pubkey)?;
                        let balance = self.rpc_client.get_balance(&from_pubkey)?;
                        
                        let wsol_ata = spl_associated_token_account::get_associated_token_address(&from_pubkey, &wsol_mint);
                        let wsol_balance = match self.rpc_client.get_token_account_balance(&wsol_ata) {
                            Ok(balance) => (balance.ui_amount.unwrap_or(0.0) * 1e9) as u64,
                            Err(_) => 0,
                        };

                        if balance > 0 || wsol_balance > 0 {
                            let from_keypair = Keypair::from_bytes(&bs58::decode(&wallet.private_key).into_vec()?)?;
                            wallets_with_balance.push((from_pubkey, from_keypair, balance, wsol_balance, wsol_ata));
                            result.push(format!("TXID: Found balance in maker wallet {}: {:.3} SOL, {:.3} WSOL", 
                                wallet.pubkey, balance as f64 / 1e9, wsol_balance as f64 / 1e9));
                            processed_pubkeys.insert(wallet.pubkey.clone());
                        }
                    }
                }
            }
        }

        if let Ok(contents) = std::fs::read_to_string("wallets/mixer.json") {
            if let Ok(mixer_wallets) = serde_json::from_str::<Vec<serde_json::Value>>(&contents) {
                for wallet in mixer_wallets {
                    let pubkey = wallet.get("pubkey").and_then(|v| v.as_str());
                    let privkey = wallet.get("private_key").or_else(|| wallet.get("privkey")).and_then(|v| v.as_str());
                    if let (Some(pubkey), Some(privkey)) = (pubkey, privkey) {
                        if !processed_pubkeys.contains(pubkey) {
                            let from_pubkey = Pubkey::from_str(pubkey)?;
                            let balance = self.rpc_client.get_balance(&from_pubkey)?;
                            let wsol_ata = spl_associated_token_account::get_associated_token_address(&from_pubkey, &wsol_mint);
                            let wsol_balance = match self.rpc_client.get_token_account_balance(&wsol_ata) {
                                Ok(balance) => (balance.ui_amount.unwrap_or(0.0) * 1e9) as u64,
                                Err(_) => 0,
                            };
                            if balance > 0 || wsol_balance > 0 {
                                let from_keypair = Keypair::from_bytes(&bs58::decode(privkey).into_vec()?)?;
                                wallets_with_balance.push((from_pubkey, from_keypair, balance, wsol_balance, wsol_ata));
                                processed_pubkeys.insert(pubkey.to_string());
                            }
                        }
                    }
                }
            }
        }

        if wallets_with_balance.is_empty() {
            return Err(anyhow!("Error: No wallets had sufficient balance"));
        }

        for (batch_idx, chunk) in wallets_with_balance.chunks(MAX_WALLETS_PER_TX).enumerate() {
            let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
            let mut instructions = Vec::new();
            let mut signers = vec![&self.payer];

            for (pubkey, keypair, balance, wsol_balance, wsol_ata) in chunk {
                signers.push(keypair);
                
                if *balance > 0 {
                    instructions.push(system_instruction::transfer(
                        pubkey,
                        &self.payer.pubkey(),
                        *balance,
                    ));
                    total_withdrawn += balance;
                }

                if *wsol_balance > 0 {
                    instructions.push(spl_token::instruction::sync_native(
                        &spl_token::id(),
                        wsol_ata,
                    )?);

                    instructions.push(spl_token::instruction::close_account(
                        &spl_token::id(),
                        wsol_ata,
                        &self.payer.pubkey(),
                        pubkey,
                        &[],
                    )?);

                    total_withdrawn += wsol_balance;
                }
            }

            if instructions.is_empty() {
                continue;
            }

            let message = TransactionMessage::try_compile(
                &self.payer.pubkey(),
                &instructions,
                &[],
                recent_blockhash,
            )?;

            let transaction = VersionedTransaction::try_new(
                VersionedMessage::V0(message),
                &signers
            )?;

            match self.rpc_client.send_transaction(&transaction) {
                Ok(signature) => {
                    signatures.push(signature.to_string());
                }
                Err(e) => {
                    return Err(anyhow!("Error: {}", e));
                }
            }
        }

        let total_sol = total_withdrawn as f64 / 1e9;
        Ok(vec![format!(
            "TXID: {}\nTotal: {:.6} SOL",
            signatures.join("\n"),
            total_sol
        )])
    }

    pub async fn withdraw_from_wallet(&self, wallet_index: usize, amount_lamports: u64) -> Result<String> {
        let mut wallets = Vec::new();
        
        if let Ok(contents) = std::fs::read_to_string("wallets/wallets.json") {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(json_wallets) = data["wallets"].as_array() {
                    for wallet in json_wallets {
                        if let (Some(pubkey), Some(privkey)) = (wallet["pubkey"].as_str(), wallet["privkey"].as_str()) {
                            wallets.push((pubkey.to_string(), privkey.to_string()));
                        }
                    }
                }
            }
        }

        if let Ok(contents) = std::fs::read_to_string("wallets/makers.json") {
            if let Ok(maker_wallets) = serde_json::from_str::<Vec<MakerWallet>>(&contents) {
                for wallet in maker_wallets {
                    wallets.push((wallet.pubkey, wallet.private_key));
                }
            }
        }

        if wallet_index >= wallets.len() {
            return Err(anyhow!("Error: Invalid wallet index"));
        }
        
        let (pubkey, privkey) = &wallets[wallet_index];
        let from_pubkey = Pubkey::from_str(pubkey)?;
        let from_keypair = Keypair::from_bytes(&bs58::decode(privkey).into_vec()?)?;
        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        
        let message = TransactionMessage::try_compile(
            &self.payer.pubkey(),
            &[system_instruction::transfer(
                &from_pubkey,
                &self.payer.pubkey(),
                amount_lamports,
            )],
            &[],
            recent_blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[&self.payer, &from_keypair],
        )?;

        let signature = self.rpc_client.send_transaction(&transaction)?;
        Ok(format!("TXID: {}", signature))
    }

    pub async fn fund_wallets_batch(&self, amount_lamports: u64) -> Result<String> {
        let contents = std::fs::read_to_string("wallets/wallets.json")
            .map_err(|_| anyhow!("Error: Failed to read wallets.json"))?;
        
        let data = serde_json::from_str::<serde_json::Value>(&contents)
            .map_err(|_| anyhow!("Error: Failed to parse wallets.json"))?;
        
        let wallets = data["wallets"].as_array()
            .ok_or_else(|| anyhow!("Error: No wallets found"))?;
        
        if wallets.is_empty() {
            return Ok("Error: No wallets available".to_string());
        }

        let mut result = String::new();
        
        for chunk in wallets.chunks(20) {
            let mut instructions = Vec::new();
            let mut wallet_pubkeys = Vec::new();
            
            for wallet in chunk {
                if let (Some(pubkey), Some(_)) = (wallet["pubkey"].as_str(), wallet["privkey"].as_str()) {
                    let to_pubkey = Pubkey::from_str(pubkey)?;
                    wallet_pubkeys.push(to_pubkey);
                    instructions.push(system_instruction::transfer(
                        &self.payer.pubkey(),
                        &to_pubkey,
                        amount_lamports,
                    ));
                }
            }
            
            let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
            
            let message = TransactionMessage::try_compile(
                &self.payer.pubkey(),
                &instructions,
                &[],
                recent_blockhash,
            )?;
            
            let transaction = VersionedTransaction::try_new(
                VersionedMessage::V0(message),
                &[&self.payer],
            )?;
            
            match self.rpc_client.send_transaction(&transaction) {
                Ok(signature) => {
                    result.push_str(&format!("TXID: {}\n", signature));
                }
                Err(e) => {
                    result.push_str(&format!("Error: {}\n", e));
                }
            }
        }
        
        Ok(result)
    }

    pub async fn fund_wallets_range_batch(&self, min_amount: u64, max_amount: u64) -> Result<String> {
        let contents = std::fs::read_to_string("wallets/wallets.json")
            .map_err(|_| anyhow!("Error: Failed to read wallets.json"))?;
        
        let data = serde_json::from_str::<serde_json::Value>(&contents)
            .map_err(|_| anyhow!("Error: Failed to parse wallets.json"))?;
        
        let wallets = data["wallets"].as_array()
            .ok_or_else(|| anyhow!("Error: No wallets found"))?;
        
        if wallets.is_empty() {
            return Ok("Error: No wallets available".to_string());
        }

        let mut result = String::new();
        let mut rng = StdRng::from_entropy();
        
        for chunk in wallets.chunks(20) {
            let mut instructions = Vec::new();
            let mut wallet_pubkeys = Vec::new();
            let mut amounts = Vec::new();
            
            for wallet in chunk {
                if let (Some(pubkey), Some(_)) = (wallet["pubkey"].as_str(), wallet["privkey"].as_str()) {
                    let amount = rng.gen_range(min_amount, max_amount + 1);
                    let to_pubkey = Pubkey::from_str(pubkey)?;
                    wallet_pubkeys.push(to_pubkey);
                    amounts.push(amount);
                    instructions.push(system_instruction::transfer(
                        &self.payer.pubkey(),
                        &to_pubkey,
                        amount,
                    ));
                }
            }
            
            let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
            
            let message = TransactionMessage::try_compile(
                &self.payer.pubkey(),
                &instructions,
                &[],
                recent_blockhash,
            )?;
            
            let transaction = VersionedTransaction::try_new(
                VersionedMessage::V0(message),
                &[&self.payer],
            )?;
            
            match self.rpc_client.send_transaction(&transaction) {
                Ok(signature) => {
                    result.push_str(&format!("TXID: {}\n", signature));
                }
                Err(e) => {
                    result.push_str(&format!("Error: {}\n", e));
                }
            }
        }
        
        Ok(result)
    }

    pub fn get_payer_pubkey(&self) -> Pubkey {
        self.payer.pubkey()
    }

    pub async fn fund_wallets_batch_with_instructions(&self, instructions: Vec<Instruction>) -> Result<String> {
        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let message = TransactionMessage::try_compile(
            &self.payer.pubkey(),
            &instructions,
            &[],
            recent_blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[&self.payer],
        )?;

        match self.rpc_client.send_transaction(&transaction) {
            Ok(signature) => Ok(format!("TXID: {}", signature)),
            Err(e) => Err(anyhow!("Error: {}", e)),
        }
    }

    pub async fn close_lut(&self, lut_address: &str) -> Result<String> {
        let result = (|| -> Result<String> {
            let lut_pubkey = Pubkey::from_str(lut_address)?;
            
            // First check if the account exists
            match self.rpc_client.get_account(&lut_pubkey) {
                Ok(account) => {
                    // Check if account is already closed (data length should be 0)
                    if account.data.is_empty() {
                        println!("{}LUT is already closed{}", GREEN, RESET);
                        return Ok("LUT already closed".to_string());
                    }

                    let table = AddressLookupTable::deserialize(&account.data)
                        .map_err(|e| anyhow!("Failed to deserialize LUT: {}", e))?;

                    if let Some(authority) = table.meta.authority {
                        if authority != self.payer.pubkey() {
                            return Err(anyhow!("You are not the authority of this LUT. Authority: {}", authority));
                        }
                    } else {
                        return Err(anyhow!("This LUT has no authority"));
                    }

                    let lut_account = AddressLookupTableAccount {
                        key: lut_pubkey,
                        addresses: table.addresses.to_vec(),
                    };

                    // Check if already deactivated
                    if table.meta.deactivation_slot == u64::MAX {
                        // Need to deactivate first
                        let deactivate_ix = deactivate_lookup_table(
                            lut_pubkey,
                            self.payer.pubkey(),
                        );

                        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
                        let message = TransactionMessage::try_compile(
                            &self.payer.pubkey(),
                            &[deactivate_ix],
                            &[lut_account.clone()],
                            recent_blockhash,
                        )?;

                        let transaction = VersionedTransaction::try_new(
                            VersionedMessage::V0(message),
                            &[&self.payer],
                        )?;

                        let deactivate_sig = self.rpc_client.send_and_confirm_transaction_with_spinner_and_config(
                            &transaction,
                            CommitmentConfig::confirmed(),
                            RpcSendTransactionConfig {
                                skip_preflight: true,
                                preflight_commitment: Some(CommitmentConfig::confirmed().commitment),
                                encoding: None,
                                max_retries: Some(5),
                                min_context_slot: None
                            }
                        )?;

                        println!("{}Deactivated LUT: {}{}", GREEN, deactivate_sig, RESET);

                        // Wait for deactivation to be confirmed
                        let mut retries = 0;
                        while retries < 10 {
                            match self.rpc_client.get_account(&lut_pubkey) {
                                Ok(account) => {
                                    let table = AddressLookupTable::deserialize(&account.data)
                                        .map_err(|e| anyhow!("Failed to deserialize LUT: {}", e))?;
                                    if table.meta.deactivation_slot != u64::MAX {
                                        break;
                                    }
                                }
                                Err(_) => {}
                            }
                            thread::sleep(Duration::from_secs(1));
                            retries += 1;
                        }
                    } else {
                        println!("{}LUT already deactivated{}", GREEN, RESET);
                    }

                    // Now close
                    let close_ix = close_lookup_table(
                        lut_pubkey,
                        self.payer.pubkey(),
                        self.payer.pubkey(),
                    );

                    let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
                    let message = TransactionMessage::try_compile(
                        &self.payer.pubkey(),
                        &[close_ix],
                        &[lut_account],
                        recent_blockhash,
                    )?;

                    let transaction = VersionedTransaction::try_new(
                        VersionedMessage::V0(message),
                        &[&self.payer],
                    )?;

                    let signature = self.rpc_client.send_and_confirm_transaction_with_spinner_and_config(
                        &transaction,
                        CommitmentConfig::confirmed(),
                        RpcSendTransactionConfig {
                            skip_preflight: true,
                            preflight_commitment: Some(CommitmentConfig::confirmed().commitment),
                            encoding: None,
                            max_retries: Some(5),
                            min_context_slot: None
                        }
                    )?;

                    println!("{}Closed LUT: {}{}", GREEN, signature, RESET);
                    Ok(format!("TXID: {}", signature))
                }
                Err(_) => {
                    // Account doesn't exist, so it's already closed
                    println!("{}LUT is already closed{}", GREEN, RESET);
                    Ok("LUT already closed".to_string())
                }
            }
        })();

        match &result {
            Ok(_) => println!("\n{}Press Enter to return to menu...{}", BRIGHT_CYAN, RESET),
            Err(e) => println!("\n{}Error: {}{}\n{}Press Enter to return to menu...{}", 
                RED, e, RESET, BRIGHT_CYAN, RESET),
        }

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        result
    }

    /// Funds the DEV wallet from PAYER with a user-specified amount of SOL.
    pub async fn dev_fund(&self) -> Result<String> {
        use std::io::{self, Write};
        dotenv().ok();
        let dev_privkey = std::env::var("DEV").map_err(|_| anyhow!("DEV not set in .env file"))?;
        let dev_bytes = bs58::decode(&dev_privkey).into_vec()?;
        let dev_keypair = Keypair::from_bytes(&dev_bytes)?;
        let dev_pubkey = dev_keypair.pubkey();

        print!("Enter amount of SOL to fund the DEV wallet: ");
        io::stdout().flush()?;
        let mut amount = String::new();
        io::stdin().read_line(&mut amount)?;
        let amount: f64 = amount.trim().parse().map_err(|e| anyhow!("Invalid amount: {}", e))?;
        let amount_lamports = (amount * 1e9) as u64;

        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let message = TransactionMessage::try_compile(
            &self.payer.pubkey(),
            &[system_instruction::transfer(&self.payer.pubkey(), &dev_pubkey, amount_lamports)],
            &[],
            recent_blockhash,
        )?;
        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[&self.payer],
        )?;
        let signature = self.rpc_client.send_transaction(&transaction)?;
        let solscan_url = format!("https://solscan.io/tx/{}", signature);
        Ok(format!("Sent {:.6} SOL to DEV wallet {}\nSolscan: {}", amount, dev_pubkey, solscan_url))
    }
} 