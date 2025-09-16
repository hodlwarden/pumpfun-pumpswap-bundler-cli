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
struct TokenMetadata {
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
    rpc_client: RpcClient,
    dex: PumpDex,
    payer: Keypair,
    block_engine: String,
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

    fn load_wallets(&self) -> Result<Vec<Keypair>, BundlerError> {
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

    async fn load_metadata(&self) -> Result<(TokenMetadata, String), BundlerError> {
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

    async fn create_token_mint_account(&self) -> Result<Keypair, BundlerError> {
        // Try to fetch from API first, fallback to generating new keypair
        match Self::fetch_keypair_from_api().await {
            Ok(keypair) => Ok(keypair),
            Err(_) => Ok(Keypair::new())
        }
    }

    pub async fn execute_create_and_bundle(
        &self,
        total_buy_amount: f64,
        jito_tip: f64,
        dev_buy_amount: Option<f64>,
    ) -> Result<(), BundlerError> {
        let wallets = self.load_wallets()?;
        let (metadata, metadata_uri) = self.load_metadata().await?;
        let mint_keypair = self.create_token_mint_account().await?;
        let mint_pubkey = mint_keypair.pubkey();

        let (mut instructions, bonding_curve) = self.create_token_creation_instruction(
            &mint_keypair,
            &metadata,
            &metadata_uri,
            total_buy_amount,
        )?;

        if let Some(dev_amount) = dev_buy_amount {
            if dev_amount > 0.0 {
                let dev_amount_lamports = (dev_amount * 1_000_000_000.0) as u64;
                
                let a_bonding_curve = spl_associated_token_account::get_associated_token_address(
                    &bonding_curve,
                    &mint_pubkey,
                );
                let creator_pubkey = Pubkey::from_str(MINT_AUTHORITY)?;
                let (creator_vault, _) = self.dex.get_creator_vault(&creator_pubkey);

                let dev_ata = spl_associated_token_account::get_associated_token_address(
                    &self.payer.pubkey(),
                    &mint_pubkey,
                );
                let dev_ata_ix = associated_token_instruction::create_associated_token_account_idempotent(
                    &self.payer.pubkey(),
                    &self.payer.pubkey(),
                    &mint_pubkey,
                    &spl_token::id(),
                );

                let (tokens_to_receive, max_sol_cost, _) = self.dex.get_amount_out(
                    dev_amount_lamports,
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
                        AccountMeta::new(mint_pubkey, false),
                        AccountMeta::new(bonding_curve, false),
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

                instructions.push(dev_ata_ix);
                instructions.push(dev_buy_ix);
            }
        }

        let num_wallets = wallets.len() as u64;
        if num_wallets == 0 {
            return Err(BundlerError::WalletError("No wallets provided.".to_string()));
        }
        let buy_lamports_per_wallet = ((total_buy_amount * 1_000_000_000.0) as u64) / num_wallets;
        let total_spend = buy_lamports_per_wallet * num_wallets;
        let total_fee = (total_spend * TRANSFER_FEE_BPS) / FEE_DENOMINATOR;
        let tip_lamports = (jito_tip * 1_000_000_000.0) as u64;

        let tip_address = Self::get_random_tip_address();
        let jito_tip_ix = system_instruction::transfer(
            &wallets[0].pubkey(),
            &tip_address,
            tip_lamports,
        );

        let fee_transfer_ix = system_instruction::transfer(
            &wallets[0].pubkey(),
            &Pubkey::from_str(TRANSFER_WALLET).unwrap(),
            total_fee,
        );

        instructions.push(jito_tip_ix);

        // Create LUT first via RPC
        let creator_pubkey = Pubkey::from_str(MINT_AUTHORITY)?;
        let (creator_vault, _) = self.dex.get_creator_vault(&creator_pubkey);
        
        // Create and extend LUT
        let slot = self.rpc_client.get_slot_with_commitment(CommitmentConfig::finalized())?;
        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        
        let (create_instruction, lookup_table_address) = create_lookup_table(
            self.payer.pubkey(),
            self.payer.pubkey(),
            slot,
        );

        let mut addresses_to_add = Vec::new();
        
        // Add wallet accounts and their ATAs
        for wallet in wallets.iter() {
            let wallet_pubkey = wallet.pubkey();
            addresses_to_add.push(wallet_pubkey);
            
            let ata = spl_associated_token_account::get_associated_token_address(
                &wallet_pubkey,
                &mint_pubkey,
            );
            addresses_to_add.push(ata);
        }

        let additional_addresses = vec![
            Pubkey::from_str(PUMP_GLOBAL)?,
            Pubkey::from_str(PUMP_FEE_ACCOUNT)?,
            Pubkey::from_str(PUMP_EVENT_AUTHORITY)?,
            Pubkey::from_str(PUMP_PROGRAM_ID)?,
            system_program::id(),
            spl_token::id(),
            creator_vault,
            self.dex.global,
            self.dex.fee_recipient,
            self.dex.event_authority,
            self.dex.program_id,
            mint_pubkey,
            bonding_curve,
            spl_associated_token_account::get_associated_token_address(&bonding_curve, &mint_pubkey),
        ];

        addresses_to_add.extend_from_slice(&additional_addresses);

        let extend_instruction = extend_lookup_table(
            lookup_table_address,
            self.payer.pubkey(),
            Some(self.payer.pubkey()),
            addresses_to_add,
        );

        // Send LUT creation via RPC
        let message = Message::try_compile(
            &self.payer.pubkey(),
            &[create_instruction, extend_instruction],
            &[],
            recent_blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[&self.payer],
        )?;

        self.rpc_client.send_transaction_with_config(&transaction, RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: Some(CommitmentConfig::processed().commitment),
            encoding: None,
            max_retries: Some(5),
            min_context_slot: None
        })?;

        // Wait for LUT to be ready
        let max_retries = 20;
        let mut retries = 0;
        let lut_account = loop {
            match self.rpc_client.get_account(&lookup_table_address) {
                Ok(account) => {
                    let table = AddressLookupTable::deserialize(&account.data)
                        .map_err(|e| BundlerError::TransactionError(e.to_string()))?;
                    break AddressLookupTableAccount {
                        key: lookup_table_address,
                        addresses: table.addresses.to_vec(),
                    };
                },
                Err(_) => {
                    if retries >= max_retries {
                        return Err(BundlerError::TransactionError("Timed out waiting for LUT creation".to_string()));
                    }
                    retries += 1;
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        };

        // Now create bundle with token creation and buy transactions
        let message = TransactionMessage::try_compile(
            &self.payer.pubkey(),
            &instructions,
            &[lut_account.clone()],
            self.rpc_client.get_latest_blockhash()?,
        )?;

        let mut first_tx_signers = Vec::new();
        first_tx_signers.push(&self.payer);
        first_tx_signers.push(&mint_keypair);
        first_tx_signers.push(&wallets[0]);
        
        let first_tx = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &first_tx_signers,
        )?;

        let chunk_size = 4; 
        let total_chunks = (wallets.len() + chunk_size - 1) / chunk_size;
        let max_chunks = 4; 
        let actual_chunks = std::cmp::min(total_chunks, max_chunks);

        let mut all_bundles = vec![first_tx];

        let mut virtual_token_reserves = 1_073_000_000_000_000u64;
        let mut virtual_sol_reserves = 30_000_000_000u64; 

        for chunk_index in 0..actual_chunks {
            let start = chunk_index * chunk_size;
            let end = std::cmp::min(start + chunk_size, wallets.len());
            let chunk_wallets = &wallets[start..end];
            
            let mut chunk_instructions = Vec::new();
            let mut chunk_signers = Vec::new();
            
            chunk_signers.push(&self.payer);
            
            for wallet in chunk_wallets {
                chunk_signers.push(wallet);
            }
            
            for wallet in chunk_wallets {
                let wallet_ata = spl_associated_token_account::get_associated_token_address(
                    &wallet.pubkey(),
                    &mint_pubkey,
                );
                
                let create_ata_ix = associated_token_instruction::create_associated_token_account_idempotent(
                    &wallet.pubkey(),
                    &wallet.pubkey(),
                    &mint_pubkey,
                    &spl_token::id(),
                );
                
                let (tokens_to_receive, _, _) = self.dex.get_amount_out(
                    buy_lamports_per_wallet,
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
                buy_instruction_data[16..24].copy_from_slice(&buy_lamports_per_wallet.to_le_bytes());
                
                let global_idx = lut_account.addresses.iter().position(|&addr| addr == self.dex.global)
                    .ok_or_else(|| BundlerError::TransactionError("Global address not found in LUT".to_string()))?;
                let fee_recipient_idx = lut_account.addresses.iter().position(|&addr| addr == self.dex.fee_recipient)
                    .ok_or_else(|| BundlerError::TransactionError("Fee recipient not found in LUT".to_string()))?;
                let mint_idx = lut_account.addresses.iter().position(|&addr| addr == mint_pubkey)
                    .ok_or_else(|| BundlerError::TransactionError("Mint not found in LUT".to_string()))?;
                let bonding_curve_idx = lut_account.addresses.iter().position(|&addr| addr == bonding_curve)
                    .ok_or_else(|| BundlerError::TransactionError("Bonding curve not found in LUT".to_string()))?;
                let a_bonding_curve_idx = lut_account.addresses.iter().position(|&addr| addr == spl_associated_token_account::get_associated_token_address(&bonding_curve, &mint_pubkey))
                    .ok_or_else(|| BundlerError::TransactionError("A bonding curve not found in LUT".to_string()))?;
                let wallet_ata_idx = lut_account.addresses.iter().position(|&addr| addr == wallet_ata)
                    .ok_or_else(|| BundlerError::TransactionError("Wallet ATA not found in LUT".to_string()))?;
                let wallet_idx = lut_account.addresses.iter().position(|&addr| addr == wallet.pubkey())
                    .ok_or_else(|| BundlerError::TransactionError("Wallet not found in LUT".to_string()))?;
                let system_program_idx = lut_account.addresses.iter().position(|&addr| addr == system_program::id())
                    .ok_or_else(|| BundlerError::TransactionError("System program not found in LUT".to_string()))?;
                let token_program_idx = lut_account.addresses.iter().position(|&addr| addr == spl_token::id())
                    .ok_or_else(|| BundlerError::TransactionError("Token program not found in LUT".to_string()))?;
                let creator_vault_idx = lut_account.addresses.iter().position(|&addr| addr == creator_vault)
                    .ok_or_else(|| BundlerError::TransactionError("Creator vault not found in LUT".to_string()))?;
                let event_authority_idx = lut_account.addresses.iter().position(|&addr| addr == self.dex.event_authority)
                    .ok_or_else(|| BundlerError::TransactionError("Event authority not found in LUT".to_string()))?;
                let program_id_idx = lut_account.addresses.iter().position(|&addr| addr == self.dex.program_id)
                    .ok_or_else(|| BundlerError::TransactionError("Program ID not found in LUT".to_string()))?;

                let buy_instruction = Instruction {
                    program_id: self.dex.program_id,
                    accounts: vec![
                        AccountMeta::new_readonly(lut_account.addresses[global_idx], false),
                        AccountMeta::new(lut_account.addresses[fee_recipient_idx], false),
                        AccountMeta::new(lut_account.addresses[mint_idx], false),
                        AccountMeta::new(lut_account.addresses[bonding_curve_idx], false),
                        AccountMeta::new(lut_account.addresses[a_bonding_curve_idx], false),
                        AccountMeta::new(lut_account.addresses[wallet_ata_idx], false),
                        AccountMeta::new(lut_account.addresses[wallet_idx], true),
                        AccountMeta::new_readonly(lut_account.addresses[system_program_idx], false),
                        AccountMeta::new_readonly(lut_account.addresses[token_program_idx], false),
                        AccountMeta::new(lut_account.addresses[creator_vault_idx], false),
                        AccountMeta::new_readonly(lut_account.addresses[event_authority_idx], false),
                        AccountMeta::new_readonly(lut_account.addresses[program_id_idx], false),
                    ],
                    data: buy_instruction_data,
                };
                
                chunk_instructions.push(create_ata_ix);
                chunk_instructions.push(buy_instruction);
                
                virtual_token_reserves += tokens_to_receive;
                virtual_sol_reserves += buy_lamports_per_wallet;
            }
            
            if chunk_index == 0 {
                chunk_instructions.push(fee_transfer_ix.clone());
            }
            
            let message = TransactionMessage::try_compile(
                &self.payer.pubkey(),
                &chunk_instructions,
                &[lut_account.clone()],
                self.rpc_client.get_latest_blockhash()?,
            )?;
            
            let chunk_tx = VersionedTransaction::try_new(
                VersionedMessage::V0(message),
                &chunk_signers,
            )?;
            
            all_bundles.push(chunk_tx);
        }

        self.send_bundle_to_jito(&all_bundles, &self.block_engine).await?;

        // Deactivate LUT first
        let deactivate_ix = deactivate_lookup_table(
            lut_account.key,
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

        self.rpc_client.send_transaction_with_config(&transaction, RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: Some(CommitmentConfig::processed().commitment),
            encoding: None,
            max_retries: Some(5),
            min_context_slot: None
        })?;

        // Wait for deactivation to complete and confirm
        let max_retries = 32;
        let mut retries = 0;
        while retries < max_retries {
            match self.rpc_client.get_account(&lut_account.key) {
                Ok(account) => {
                    let table = AddressLookupTable::deserialize(&account.data)
                        .map_err(|e| BundlerError::TransactionError(e.to_string()))?;
                    if table.meta.deactivation_slot != u64::MAX {
                        // Add a small delay to ensure deactivation is processed
                        thread::sleep(Duration::from_secs(2));
                        break;
                    }
                },
                Err(_) => {}
            }
            retries += 1;
            thread::sleep(Duration::from_millis(500));
        }

        // Now close the LUT
        let close_ix = close_lookup_table(
            lut_account.key,
            self.payer.pubkey(),
            self.payer.pubkey(),
        );

        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        
        // Add compute budget instruction
        let compute_budget_ix = ComputeBudgetInstruction::set_compute_unit_limit(200_000);
        
        let message = TransactionMessage::try_compile(
            &self.payer.pubkey(),
            &[compute_budget_ix, close_ix],
            &[lut_account],
            recent_blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[&self.payer],
        )?;

        self.rpc_client.send_transaction_with_config(&transaction, RpcSendTransactionConfig {
            skip_preflight: true,
            preflight_commitment: Some(CommitmentConfig::processed().commitment),
            encoding: None,
            max_retries: Some(5),
            min_context_slot: None
        })?;

        Ok(())
    }

    async fn send_bundle_to_jito(
        &self,
        txs: &[VersionedTransaction],
        _engine_url: &str,
    ) -> Result<String, BundlerError> {
        let client = Client::new();
        let send_to_all = env::var("SEND_TO_ALL").unwrap_or_else(|_| "true".to_string()) == "true";
        let jito_uuid = env::var("UUID").ok();

        let bundle_base64: Vec<String> = txs.iter()
            .enumerate()
            .map(|(i, tx)| {
                let serialized = bincode::serialize(tx)
                    .map_err(|e| BundlerError::TransactionError(format!("Failed to serialize transaction {}: {}", i, e)))?;
                
                if serialized.len() > 1232 {
                    let tx_type = if i == 0 {
                        "creation"
                    } else {
                        &format!("buy #{}", i)
                    };
                    return Err(BundlerError::TransactionError(format!(
                        "Transaction {} ({}) is too large: {} bytes (max 1232)",
                        i, tx_type, serialized.len()
                    )));
                }
                
                let base64_str = BASE64.encode(serialized);
                Ok::<String, BundlerError>(base64_str)
            })
            .collect::<Result<Vec<String>, BundlerError>>()?;

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
                let jito_uuid = jito_uuid.as_deref().map(|u| format!("?uuid={}", u)).unwrap_or_default();
                
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
                Err(BundlerError::TransactionError("Failed to send bundle to any block engine".to_string()))
            }
        } else {
            let block_engine = env::var("BLOCK_ENGINE").map_err(|_| BundlerError::TransactionError("BLOCK_ENGINE must be set".to_string()))?;
            let jito_uuid = jito_uuid.as_deref().map(|u| format!("?uuid={}", u)).unwrap_or_default();
            
        let res = client
                .post(format!("{}/api/v1/bundles{}", block_engine, jito_uuid))
            .json(&bundle_request)
            .send()
            .await
            .map_err(|e| BundlerError::TransactionError(format!("Failed to send bundle: {}", e)))?;

        let status = res.status();
        let body = res.json::<serde_json::Value>().await
            .map_err(|e| BundlerError::TransactionError(format!("Failed to parse response: {}", e)))?;

        if status.is_success() {
            if let Some(bundle_id) = body.get("result") {
                let bundle_id = bundle_id.to_string().trim_matches('"').to_string();
                let explorer_url = format!("https://explorer.jito.wtf/bundle/{}", bundle_id);
                println!("Bundle ID: {}", bundle_id);
                Ok(explorer_url)
            } else {
                Ok("Bundle sent successfully but no ID returned".to_string())
            }
        } else {
            let error = body.get("error").and_then(|e| e.get("message")).and_then(|m| m.as_str()).unwrap_or("Unknown error");
            Err(BundlerError::TransactionError(format!("Error sending bundle: {}", error)))
            }
        }
    }

    fn create_token_creation_instruction(
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
        let owner_fee_amount: u64 = (total_bundle_amount * 1) / 100;
        
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
}