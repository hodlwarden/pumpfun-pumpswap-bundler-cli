use serde::{Serialize, Deserialize};
use std::fs;
use anyhow::anyhow;
use solana_client::rpc_client::RpcClient;
use solana_program::{
    pubkey::Pubkey,
    message::v0::Message as TransactionMessage,
    program_pack::Pack,
    instruction::{AccountMeta, Instruction},
    system_program,
};
use solana_sdk::{
    commitment_config::{CommitmentConfig, CommitmentLevel}, compute_budget::ComputeBudgetInstruction, message::VersionedMessage, signature::{read_keypair_file, Keypair, Signer}, system_instruction, transaction::{Transaction, VersionedTransaction}
};
use spl_token::{
    state::Account as TokenAccount,
    instruction as token_instruction,
    ID as TOKEN_PROGRAM_ID,
    native_mint::ID as NATIVE_MINT,
};
use solana_client::rpc_request::TokenAccountsFilter;
use solana_client::rpc_config::RpcTokenAccountsFilter;
use std::str::FromStr;
use crate::modules::wallet_gen::WalletGenerator;
use crate::dex::pump::PumpDex;
use crate::dex::pumpswap::PumpSwap;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_config::RpcAccountInfoConfig;
use spl_associated_token_account::{
    get_associated_token_address,
    instruction::create_associated_token_account_idempotent,
    ID as ASSOCIATED_TOKEN_PROGRAM_ID,
};
use std::time::Duration;
use tokio::time::sleep;
use num_bigint::BigUint;
use num_traits::{One, Zero, ToPrimitive};

const PUMP_AMM_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
const PUMP_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const GLOBAL_CONFIG: &str = "ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw";
const PROTOCOL_FEE_RECIPIENT: &str = "62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV";
const PROTOCOL_FEE_ACCOUNT: &str = "7xQYoUjUJF1Kg6WVczoTAkaNhn5syQYcbvjmFrhjWpx";
const EVENT_AUTH: &str = "GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR";
const SYSTEM_PROGRAM_ID: Pubkey = system_program::id();
const COMPUTE_UNIT_LIMIT_POWER_BUMP: u32 = 1_400_000;
const OWNER_WALLET: &str = "FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT";

const PUMP_GLOBAL: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
const PUMP_FEE_RECIPIENT: &str = "CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM";
const PUMP_EVENT_AUTHORITY: &str = "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1";

#[derive(Debug, Serialize, Deserialize, Clone)]
struct UserWallet {
    pubkey: String,
    privkey: String,
    balance: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct UserData {
    user_id: i64,
    username: String,
    wallets: Vec<UserWallet>,
    last_activity: String,
    funder_pubkey: Option<String>,
    funder_privkey: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct UsersData {
    users: std::collections::HashMap<String, UserData>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MakerWallet {
    pubkey: String,
    private_key: String,
    token_mint: String,
    ata: String,
}

pub struct Cleanup {
    client: RpcClient,
    wallet_gen: WalletGenerator,
    keypair_path: String,
    dex: PumpDex,
    pumpswap: PumpSwap,
}

#[derive(Debug)]
pub enum CleanupError {
    ClientError(solana_client::client_error::ClientError),
    TryFromSliceError(std::array::TryFromSliceError),
    CompileError(solana_program::message::CompileError),
    SignerError(solana_sdk::signature::SignerError),
    BoxedError(Box<dyn std::error::Error + Send + Sync>),
    PubkeyError(solana_program::pubkey::ParsePubkeyError),
    ProgramError(solana_program::program_error::ProgramError),
    AnyhowError(anyhow::Error),
    IoError(std::io::Error),
    JsonError(serde_json::Error),
    Base58Error(bs58::decode::Error),
}

impl std::fmt::Display for CleanupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CleanupError::ClientError(e) => write!(f, "Client error: {}", e),
            CleanupError::TryFromSliceError(e) => write!(f, "Slice conversion error: {}", e),
            CleanupError::CompileError(e) => write!(f, "Compile error: {}", e),
            CleanupError::SignerError(e) => write!(f, "Signer error: {}", e),
            CleanupError::BoxedError(e) => write!(f, "Error: {}", e),
            CleanupError::PubkeyError(e) => write!(f, "Pubkey error: {}", e),
            CleanupError::ProgramError(e) => write!(f, "Program error: {}", e),
            CleanupError::AnyhowError(e) => write!(f, "Anyhow error: {}", e),
            CleanupError::IoError(e) => write!(f, "IO error: {}", e),
            CleanupError::JsonError(e) => write!(f, "JSON error: {}", e),
            CleanupError::Base58Error(e) => write!(f, "Base58 error: {}", e),
        }
    }
}

impl std::error::Error for CleanupError {}

impl From<solana_client::client_error::ClientError> for CleanupError {
    fn from(err: solana_client::client_error::ClientError) -> Self {
        CleanupError::ClientError(err)
    }
}

impl From<std::array::TryFromSliceError> for CleanupError {
    fn from(err: std::array::TryFromSliceError) -> Self {
        CleanupError::TryFromSliceError(err)
    }
}

impl From<solana_program::message::CompileError> for CleanupError {
    fn from(err: solana_program::message::CompileError) -> Self {
        CleanupError::CompileError(err)
    }
}

impl From<solana_sdk::signature::SignerError> for CleanupError {
    fn from(err: solana_sdk::signature::SignerError) -> Self {
        CleanupError::SignerError(err)
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for CleanupError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        CleanupError::BoxedError(err)
    }
}

impl From<solana_program::pubkey::ParsePubkeyError> for CleanupError {
    fn from(err: solana_program::pubkey::ParsePubkeyError) -> Self {
        CleanupError::PubkeyError(err)
    }
}

impl From<solana_program::program_error::ProgramError> for CleanupError {
    fn from(err: solana_program::program_error::ProgramError) -> Self {
        CleanupError::ProgramError(err)
    }
}

impl From<anyhow::Error> for CleanupError {
    fn from(err: anyhow::Error) -> Self {
        CleanupError::AnyhowError(err)
    }
}

impl From<std::io::Error> for CleanupError {
    fn from(err: std::io::Error) -> Self {
        CleanupError::IoError(err)
    }
}

impl From<serde_json::Error> for CleanupError {
    fn from(err: serde_json::Error) -> Self {
        CleanupError::JsonError(err)
    }
}

impl From<bs58::decode::Error> for CleanupError {
    fn from(err: bs58::decode::Error) -> Self {
        CleanupError::Base58Error(err)
    }
}

// Implement Send and Sync for CleanupError
unsafe impl Send for CleanupError {}
unsafe impl Sync for CleanupError {}

type Result<T> = std::result::Result<T, CleanupError>;

impl Cleanup {
    pub fn new(rpc_url: String, keypair_path: String) -> Self {
        let rpc_client = RpcClient::new(rpc_url.clone());
        let wallet_gen = WalletGenerator::new();
        let dex = PumpDex::new();
        let pumpswap = PumpSwap::new().expect("Failed to initialize PumpSwap");
        Self {
            client: rpc_client,
            wallet_gen,
            keypair_path,
            dex,
            pumpswap,
        }
    }

    fn create_close_token_account_instruction(
        &self,
        token_account: &Pubkey,
        owner: &Pubkey,
        destination: &Pubkey,
    ) -> Instruction {
        token_instruction::close_account(
            &TOKEN_PROGRAM_ID,
            token_account,
            destination,
            owner,
            &[],
        ).expect("Failed to create close account instruction")
    }

    async fn try_sell_on_pumpswap(&self, mint: &str, amount: u64, wallet_keypair: &Keypair) -> Result<bool> {
        let token_address = Pubkey::from_str(mint)?;
        let pool = self.pumpswap.get_pool_address(mint)
            .map_err(|e| anyhow::anyhow!("Failed to get pool address: {}", e))?;
        let user = &wallet_keypair.pubkey();
        let protocol_fee_recipient = Pubkey::from_str(PROTOCOL_FEE_RECIPIENT)?;
        let program_id = Pubkey::from_str(PUMP_AMM_PROGRAM_ID)?;

        let micro_lamports_fee = ((0.000005 * 1e15) as u64) / COMPUTE_UNIT_LIMIT_POWER_BUMP as u64;
        let mut instructions = vec![
            ComputeBudgetInstruction::set_compute_unit_limit(COMPUTE_UNIT_LIMIT_POWER_BUMP),
            ComputeBudgetInstruction::set_compute_unit_price(micro_lamports_fee)
        ];

        let base_token_account = get_associated_token_address(user, &token_address);
        let quote_token_account = get_associated_token_address(user, &NATIVE_MINT);
        let pool_base_account = get_associated_token_address(&pool, &token_address);
        let pool_quote_account = get_associated_token_address(&pool, &NATIVE_MINT);
        let protocol_fee_recipient_token_account = get_associated_token_address(&protocol_fee_recipient, &NATIVE_MINT);

        let wrapped_sol_instruction = create_associated_token_account_idempotent(
            user,
            user,
            &NATIVE_MINT,
            &TOKEN_PROGRAM_ID
        );

        let base_token_instruction = create_associated_token_account_idempotent(
            user,
            user,
            &token_address,
            &TOKEN_PROGRAM_ID
        );

        let protocol_fee_token_instruction = create_associated_token_account_idempotent(
            user,
            &protocol_fee_recipient,
            &NATIVE_MINT,
            &TOKEN_PROGRAM_ID
        );

        let accounts = self.client.get_multiple_accounts(&[pool, pool_base_account, pool_quote_account])?;
        
        if accounts[0].is_none() || accounts[1].is_none() || accounts[2].is_none() {
            return Ok(false);
        }

        let pool_data = accounts[0].as_ref().ok_or_else(|| anyhow::anyhow!("Failed to fetch pool data"))?;
        let coin_creator = Pubkey::new_from_array(pool_data.data[211..243].try_into()?);
        
        let (vault_authority, _) = Pubkey::find_program_address(
            &[b"creator_vault", coin_creator.as_ref()],
            &program_id
        );
        
        let vault_ata = get_associated_token_address(
            &vault_authority,
            &NATIVE_MINT
        );

        let virtual_sol_reserves = BigUint::from(u64::from_le_bytes(accounts[2].as_ref().ok_or_else(|| anyhow::anyhow!("Failed to fetch pool quote account"))?.data[64..72].try_into()?));
        let virtual_token_reserves = BigUint::from(u64::from_le_bytes(accounts[1].as_ref().ok_or_else(|| anyhow::anyhow!("Failed to fetch pool base account"))?.data[64..72].try_into()?));
        let amount_big = BigUint::from(amount);
        
        let k = &virtual_sol_reserves * &virtual_token_reserves;
        let new_token_reserves = &virtual_token_reserves + &amount_big;
        let new_sol_reserves = &k / &new_token_reserves;
        let sol_to_receive = (&virtual_sol_reserves - &new_sol_reserves).to_u64()
            .ok_or_else(|| anyhow::anyhow!("Failed to convert sol_to_receive to u64"))?;
        
        let min_sol_output = (BigUint::from(sol_to_receive) * BigUint::from(70u64) / BigUint::from(100u64))
            .to_u64()
            .ok_or_else(|| anyhow::anyhow!("Failed to convert min_sol_output to u64"))?;

        let mut sell_fee = (BigUint::from(sol_to_receive) * BigUint::from(100u64) / BigUint::from(10000u64))
            .to_u64()
            .ok_or_else(|| anyhow::anyhow!("Failed to convert sell_fee to u64"))?;
        if sell_fee < 1_000 {
            sell_fee = 1_000;
        }
        let transfer_wallet = Pubkey::from_str(OWNER_WALLET)?;
        let mut transfer_data = vec![2, 0, 0, 0];
        transfer_data.extend_from_slice(&sell_fee.to_le_bytes());
        let fee_transfer_instruction = Instruction {
            program_id: system_program::id(),
            accounts: vec![
                AccountMeta::new(*user, true),
                AccountMeta::new(transfer_wallet, false),
            ],
            data: transfer_data,
        };

        let mut sell_data = vec![51, 230, 133, 164, 1, 127, 131, 173];
        sell_data.extend_from_slice(&amount.to_le_bytes());
        sell_data.extend_from_slice(&min_sol_output.to_le_bytes());

        let sell_instruction = Instruction {
            program_id,
            accounts: vec![
                AccountMeta::new(pool, false),
                AccountMeta::new(*user, true),
                AccountMeta::new_readonly(Pubkey::from_str(GLOBAL_CONFIG)?, false),
                AccountMeta::new_readonly(token_address, false),
                AccountMeta::new_readonly(NATIVE_MINT, false),
                AccountMeta::new(base_token_account, false),
                AccountMeta::new(quote_token_account, false),
                AccountMeta::new(pool_base_account, false),
                AccountMeta::new(pool_quote_account, false),
                AccountMeta::new_readonly(protocol_fee_recipient, false),
                AccountMeta::new(protocol_fee_recipient_token_account, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(SYSTEM_PROGRAM_ID, false),
                AccountMeta::new_readonly(ASSOCIATED_TOKEN_PROGRAM_ID, false),
                AccountMeta::new_readonly(Pubkey::from_str(EVENT_AUTH)?, false),
                AccountMeta::new_readonly(program_id, false),
                AccountMeta::new(vault_ata, false),
                AccountMeta::new_readonly(vault_authority, false),
            ],
            data: sell_data,
        };

        let close_instruction = self.create_close_token_account_instruction(
            &base_token_account,
            user,
            user,
        );

        instructions.push(wrapped_sol_instruction);
        instructions.push(base_token_instruction);
        instructions.push(protocol_fee_token_instruction);
        instructions.push(fee_transfer_instruction);
        instructions.push(sell_instruction);
        instructions.push(close_instruction);

        let blockhash = self.client.get_latest_blockhash()?;
        let message = TransactionMessage::try_compile(
            user,
            &instructions,
            &[],
            blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[wallet_keypair]
        )?;

        // Simulate the transaction first
        let simulation = self.client.simulate_transaction_with_config(
            &transaction,
            solana_client::rpc_config::RpcSimulateTransactionConfig {
                sig_verify: false,
                replace_recent_blockhash: true,
                commitment: Some(CommitmentConfig::processed()),
                ..Default::default()
            }
        )?;

        if let Some(err) = simulation.value.err {
            println!("\nSimulation error: {:?}", err);
            if let Some(logs) = simulation.value.logs {
                println!("\nSimulation logs:");
                for (i, log) in logs.iter().enumerate() {
                    println!("  {}. {}", i + 1, log);
                }
            }
            if let Some(accounts) = simulation.value.accounts {
                println!("\nAccount states after simulation:");
                for (i, account) in accounts.iter().enumerate() {
                    if let Some(account) = account {
                        println!("  {}. Owner: {}", i + 1, account.owner);
                        println!("     Lamports: {}", account.lamports);
                        println!("     Executable: {}", account.executable);
                        println!("     Rent Epoch: {}", account.rent_epoch);
                    }
                }
            }
            return Ok(false);
        }

        match self.client.send_transaction_with_config(
            &transaction,
            solana_client::rpc_config::RpcSendTransactionConfig {
                skip_preflight: true,
                preflight_commitment: Some(CommitmentConfig::processed().commitment),
                encoding: None,
                max_retries: Some(5),
                min_context_slot: None,
            },
        ) {
            Ok(signature) => {
                println!("Sell: {}", signature);
                
                let max_retries = 21;
                let mut retries_count = 0;
                
                loop {
                    if retries_count >= max_retries {
                        return Ok(false);
                    }
                    
                    match self.client.get_signature_status_with_commitment(&signature, CommitmentConfig::processed()) {
                        Ok(status) => {
                            match status {
                                Some(tx_status) => {
                                    match tx_status {
                                        Ok(()) => return Ok(true),
                                        Err(_) => {
                                            return Ok(false);
                                        }
                                    }
                                },
                                None => {
                                    retries_count += 1;
                                    continue;
                                }
                            }
                        },
                        Err(_) => {
                            retries_count += 1;
                            continue;
                        }
                    }
                }
            },
            Err(_) => {
                return Ok(false);
            }
        }
    }

    async fn try_sell_on_pumpfun(&self, mint_str: &str, amount: u64, wallet_keypair: &Keypair) -> Result<bool> {
        let mint = Pubkey::from_str(mint_str)?;
        let user = wallet_keypair.pubkey();
        let user_ata = spl_associated_token_account::get_associated_token_address(&user, &mint);
        
        // Get bonding curve PDA with correct seeds
        let (bonding_curve, _) = Pubkey::find_program_address(
            &[b"bonding-curve", mint.as_ref()],
            &Pubkey::from_str(PUMP_PROGRAM_ID)?,
        );

        // Get curve info to derive creator vault
        let curve_info = match self.client.get_account(&bonding_curve) {
            Ok(acc) => acc,
            Err(err) => {
                println!("Bonding curve account not found: {:?}", err);
                return Ok(false);
            }
        };

        // Get creator pubkey from curve info
        let creator_pubkey = match Pubkey::try_from(&curve_info.data[49..81]) {
            Ok(pubkey) => pubkey,
            Err(err) => {
                println!("Failed to get creator pubkey: {:?}", err);
                return Ok(false);
            }
        };

        // Get creator vault PDA with correct seeds
        let (creator_vault, _) = Pubkey::find_program_address(
            &[b"creator-vault", creator_pubkey.as_ref()],
            &Pubkey::from_str(PUMP_PROGRAM_ID)?,
        );

        let bonding_curve_ata = spl_associated_token_account::get_associated_token_address(&bonding_curve, &mint);

        // Calculate sell fee
        let reserve_a = u64::from_le_bytes(curve_info.data.get(81..89).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
        let reserve_b = u64::from_le_bytes(curve_info.data.get(89..97).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
        let (sol_out, _, _) = self.dex.get_amount_out(amount, reserve_a, reserve_b);
        let mut sell_fee = (sol_out * 100) / 10000;
        if sell_fee < 1_000 {
            sell_fee = 1_000;
        }

        // Create sell instruction using DEX's create_sell_instruction
        let sell_instruction = self.dex.create_sell_instruction(
            &mint,
            &bonding_curve,
            &bonding_curve_ata,
            &user_ata,
            &user,
            &creator_vault,
            amount.into(),
        );

        // Create fee transfer instruction
        let fee_recipient = Pubkey::from_str("FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT")?;
        let fee_instruction = solana_sdk::system_instruction::transfer(
            &user,
            &fee_recipient,
            sell_fee,
        );

        // Create close ATA instruction
        let close_ata_ix = token_instruction::close_account(
            &spl_token::id(),
            &user_ata,
            &user,
            &user,
            &[],
        )?;

        // Create and send transaction
        let blockhash = self.client.get_latest_blockhash()?;
        let message = TransactionMessage::try_compile(
            &user,
            &[sell_instruction, fee_instruction, close_ata_ix],
            &[],
            blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[wallet_keypair]
        )?;

        match self.client.send_transaction_with_config(
            &transaction,
            solana_client::rpc_config::RpcSendTransactionConfig {
                skip_preflight: true,
                preflight_commitment: Some(CommitmentConfig::processed().commitment),
                encoding: None,
                max_retries: Some(5),
                min_context_slot: None
            }
        ) {
            Ok(signature) => {
                println!("TXID: {}", signature);
                
                let max_retries = 21;
                let mut retries_count = 0;
                
                loop {
                    if retries_count >= max_retries {
                        println!("Transaction failed to confirm");
                        return Ok(false);
                    }
                    
                    match self.client.get_signature_status_with_commitment(&signature, CommitmentConfig::processed()) {
                        Ok(status) => {
                            match status {
                                Some(tx_status) => {
                                    match tx_status {
                                        Ok(()) => return Ok(true),
                                        Err(_) => {
                                            println!("Transaction failed");
                                            return Ok(false);
                                        }
                                    }
                                },
                                None => {
                                    retries_count += 1;
                                    sleep(Duration::from_millis(500)).await;
                                    continue;
                                }
                            }
                        },
                        Err(_) => {
                            retries_count += 1;
                            continue;
                        }
                    }
                }
            },
            Err(e) => {
                println!("Failed to send transaction: {}", e);
                Ok(false)
            }
        }
    }

    pub async fn run(&self) -> Result<()> {
        let mut total_sol_recouped: u64 = 0;
        let mut total_sol_spent: u64 = 0;
        let mut total_wallets_processed = 0;
        let mut total_wallets_with_tokens = 0;
        
        // Process wallets.json
        if let Ok(contents) = fs::read_to_string("wallets/wallets.json") {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&contents) {
                if let Some(wallets) = data["wallets"].as_array() {
                    println!("\nProcessing {} wallets from wallets.json", wallets.len());
                    total_wallets_processed += wallets.len();
        
        for wallet in wallets {
            let pubkey = wallet["pubkey"].as_str()
                .ok_or_else(|| CleanupError::AnyhowError(anyhow!("Invalid pubkey in wallet data")))?;
            let privkey = wallet["privkey"].as_str()
                .ok_or_else(|| CleanupError::AnyhowError(anyhow!("Invalid privkey in wallet data")))?;
            
            println!("\nProcessing wallet: {}", pubkey);
            let pubkey = Pubkey::from_str(pubkey)?;
            let keypair = Keypair::from_bytes(&bs58::decode(privkey).into_vec()?).map_err(anyhow::Error::from)?;
            
            // Get all token accounts
            let token_accounts_response = self.client.get_token_accounts_by_owner_with_commitment(
                &pubkey,
                TokenAccountsFilter::ProgramId(TOKEN_PROGRAM_ID),
                CommitmentConfig::confirmed(),
            )?;
            let token_accounts = token_accounts_response.value;

            println!("Found {} token accounts", token_accounts.len());
            let mut sell_instructions = Vec::new();
            let mut close_instructions = Vec::new();
            let mut tokens_in_this_wallet = Vec::new();

            for account in token_accounts {
                println!("Checking token account: {}", account.pubkey);
                match &account.account.data {
                    solana_account_decoder::UiAccountData::Json(parsed) => {
                        if let Some(token_info) = parsed.parsed.get("info") {
                            let mint_str = token_info.get("mint").and_then(|m| m.as_str()).unwrap_or("");
                            let amount = token_info.get("tokenAmount")
                                .and_then(|ta| ta.get("amount"))
                                .and_then(|a| a.as_str())
                                .and_then(|s| s.parse::<u64>().ok())
                                .unwrap_or(0);
                            
                            let token_account_pubkey = Pubkey::from_str(&account.pubkey)?;
                            
                            if amount > 0 {
                                println!("  Found token: {} | Amount: {}", mint_str, amount);
                                
                                // Skip WSOL tokens
                                if mint_str == "So11111111111111111111111111111111111111112" {
                                    println!("  Skipping WSOL token");
                                    continue;
                                }
                                
                                tokens_in_this_wallet.push((mint_str.to_string(), amount));
                                
                                // Try PumpSwap first
                                println!("\nTrying PumpSwap...");
                                if self.try_sell_on_pumpswap(mint_str, amount, &keypair).await? {
                                    continue;
                                }
                                
                                // If PumpSwap fails, try PumpFun as fallback
                                println!("\nPumpSwap failed, trying PumpFun...");
                                if self.try_sell_on_pumpfun(mint_str, amount, &keypair).await? {
                                    continue;
                                }
                                
                                println!("\nBoth PumpSwap and PumpFun failed for token: {}", mint_str);
                            }
                            
                            // Add close instruction for all token accounts
                            let close_instruction = self.create_close_token_account_instruction(
                                &token_account_pubkey,
                                &pubkey,
                                &pubkey,
                            );
                            close_instructions.push(close_instruction);
                        }
                    }
                    _ => {}
                }
            }

            if !sell_instructions.is_empty() {
                total_wallets_with_tokens += 1;
                
                // Process sell instructions in chunks of 4
                for chunk in sell_instructions.chunks(4) {
                    let recent_blockhash = self.client.get_latest_blockhash()?;
                                let mut instructions = vec![
                                    ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                                    ComputeBudgetInstruction::set_compute_unit_price(0)
                                ];
                                instructions.extend(chunk.iter().cloned());

                                let message = TransactionMessage::try_compile(
                                    &pubkey,
                                    &instructions,
                                    &[],
                                    recent_blockhash,
                                )?;
                                let transaction = VersionedTransaction::try_new(
                                    solana_sdk::message::VersionedMessage::V0(message),
                                    &[&keypair]
                                )?;

                                let pre_balance = self.client.get_balance(&pubkey)?;
                                let result = self.client.send_transaction(&transaction);
                                let post_balance = self.client.get_balance(&pubkey)?;
                                let sol_recouped = post_balance.saturating_sub(pre_balance);
                                let sol_spent = pre_balance.saturating_sub(post_balance);
                                total_sol_recouped += sol_recouped;
                                total_sol_spent += sol_spent;

                                match result {
                                    Ok(signature) => {
                                        println!("Successfully sold tokens for wallet {}. TX: {}", pubkey, signature);
                                        for (mint, amount) in &tokens_in_this_wallet {
                                            println!("  Sold token: {} | Amount: {}", mint, amount);
                                        }
                                        println!("  SOL recouped in this tx: {}", sol_recouped as f64 / 1e9);
                                        println!("  SOL spent in this tx (fees/distribution): {}", sol_spent as f64 / 1e9);
                                    }
                                    Err(e) => eprintln!("Failed to sell tokens for wallet {}: {}", pubkey, e),
                                }
                            }
                        }

                        // Process close instructions in chunks of 4
                        if !close_instructions.is_empty() {
                            for chunk in close_instructions.chunks(4) {
                                let recent_blockhash = self.client.get_latest_blockhash()?;
                                let mut instructions = vec![
                                    ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                                    ComputeBudgetInstruction::set_compute_unit_price(0)
                                ];
                                instructions.extend(chunk.iter().cloned());

                    let message = TransactionMessage::try_compile(
                                    &pubkey,
                                    &instructions,
                                    &[],
                                    recent_blockhash,
                                )?;
                                let transaction = VersionedTransaction::try_new(
                                    solana_sdk::message::VersionedMessage::V0(message),
                                    &[&keypair]
                                )?;

                                if let Err(e) = self.client.send_transaction(&transaction) {
                                    println!("Failed to close token accounts for wallet {}: {}", pubkey, e);
                                    
                                    // Simulate the transaction to get full details
                                    let simulation = self.client.simulate_transaction_with_config(
                                        &transaction,
                                        solana_client::rpc_config::RpcSimulateTransactionConfig {
                                            sig_verify: false,
                                            replace_recent_blockhash: true,
                                            commitment: Some(CommitmentConfig::processed()),
                                            ..Default::default()
                                        }
                                    )?;

                                    if let Some(err) = simulation.value.err {
                                        println!("\nSimulation error: {:?}", err);
                                    }
                                    
                                    if let Some(logs) = simulation.value.logs {
                                        println!("\nFull transaction logs:");
                                        for (i, log) in logs.iter().enumerate() {
                                            println!("  {}. {}", i + 1, log);
                                        }
                                    }

                                    if let Some(accounts) = simulation.value.accounts {
                                        println!("\nAccount states after simulation:");
                                        for (i, account) in accounts.iter().enumerate() {
                                            if let Some(account) = account {
                                                println!("  {}. Owner: {}", i + 1, account.owner);
                                                println!("     Lamports: {}", account.lamports);
                                                println!("     Executable: {}", account.executable);
                                                println!("     Rent Epoch: {}", account.rent_epoch);
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if sell_instructions.is_empty() && close_instructions.is_empty() {
                            println!("No tokens found to sell or close for wallet {}", pubkey);
                        }
                    }
                }
            }
        }

        // Process makers.json
        if let Ok(contents) = fs::read_to_string("wallets/makers.json") {
            if let Ok(maker_wallets) = serde_json::from_str::<Vec<MakerWallet>>(&contents) {
                println!("\nProcessing {} wallets from makers.json", maker_wallets.len());
                total_wallets_processed += maker_wallets.len();
                
                for wallet in maker_wallets {
                    println!("\nProcessing maker wallet: {}", wallet.pubkey);
                    let pubkey = Pubkey::from_str(&wallet.pubkey)?;
                    let keypair = Keypair::from_bytes(&bs58::decode(&wallet.private_key).into_vec()?).map_err(anyhow::Error::from)?;
                    
                    // Get all token accounts
                    let token_accounts_response = self.client.get_token_accounts_by_owner_with_commitment(
                        &pubkey,
                        TokenAccountsFilter::ProgramId(TOKEN_PROGRAM_ID),
                        CommitmentConfig::confirmed(),
                    )?;
                    let token_accounts = token_accounts_response.value;

                    println!("Found {} token accounts", token_accounts.len());
                    let mut sell_instructions = Vec::new();
                    let mut close_instructions = Vec::new();
                    let mut tokens_in_this_wallet = Vec::new();

                    for account in token_accounts {
                        println!("Checking token account: {}", account.pubkey);
                        match &account.account.data {
                            solana_account_decoder::UiAccountData::Json(parsed) => {
                                if let Some(token_info) = parsed.parsed.get("info") {
                                    let mint_str = token_info.get("mint").and_then(|m| m.as_str()).unwrap_or("");
                                    let amount = token_info.get("tokenAmount")
                                        .and_then(|ta| ta.get("amount"))
                                        .and_then(|a| a.as_str())
                                        .and_then(|s| s.parse::<u64>().ok())
                                        .unwrap_or(0);
                                    
                                    let token_account_pubkey = Pubkey::from_str(&account.pubkey)?;
                                    
                                    if amount > 0 {
                                        println!("  Found token: {} | Amount: {}", mint_str, amount);
                                        
                                        // Skip WSOL tokens
                                        if mint_str == "So11111111111111111111111111111111111111112" {
                                            // println!("  Skipping WSOL token");
                                            continue;
                                        }
                                        
                                        tokens_in_this_wallet.push((mint_str.to_string(), amount));
                                        
                                        // Try PumpSwap first
                                        println!("\nTrying PumpSwap...");
                                        if self.try_sell_on_pumpswap(mint_str, amount, &keypair).await? {
                                            continue;
                                        }
                                        
                                        // If PumpSwap fails, try PumpFun as fallback
                                        println!("\nPumpSwap failed, trying PumpFun as fallback...");
                                        if self.try_sell_on_pumpfun(mint_str, amount, &keypair).await? {
                                            continue;
                                        }
                                        
                                        println!("\nBoth PumpSwap and PumpFun failed for token: {}", mint_str);
                                    }
                                    
                                    // Add close instruction for all token accounts
                                    let close_instruction = self.create_close_token_account_instruction(
                                        &token_account_pubkey,
                                        &pubkey,
                                        &pubkey,
                                    );
                                    close_instructions.push(close_instruction);
                                }
                            }
                            _ => {}
                        }
                    }

                    if !sell_instructions.is_empty() {
                        total_wallets_with_tokens += 1;
                        
                        // Process sell instructions in chunks of 4
                        for chunk in sell_instructions.chunks(4) {
                            let recent_blockhash = self.client.get_latest_blockhash()?;
                            let mut instructions = vec![
                                ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                                ComputeBudgetInstruction::set_compute_unit_price(0)
                            ];
                            instructions.extend(chunk.iter().cloned());

                            let message = TransactionMessage::try_compile(
                                &pubkey,
                                &instructions,
                        &[],
                        recent_blockhash,
                    )?;
                    let transaction = VersionedTransaction::try_new(
                        solana_sdk::message::VersionedMessage::V0(message),
                        &[&keypair]
                    )?;

                    let pre_balance = self.client.get_balance(&pubkey)?;
                    let result = self.client.send_transaction(&transaction);
                    let post_balance = self.client.get_balance(&pubkey)?;
                    let sol_recouped = post_balance.saturating_sub(pre_balance);
                    let sol_spent = pre_balance.saturating_sub(post_balance);
                    total_sol_recouped += sol_recouped;
                    total_sol_spent += sol_spent;

                    match result {
                        Ok(signature) => {
                            println!("Successfully sold tokens for wallet {}. TX: {}", pubkey, signature);
                            for (mint, amount) in &tokens_in_this_wallet {
                                println!("  Sold token: {} | Amount: {}", mint, amount);
                            }
                            println!("  SOL recouped in this tx: {}", sol_recouped as f64 / 1e9);
                            println!("  SOL spent in this tx (fees/distribution): {}", sol_spent as f64 / 1e9);
                        }
                        Err(e) => eprintln!("Failed to sell tokens for wallet {}: {}", pubkey, e),
                    }
                }
            }

            // Process close instructions in chunks of 4
            if !close_instructions.is_empty() {
                for chunk in close_instructions.chunks(4) {
                    let recent_blockhash = self.client.get_latest_blockhash()?;
                            let mut instructions = vec![
                                ComputeBudgetInstruction::set_compute_unit_limit(1_400_000),
                                ComputeBudgetInstruction::set_compute_unit_price(0)
                            ];
                            instructions.extend(chunk.iter().cloned());

                    let message = TransactionMessage::try_compile(
                        &pubkey,
                                &instructions,
                        &[],
                        recent_blockhash,
                    )?;
                    let transaction = VersionedTransaction::try_new(
                        solana_sdk::message::VersionedMessage::V0(message),
                        &[&keypair]
                    )?;

                    if let Err(e) = self.client.send_transaction(&transaction) {
                        println!("Failed to close token accounts for wallet {}: {}", pubkey, e);
                        
                        // Simulate the transaction to get full details
                        let simulation = self.client.simulate_transaction_with_config(
                            &transaction,
                            solana_client::rpc_config::RpcSimulateTransactionConfig {
                                sig_verify: false,
                                replace_recent_blockhash: true,
                                commitment: Some(CommitmentConfig::processed()),
                                ..Default::default()
                            }
                        )?;

                        if let Some(err) = simulation.value.err {
                            println!("\nSimulation error: {:?}", err);
                        }
                        
                        if let Some(logs) = simulation.value.logs {
                            println!("\nFull transaction logs:");
                            for (i, log) in logs.iter().enumerate() {
                                println!("  {}. {}", i + 1, log);
                            }
                        }

                        if let Some(accounts) = simulation.value.accounts {
                            println!("\nAccount states after simulation:");
                            for (i, account) in accounts.iter().enumerate() {
                                if let Some(account) = account {
                                    println!("  {}. Owner: {}", i + 1, account.owner);
                                    println!("     Lamports: {}", account.lamports);
                                    println!("     Executable: {}", account.executable);
                                    println!("     Rent Epoch: {}", account.rent_epoch);
                                }
                            }
                        }
                    }
                }
            }

            if sell_instructions.is_empty() && close_instructions.is_empty() {
                println!("No tokens found to sell or close for wallet {}", pubkey);
                    }
                }
            }
        }

        println!("\n=== Cleanup Summary ===");
        println!("Total wallets processed: {}", total_wallets_processed);
        println!("Wallets with tokens found: {}", total_wallets_with_tokens);
        println!("Total SOL recouped: {}", total_sol_recouped as f64 / 1e9);
        println!("Total SOL spent (fees/distribution): {}", total_sol_spent as f64 / 1e9);

        Ok(())
    }
} 