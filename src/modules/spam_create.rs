use anyhow::Result;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::Keypair,
    transaction::VersionedTransaction,
    instruction::{Instruction, AccountMeta},
    message::v0::Message as TransactionMessage,
    system_program,
    signer::Signer,
    compute_budget::ComputeBudgetInstruction,
    commitment_config::CommitmentConfig,
    native_token::sol_to_lamports,
    system_instruction,
};
use std::str::FromStr;
use reqwest::Client;
use serde_json::json;
use base64;
use bincode::serialize;
use dotenv::dotenv;
use std::env;
use std::time::Duration;
use tokio;
use std::sync::Arc;
use std::fs;
use std::path::Path;
use serde::{Serialize, Deserialize};
use spl_associated_token_account::get_associated_token_address;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

const PUMP_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const PUMP_GLOBAL: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
const PUMP_EVENT_AUTHORITY: &str = "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1";
const PUMP_FEE_ACCOUNT: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
const OWNER_FEE_ACCOUNT: &str = "FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT";
const MINT_AUTHORITY: &str = "TSLvdd1pWpHVjahSpsvCXUbgwsL3JAcvokwaKt1eokM";
const METAPLEX_PROGRAM_ID: &str = "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s";
const COMPUTE_UNIT_LIMIT: u32 = 1_400_000;

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

struct TokenReserves {
    virtual_sol_reserves: u64,
    virtual_token_reserves: u64,
}

fn reserves_calculation(amount: u64, reserves: &mut TokenReserves, fee: u8) -> (u64, u64) {
    let mut fee_amount = (amount as f64 * (fee as f64 / 100.0)) as u64;
    if fee_amount < 1_000 {
        fee_amount = 1_000;
    }
    let amount_after_fee = amount - fee_amount;
    
    let tokens_to_receive = (reserves.virtual_token_reserves as f64 * (amount_after_fee as f64 / (reserves.virtual_sol_reserves as f64 + amount_after_fee as f64))) as u64;
    
    reserves.virtual_sol_reserves += amount_after_fee;
    reserves.virtual_token_reserves -= tokens_to_receive;
    
    (tokens_to_receive, amount)
}

pub struct SpamCreate {
    rpc_client: RpcClient,
    payer: Keypair,
}

impl SpamCreate {
    pub fn new(rpc_url: String) -> Result<Self> {
        dotenv().ok();
        let payer_privkey = env::var("PAYER").expect("PAYER not set in .env file");
        let bytes = bs58::decode(&payer_privkey).into_vec()?;
        let payer = Keypair::from_bytes(&bytes)?;
        
        Ok(Self {
            rpc_client: RpcClient::new(rpc_url),
            payer,
        })
    }

    async fn load_metadata(&self) -> Result<(TokenMetadata, String)> {
        let metadata_path = Path::new("metadata/metadata.json");
        if !metadata_path.exists() {
            return Err(anyhow::anyhow!("metadata.json not found in metadata directory"));
        }

        let metadata_content = fs::read_to_string(metadata_path)?;
        let metadata: TokenMetadata = serde_json::from_str(&metadata_content)?;

        if metadata.name.len() > 32 {
            return Err(anyhow::anyhow!("Token name exceeds 32 characters limit"));
        }

        if metadata.symbol.len() > 10 {
            return Err(anyhow::anyhow!("Token symbol exceeds 10 characters limit"));
        }

        let image_path = Path::new(&metadata.filePath);
        if !image_path.exists() {
            return Err(anyhow::anyhow!("Image file not found at path: {}", metadata.filePath));
        }

        let image_data = fs::read(image_path)?;
        let image_mime = match image_path.extension().and_then(|ext| ext.to_str()) {
            Some("png") => "image/png",
            Some("jpg") | Some("jpeg") => "image/jpeg",
            _ => return Err(anyhow::anyhow!("Unsupported image format")),
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

        let metadata_json = serde_json::to_string(&metaplex_metadata)?;
        let metadata_uri = self.upload_metadata(&metadata_json, &image_data).await?;

        Ok((metadata, metadata_uri))
    }

    async fn upload_metadata(&self, metadata_json: &str, image_data: &[u8]) -> Result<String> {
        let client = Client::new();
        
        let image_form = reqwest::multipart::Form::new()
            .part("file", reqwest::multipart::Part::bytes(image_data.to_vec())
                .file_name("image.png"));
        
        let image_response = client
            .post("https://api.pinata.cloud/pinning/pinFileToIPFS")
            .header("pinata_api_key", env::var("PINATA_API_KEY").unwrap_or_default())
            .header("pinata_secret_api_key", env::var("PINATA_SECRET_KEY").unwrap_or_default())
            .multipart(image_form)
            .send()
            .await?;
            
        let image_result = image_response.json::<serde_json::Value>().await?;
        let image_cid = image_result["IpfsHash"].as_str()
            .ok_or_else(|| anyhow::anyhow!("Failed to get image CID"))?;
        
        let mut metadata: serde_json::Value = serde_json::from_str(metadata_json)?;
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
            .part("file", reqwest::multipart::Part::bytes(serde_json::to_vec(&metadata)?)
                .file_name("metadata.json"));
        
        let metadata_response = client
            .post("https://api.pinata.cloud/pinning/pinFileToIPFS")
            .header("pinata_api_key", env::var("PINATA_API_KEY").unwrap_or_default())
            .header("pinata_secret_api_key", env::var("PINATA_SECRET_KEY").unwrap_or_default())
            .multipart(metadata_form)
            .send()
            .await?;
            
        let metadata_result = metadata_response.json::<serde_json::Value>().await?;
        let metadata_cid = metadata_result["IpfsHash"].as_str()
            .ok_or_else(|| anyhow::anyhow!("Failed to get metadata CID"))?;
        
        Ok(format!("ipfs://{}", metadata_cid))
    }

    pub async fn run_spam_create(&self, delay_ms: u64) -> Result<()> {
        let (metadata, metadata_uri) = self.load_metadata().await?;
        let mut _attempt = 1;
        loop {
            let token_keypair = Keypair::new();
            let token_pubkey = token_keypair.pubkey();

            let (bonding_curve, _) = Pubkey::find_program_address(
                &[b"bonding-curve", token_pubkey.as_ref()],
                &Pubkey::from_str(PUMP_PROGRAM_ID)?
            );

            let (metadata_account, _) = Pubkey::find_program_address(
                &[b"metadata", Pubkey::from_str(METAPLEX_PROGRAM_ID)?.as_ref(), token_pubkey.as_ref()],
                &Pubkey::from_str(METAPLEX_PROGRAM_ID)?
            );

            let a_bonding_curve = get_associated_token_address(&bonding_curve, &token_pubkey);

            let mut token_data = Vec::from([0x18, 0x1e, 0xc8, 0x28, 0x05, 0x1c, 0x07, 0x77]);
            
            let metadata_json = format!(
                r#"{{"name":"{}","symbol":"{}","description":"{}","image":"{}"}}"#,
                metadata.name,
                metadata.symbol,
                metadata.description,
                metadata_uri
            );
            
            for field in [&metadata.name, &metadata.symbol, &metadata_json] {
                let len = field.len() as u32;
                if len > u16::MAX as u32 {
                    return Err(anyhow::anyhow!("Field length too large: {}", len));
                }
                token_data.extend_from_slice(&len.to_le_bytes());
                token_data.extend_from_slice(field.as_bytes());
            }
            token_data.extend_from_slice(&Pubkey::from_str(MINT_AUTHORITY)?.to_bytes());
            token_data.push(100);
            token_data.push(1);

            let create_instruction = Instruction {
                program_id: Pubkey::from_str(PUMP_PROGRAM_ID)?,
                accounts: vec![
                    AccountMeta::new(token_pubkey, true),
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
                    AccountMeta::new_readonly(Pubkey::from_str(PUMP_EVENT_AUTHORITY)?, false),
                    AccountMeta::new_readonly(Pubkey::from_str(PUMP_PROGRAM_ID)?, false),
                ],
                data: extend_discriminator,
            };

            let fee_amount = 100_000;
            let fee_transfer_instruction = system_instruction::transfer(
                &self.payer.pubkey(),
                &Pubkey::from_str(PUMP_FEE_ACCOUNT)?,
                fee_amount,
            );

            let owner_fee_transfer_instruction = system_instruction::transfer(
                &self.payer.pubkey(),
                &Pubkey::from_str(OWNER_FEE_ACCOUNT)?,
                fee_amount,
            );

            let instructions = vec![
                ComputeBudgetInstruction::set_compute_unit_limit(COMPUTE_UNIT_LIMIT),
                ComputeBudgetInstruction::set_compute_unit_price(0),
                create_instruction,
                extend_instruction,
                fee_transfer_instruction,
                owner_fee_transfer_instruction,
            ];

            let recent_blockhash = self.rpc_client.get_latest_blockhash()?;

            let message = TransactionMessage::try_compile(
                &self.payer.pubkey(),
                &instructions,
                &[],
                recent_blockhash,
            )?;

            let transaction = VersionedTransaction::try_new(
                solana_sdk::message::VersionedMessage::V0(message),
                &[&self.payer, &token_keypair],
            )?;

            match self.rpc_client.send_transaction(&transaction) {
                Ok(signature) => {
                    println!("TXID: {}", signature);
                }
                Err(e) => {
                    println!("Error: {}", e);
                }
            }

            _attempt += 1;
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
    }
} 