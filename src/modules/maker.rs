use anyhow::Result;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::{Pubkey, ParsePubkeyError},
    signature::Keypair,
    transaction::VersionedTransaction,
    instruction::{Instruction, AccountMeta},
    message::{Message, v0::Message as VersionedMessage, v0::Message as TransactionMessage},
    system_program,
    signer::Signer,
    compute_budget::ComputeBudgetInstruction,
};
use solana_program::program_error::ProgramError;
use spl_token::instruction as token_instruction;
use spl_associated_token_account::instruction as associated_token_instruction;
use std::str::FromStr;
use reqwest::Client;
use serde_json::json;
use base64;
use bincode::serialize;
use crate::dex::pump::{PumpDex, TRANSFER_FEE_BPS, FEE_DENOMINATOR, TRANSFER_WALLET};
use crate::dex::pumpswap::PumpSwap;
use rand::seq::SliceRandom;
use std::sync::Arc;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;
use spl_token::state::Account as TokenAccount;
use spl_token::ID as TOKEN_PROGRAM_ID;
use solana_client::rpc_request::TokenAccountsFilter;
use solana_account_decoder::UiAccountData;
use solana_program::program_pack::Pack;
use dotenv::dotenv;
use std::env;
use bs58;
use serde::{Serialize, Deserialize};
use spl_token::native_mint::ID as NATIVE_MINT;

const BLOCK_ENGINES: [&str; 6] = [
    "https://frankfurt.mainnet.block-engine.jito.wtf",
    "https://amsterdam.mainnet.block-engine.jito.wtf",
    "https://london.mainnet.block-engine.jito.wtf",
    "https://ny.mainnet.block-engine.jito.wtf",
    "https://tokyo.mainnet.block-engine.jito.wtf",
    "https://slc.mainnet.block-engine.jito.wtf"
];

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

const PUMP_AMM_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
const PUMP_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const PUMPFUN_PROGRAM_ID: &str = "PFu1zUcN6x4Kb6y6K5H5K5H5K5H5K5H5K5H5K5H5K";
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const GLOBAL_CONFIG: &str = "ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw";
const PROTOCOL_FEE_RECIPIENT: &str = "62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV";
const PROTOCOL_FEE_ACCOUNT: &str = "7xQYoUjUJF1Kg6WVczoTAkaNhn5syQYcbvjmFrhjWpx";
const EVENT_AUTH: &str = "GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR";
const SYSTEM_PROGRAM_ID: Pubkey = system_program::id();
const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey = spl_associated_token_account::id();


const PUMP_GLOBAL: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
const PUMP_FEE_RECIPIENT: &str = "CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM";
const PUMP_EVENT_AUTHORITY: &str = "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1";

const COMPUTE_UNIT_LIMIT_POWER_BUMP: u32 = 1_400_000;
const ATA_CREATE_FEE: f64 = 0.00203928;
const LP_FEE_BASIS_POINTS: u64 = 30;
const PROTOCOL_FEE_BASIS_POINTS: u64 = 10;
const COIN_CREATOR_FEE_BASIS_POINTS: u64 = 10;
const DEFAULT_BUY_AMOUNT_LAMPORTS: u64 = 1_000_000;

#[derive(Serialize, Deserialize)]
struct MakerWallet {
    pubkey: String,
    private_key: String,
    token_mint: String,
    ata: String,
    num_makers: usize,
}

#[derive(Debug)]
pub enum MakerError {
    RpcError(String),
    TokenError(String),
    TransactionError(String),
    InvalidAmount(String),
    ProgramError(String),
    FileError(String),
    ParsePubkeyError(String),
    ParseError(String),
    TryFromError(std::array::TryFromSliceError),
    ClientError(solana_client::client_error::ClientError),
}

impl std::fmt::Display for MakerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MakerError::RpcError(e) => write!(f, "Error: {}", e),
            MakerError::TokenError(e) => write!(f, "Error: {}", e),
            MakerError::TransactionError(e) => write!(f, "Error: {}", e),
            MakerError::InvalidAmount(e) => write!(f, "Error: {}", e),
            MakerError::ProgramError(e) => write!(f, "Error: {}", e),
            MakerError::FileError(e) => write!(f, "Error: {}", e),
            MakerError::ParsePubkeyError(e) => write!(f, "Error: {}", e),
            MakerError::ParseError(e) => write!(f, "Error: {}", e),
            MakerError::TryFromError(e) => write!(f, "Error: {}", e),
            MakerError::ClientError(e) => write!(f, "Error: {}", e),
        }
    }
}

impl std::error::Error for MakerError {}

impl From<ProgramError> for MakerError {
    fn from(err: ProgramError) -> Self {
        MakerError::ProgramError(err.to_string())
    }
}

impl From<ParsePubkeyError> for MakerError {
    fn from(err: ParsePubkeyError) -> Self {
        MakerError::ParsePubkeyError(err.to_string())
    }
}

impl From<Box<dyn std::error::Error>> for MakerError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        MakerError::TokenError(err.to_string())
    }
}

impl From<&str> for MakerError {
    fn from(err: &str) -> Self {
        MakerError::ParseError(err.to_string())
    }
}

impl From<std::array::TryFromSliceError> for MakerError {
    fn from(err: std::array::TryFromSliceError) -> Self {
        MakerError::TryFromError(err)
    }
}

impl From<std::io::Error> for MakerError {
    fn from(err: std::io::Error) -> Self {
        MakerError::FileError(err.to_string())
    }
}

impl From<solana_client::client_error::ClientError> for MakerError {
    fn from(err: solana_client::client_error::ClientError) -> Self {
        MakerError::ClientError(err)
    }
}

impl From<solana_sdk::signature::SignerError> for MakerError {
    fn from(err: solana_sdk::signature::SignerError) -> Self {
        MakerError::TransactionError(err.to_string())
    }
}

impl From<solana_sdk::message::CompileError> for MakerError {
    fn from(err: solana_sdk::message::CompileError) -> Self {
        MakerError::TransactionError(err.to_string())
    }
}

pub struct MakerBot {
    rpc_client: RpcClient,
    payer: Keypair,
    pump_dex: PumpDex,
    pump_swap: PumpSwap,
    use_pumpswap: bool,
    use_pumpfun: bool,
}

impl MakerBot {
    fn get_payer_from_env() -> Result<Keypair, MakerError> {
        dotenv().ok();
        let payer_privkey = env::var("PAYER").map_err(|_| MakerError::TokenError("PAYER not set in .env file".to_string()))?;
        let bytes = bs58::decode(&payer_privkey).into_vec().map_err(|e| MakerError::TokenError(format!("Failed to decode payer private key: {}", e)))?;
        Keypair::from_bytes(&bytes).map_err(|e| MakerError::TokenError(format!("Failed to create keypair: {}", e)))
    }

    fn get_jito_uuid() -> Option<String> {
        dotenv().ok();
        env::var("UUID").ok()
    }

    pub fn new(rpc_url: String) -> Result<Self, MakerError> {
        let rpc_client = RpcClient::new(rpc_url.clone());
        let payer = Self::get_payer_from_env()?;
        
        Ok(Self {
            rpc_client,
            payer,
            pump_dex: PumpDex::new(),
            pump_swap: PumpSwap::new().map_err(|e| MakerError::TokenError(e.to_string()))?,
            use_pumpswap: false,
            use_pumpfun: false,
        })
    }

    pub fn set_dex(&mut self, use_pumpswap: bool, use_pumpfun: bool) {
        self.use_pumpswap = use_pumpswap;
        self.use_pumpfun = use_pumpfun;
    }

    fn get_random_tip_address() -> Pubkey {
        let mut rng = rand::thread_rng();
        let tip_address = TIP_ADDRESSES.choose(&mut rng).unwrap();
        Pubkey::from_str(tip_address).unwrap()
    }

    fn save_wallets(&self, wallets: &[Keypair], num_makers: usize) -> Result<(), MakerError> {
        let maker_wallets: Vec<MakerWallet> = wallets.iter().map(|wallet| {
            MakerWallet {
                pubkey: wallet.pubkey().to_string(),
                private_key: bs58::encode(wallet.to_bytes()).into_string(),
                token_mint: "".to_string(),
                ata: "".to_string(),
                num_makers,
            }
        }).collect();

        let json = serde_json::to_string_pretty(&maker_wallets)
            .map_err(|e| MakerError::FileError(format!("Failed to serialize wallets: {}", e)))?;

        fs::write("wallets/makers.json", json)
            .map_err(|e| MakerError::FileError(format!("Failed to write wallets file: {}", e)))?;

        Ok(())
    }

    pub async fn run_maker(
        &self,
        num_holders: usize,
        jito_tip_sol: f64,
        token_mint: &str,
        delay_ms: u64,
    ) -> Result<Vec<(u64, String)>, MakerError> {
        if num_holders == 0 {
            return Err(MakerError::InvalidAmount("Number of holders must be greater than 0".to_string()));
        }

        if jito_tip_sol <= 0.0 {
            return Err(MakerError::InvalidAmount("Jito tip amount must be greater than 0".to_string()));
        }

        let mut results: Vec<(u64, String)> = Vec::new();
        let token_mint = Pubkey::from_str(token_mint)
            .map_err(|e| MakerError::TokenError(format!("Invalid token mint: {}", e)))?;
        
        println!("\nCreating {} new wallets", num_holders);
        let mut new_wallets = Vec::new();
        for _ in 0..num_holders {
            let keypair = Keypair::new();
            new_wallets.push(keypair);
        }
        self.save_wallets(&new_wallets, num_holders)?;

        let wallet_rent = 890_088u64;
        let ata_rent = 2_039_280u64;
        let min_extra = 5_000u64;
        let claim_extra = 10_000u64;
        let buy_amount_lamports = 100u64;
        let tip_lamports = (jito_tip_sol * 1_000_000_000.0) as u64;
        let tip_address = Self::get_random_tip_address();
        let affiliate_wallet = Pubkey::from_str("FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT")?;
        
        let mut bundle_urls = Vec::new();
        let mut remaining_wallets = new_wallets.as_slice();
        
        let total_wallets = remaining_wallets.len();
        let max_makers_per_bundle = 4;
        let total_bundles = (total_wallets + max_makers_per_bundle - 1) / max_makers_per_bundle;

        for bundle_index in 0..total_bundles {
            let start = bundle_index * max_makers_per_bundle;
            let end = std::cmp::min(start + max_makers_per_bundle, total_wallets);
            let bundle_wallets = &remaining_wallets[start..end];
            
            println!("\nSending bundle {}/{}", bundle_index + 1, total_bundles);
            
            let mut funding_instructions = Vec::new();
            let mut total_funding = 0u64;
            
            let micro_lamports_fee = ((0.000005 * 1e15) as u64) / COMPUTE_UNIT_LIMIT_POWER_BUMP as u64;
            funding_instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(COMPUTE_UNIT_LIMIT_POWER_BUMP));
            funding_instructions.push(ComputeBudgetInstruction::set_compute_unit_price(micro_lamports_fee));
            
            let affiliate_fee = (bundle_wallets.len() as u64) * 100_000;
            funding_instructions.push(solana_sdk::system_instruction::transfer(
                &self.payer.pubkey(),
                &affiliate_wallet,
                affiliate_fee
            ));
            
            funding_instructions.push(solana_sdk::system_instruction::transfer(
                &self.payer.pubkey(),
                &tip_address,
                tip_lamports
            ));
            
            for wallet in bundle_wallets {
                let wallet_pubkey = wallet.pubkey();
                let total_needed = buy_amount_lamports + 2_039_280 + 2_039_280 + 5_000 + 10_000;
                total_funding += total_needed;
                
                funding_instructions.push(solana_sdk::system_instruction::transfer(
                    &self.payer.pubkey(),
                    &wallet_pubkey,
                    total_needed
                ));
            }
            
            let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
            let funding_message = TransactionMessage::try_compile(
                &self.payer.pubkey(),
                &funding_instructions,
                &[],
                recent_blockhash,
            )?;
            
            let funding_tx = VersionedTransaction::try_new(
                solana_sdk::message::VersionedMessage::V0(funding_message),
                &[&self.payer]
            )?;

            let mut buy_txs = Vec::new();
            for wallet in bundle_wallets {
                let wallet_pubkey = wallet.pubkey();
                let mut buy_instructions = Vec::new();

                buy_instructions.push(ComputeBudgetInstruction::set_compute_unit_limit(COMPUTE_UNIT_LIMIT_POWER_BUMP));
                buy_instructions.push(ComputeBudgetInstruction::set_compute_unit_price(micro_lamports_fee));

                if self.use_pumpfun {
                    let (bonding_curve, _) = Pubkey::find_program_address(
                        &[b"bonding-curve", token_mint.as_ref()],
                        &Pubkey::from_str(PUMP_PROGRAM_ID)?
                    );

                    let curve_info = self.rpc_client.get_account(&bonding_curve)?;
                    let creator_pubkey = if curve_info.data.len() >= 81 {
                        Pubkey::try_from(&curve_info.data[49..81])?
                    } else {
                        return Err(MakerError::RpcError("Invalid bonding curve account data".to_string()));
                    };
                    let (creator_vault, _) = Pubkey::find_program_address(
                        &[b"creator-vault", creator_pubkey.as_ref()],
                        &Pubkey::from_str(PUMP_PROGRAM_ID)?
                    );

                    let user_ata = spl_associated_token_account::get_associated_token_address(&wallet_pubkey, &token_mint);
                    let a_bonding_curve = spl_associated_token_account::get_associated_token_address(&bonding_curve, &token_mint);

                    buy_instructions.push(spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                        &wallet_pubkey,
                        &wallet_pubkey,
                        &token_mint,
                        &spl_token::id()
                    ));

                    let tokens_to_receive = buy_amount_lamports;

                    let mut buy_data = vec![
                        0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
                    ];
                    buy_data[8..16].copy_from_slice(&tokens_to_receive.to_le_bytes());
                    buy_data[16..24].copy_from_slice(&buy_amount_lamports.to_le_bytes());

                    let buy_instruction = Instruction {
                        program_id: Pubkey::from_str(PUMP_PROGRAM_ID)?,
                        accounts: vec![
                            AccountMeta::new_readonly(Pubkey::from_str(PUMP_GLOBAL)?, false),
                            AccountMeta::new(Pubkey::from_str(PUMP_FEE_RECIPIENT)?, false),
                            AccountMeta::new(token_mint, false),
                            AccountMeta::new(bonding_curve, false),
                            AccountMeta::new(a_bonding_curve, false),
                            AccountMeta::new(user_ata, false),
                            AccountMeta::new(wallet_pubkey, true),
                            AccountMeta::new_readonly(system_program::id(), false),
                            AccountMeta::new_readonly(spl_token::id(), false),
                            AccountMeta::new(creator_vault, false),
                            AccountMeta::new_readonly(Pubkey::from_str(PUMP_EVENT_AUTHORITY)?, false),
                            AccountMeta::new_readonly(Pubkey::from_str(PUMP_PROGRAM_ID)?, false),
                        ],
                        data: buy_data,
                    };

                    buy_instructions.push(buy_instruction);

                    let buy_message = TransactionMessage::try_compile(
                        &wallet_pubkey,
                        &buy_instructions,
                        &[],
                        recent_blockhash,
                    ).map_err(|e| MakerError::TransactionError(format!("Failed to compile buy message: {}", e)))?;

                    let buy_tx = VersionedTransaction::try_new(
                        solana_sdk::message::VersionedMessage::V0(buy_message),
                        &[wallet],
                    ).map_err(|e| MakerError::TransactionError(format!("Failed to create buy transaction: {}", e)))?;

                    buy_txs.push(buy_tx);
                } else if self.use_pumpswap {
                    let quote_token_account = spl_associated_token_account::get_associated_token_address(&wallet_pubkey, &NATIVE_MINT);
                    buy_instructions.push(spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                        &wallet_pubkey,
                        &wallet_pubkey,
                        &NATIVE_MINT,
                        &spl_token::id()
                    ));

                    let max_sol_cost = buy_amount_lamports + (buy_amount_lamports * 15 / 100) + 2;
                    buy_instructions.push(solana_sdk::system_instruction::transfer(
                        &wallet_pubkey,
                        &quote_token_account,
                        max_sol_cost
                    ));

                    buy_instructions.push(spl_token::instruction::sync_native(
                        &spl_token::id(),
                        &quote_token_account
                    )?);

                    let base_token_account = spl_associated_token_account::get_associated_token_address(&wallet_pubkey, &token_mint);
                    buy_instructions.push(spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                        &wallet_pubkey,
                        &wallet_pubkey,
                        &token_mint,
                        &spl_token::id()
                    ));

                    let protocol_fee_recipient = Pubkey::from_str(PROTOCOL_FEE_RECIPIENT)?;
                    buy_instructions.push(spl_associated_token_account::instruction::create_associated_token_account_idempotent(
                        &wallet_pubkey,
                        &protocol_fee_recipient,
                        &NATIVE_MINT,
                        &spl_token::id()
                    ));

                    let pool = self.pump_swap.get_pool_address(&token_mint.to_string())
                        .map_err(|e| MakerError::TokenError(e.to_string()))?;
                    let pool_base_account = spl_associated_token_account::get_associated_token_address(&pool, &token_mint);
                    let pool_quote_account = spl_associated_token_account::get_associated_token_address(&pool, &NATIVE_MINT);
                    let protocol_fee_recipient_token_account = spl_associated_token_account::get_associated_token_address(&protocol_fee_recipient, &NATIVE_MINT);

                    let accounts_to_verify = vec![
                        pool,
                        pool_base_account,
                        pool_quote_account,
                        protocol_fee_recipient,
                        protocol_fee_recipient_token_account,
                        token_mint,
                        NATIVE_MINT,
                    ];
                    let accounts = self.rpc_client.get_multiple_accounts(&accounts_to_verify)?;
                    for (i, account) in accounts.iter().enumerate() {
                        if account.is_none() {
                            return Err(MakerError::RpcError(format!("Account not found: {}", accounts_to_verify[i])));
                        }
                    }

                    let pool_data = accounts[0].as_ref().ok_or_else(|| MakerError::RpcError("Failed to fetch pool data".to_string()))?;
                    let coin_creator = Pubkey::new_from_array(pool_data.data[211..243].try_into()?);
                    let (vault_authority, _) = Pubkey::find_program_address(
                        &[b"creator_vault", coin_creator.as_ref()],
                        &Pubkey::from_str(PUMP_AMM_PROGRAM_ID)?
                    );
                    let vault_ata = spl_associated_token_account::get_associated_token_address(&vault_authority, &NATIVE_MINT);

                    let pool_base_reserves = u64::from_le_bytes(accounts[1].as_ref().unwrap().data[64..72].try_into().unwrap()) as u128;
                    let pool_quote_reserves = u64::from_le_bytes(accounts[2].as_ref().unwrap().data[64..72].try_into().unwrap()) as u128;
                    
                    let denominator = 10000 + LP_FEE_BASIS_POINTS + PROTOCOL_FEE_BASIS_POINTS + COIN_CREATOR_FEE_BASIS_POINTS;
                    let actual_swap_amount = ((buy_amount_lamports as u128) * 10000) / (denominator as u128);
                    
                    let k = pool_base_reserves * pool_quote_reserves;
                    let new_quote_reserves = pool_quote_reserves + actual_swap_amount - 1;
                    let new_base_reserves = k / new_quote_reserves;
                    let base_amount_out = (pool_base_reserves - new_base_reserves) as u64;

                    let mut buy_data = vec![0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea];
                    buy_data.extend_from_slice(&base_amount_out.to_le_bytes());
                    buy_data.extend_from_slice(&max_sol_cost.to_le_bytes());

                    buy_instructions.push(Instruction {
                        program_id: Pubkey::from_str(PUMP_AMM_PROGRAM_ID)?,
                        accounts: vec![
                            AccountMeta::new(pool, false),
                            AccountMeta::new(wallet_pubkey, true),
                            AccountMeta::new_readonly(Pubkey::from_str(GLOBAL_CONFIG)?, false),
                            AccountMeta::new_readonly(token_mint, false),
                            AccountMeta::new_readonly(NATIVE_MINT, false),
                            AccountMeta::new(base_token_account, false),
                            AccountMeta::new(quote_token_account, false),
                            AccountMeta::new(pool_base_account, false),
                            AccountMeta::new(pool_quote_account, false),
                            AccountMeta::new_readonly(protocol_fee_recipient, false),
                            AccountMeta::new(protocol_fee_recipient_token_account, false),
                            AccountMeta::new_readonly(spl_token::id(), false),
                            AccountMeta::new_readonly(spl_token::id(), false),
                            AccountMeta::new_readonly(system_program::id(), false),
                            AccountMeta::new_readonly(spl_associated_token_account::id(), false),
                            AccountMeta::new_readonly(Pubkey::from_str(EVENT_AUTH)?, false),
                            AccountMeta::new_readonly(Pubkey::from_str(PUMP_AMM_PROGRAM_ID)?, false),
                            AccountMeta::new(vault_ata, false),
                            AccountMeta::new_readonly(vault_authority, false),
                        ],
                        data: buy_data,
                    });

                    buy_instructions.push(spl_token::instruction::close_account(
                        &spl_token::id(),
                        &quote_token_account,
                        &wallet_pubkey,
                        &wallet_pubkey,
                        &[],
                    )?);

                let buy_message = TransactionMessage::try_compile(
                    &wallet_pubkey,
                    &buy_instructions,
                    &[],
                    recent_blockhash,
                    ).map_err(|e| MakerError::TransactionError(format!("Failed to compile buy message: {}", e)))?;
                
                let buy_tx = VersionedTransaction::try_new(
                    solana_sdk::message::VersionedMessage::V0(buy_message),
                        &[wallet],
                    ).map_err(|e| MakerError::TransactionError(format!("Failed to create buy transaction: {}", e)))?;
                
                buy_txs.push(buy_tx);
                }
            }

            let mut bundle_txs = vec![funding_tx];
            bundle_txs.extend(buy_txs);

            let jito_uuid = Self::get_jito_uuid();
            let bundle_id = Self::send_jito_bundle(bundle_txs, jito_uuid.as_deref()).await?;
            println!("TXID: {}", bundle_id);
            
            bundle_urls.push(bundle_id);
            
            if bundle_index < total_bundles - 1 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }

        Ok(bundle_urls.into_iter().map(|url| (1, url)).collect())
    }

    async fn send_jito_bundle(
        txs: Vec<VersionedTransaction>,
        jito_uuid: Option<&str>,
    ) -> Result<String, MakerError> {
        let client = Client::new();
        let send_to_all = env::var("SEND_TO_ALL").unwrap_or_else(|_| "true".to_string()) == "true";

        let bundle_base64: Vec<String> = txs.iter()
            .map(|tx| {
                let serialized = bincode::serialize(tx)
                    .map_err(|e| MakerError::TransactionError(format!("Failed to serialize transaction: {}", e)))?;
                Ok::<String, MakerError>(base64::engine::general_purpose::STANDARD.encode(serialized))
            })
            .collect::<Result<Vec<String>, MakerError>>()?;

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
                Err(MakerError::TransactionError("Failed to send bundle to any block engine".to_string()))
            }
        } else {
            let block_engine = env::var("BLOCK_ENGINE").map_err(|_| MakerError::TransactionError("BLOCK_ENGINE must be set".to_string()))?;
            let jito_uuid = jito_uuid.map(|u| format!("?uuid={}", u)).unwrap_or_default();
            
            let res = client
                .post(format!("{}/api/v1/bundles{}", block_engine, jito_uuid))
                .json(&bundle_request)
                .send()
                .await
                .map_err(|e| MakerError::TransactionError(e.to_string()))?;

            let status = res.status();
            match res.text().await {
                Ok(text) => {
                    if text.is_empty() {
                        return Err(MakerError::TransactionError("Empty response from bundle send".to_string()));
                    }
                    match serde_json::from_str::<serde_json::Value>(&text) {
                        Ok(body) => {
                            if status.is_success() {
                                if let Some(bundle_id) = body.get("result") {
                                    let bundle_id = bundle_id.to_string().trim_matches('"').to_string();
                                    Ok(format!("https://explorer.jito.wtf/bundle/{}", bundle_id))
                                } else {
                                    Err(MakerError::TransactionError("No bundle ID returned".to_string()))
                                }
                            } else {
                                let error = body.get("error")
                                    .and_then(|e| e.get("message"))
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("Unknown error");
                                Err(MakerError::TransactionError(format!("Error sending bundle: {}", error)))
                            }
                        }
                        Err(e) => {
                            Err(MakerError::TransactionError(format!("Failed to parse bundle response: {}", e)))
                        }
                    }
                }
                Err(e) => Err(MakerError::TransactionError(format!("Failed to read bundle response: {}", e)))
            }
        }
    }
} 