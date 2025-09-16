use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::{Instruction, AccountMeta},
    message::{Message, v0::Message as TransactionMessage},
    pubkey::Pubkey,
    signature::Keypair,
    signer::Signer,
    system_program,
    transaction::VersionedTransaction,
};
use solana_client::rpc_client::RpcClient;
use solana_program::program_error::ProgramError;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use rand::rngs::StdRng;
use rand::SeedableRng;
use anyhow::Result;
use crate::dex::pump::{PumpDex, PUMP_PROGRAM_ID, GLOBAL, FEE_RECIPIENT, EVENT_AUTHORITY, TRANSFER_FEE_BPS, FEE_DENOMINATOR, TRANSFER_WALLET};
use spl_associated_token_account::{
    get_associated_token_address,
    instruction::create_associated_token_account_idempotent,
};
use spl_token;
use std::str::FromStr;
use std::error::Error;
use std::thread;
use bs58;
use rand;
use rand::seq::SliceRandom;
use rand::Rng;
use crate::modules::wallet_gen::WalletGenerator;
use num_bigint::BigUint;
use num_traits::{One, Zero, ToPrimitive};
use solana_client::rpc_config::RpcSendTransactionConfig;

#[derive(Debug)]
pub enum SpamError {
    ClientError(solana_client::client_error::ClientError),
    TryFromSliceError(std::array::TryFromSliceError),
    CompileError(solana_program::message::CompileError),
    SignerError(solana_sdk::signer::SignerError),
    ProgramError(ProgramError),
    Other(String),
}

unsafe impl Send for SpamError {}
unsafe impl Sync for SpamError {}

impl std::fmt::Display for SpamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpamError::ClientError(e) => write!(f, "Error: {}", e),
            SpamError::TryFromSliceError(e) => write!(f, "Error: {}", e),
            SpamError::CompileError(e) => write!(f, "Error: {}", e),
            SpamError::SignerError(e) => write!(f, "Error: {}", e),
            SpamError::ProgramError(e) => write!(f, "Error: {}", e),
            SpamError::Other(e) => write!(f, "Error: {}", e),
        }
    }
}

impl std::error::Error for SpamError {}

impl From<solana_client::client_error::ClientError> for SpamError {
    fn from(err: solana_client::client_error::ClientError) -> Self {
        SpamError::ClientError(err)
    }
}

impl From<std::array::TryFromSliceError> for SpamError {
    fn from(err: std::array::TryFromSliceError) -> Self {
        SpamError::TryFromSliceError(err)
    }
}

impl From<solana_program::message::CompileError> for SpamError {
    fn from(err: solana_program::message::CompileError) -> Self {
        SpamError::CompileError(err)
    }
}

impl From<solana_sdk::signer::SignerError> for SpamError {
    fn from(err: solana_sdk::signer::SignerError) -> Self {
        SpamError::SignerError(err)
    }
}

impl From<ProgramError> for SpamError {
    fn from(err: ProgramError) -> Self {
        SpamError::ProgramError(err)
    }
}

pub type SpamResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>;

#[derive(Clone)]
pub struct Spam {
    rpc_url: String,
    token: String,
    wallets: Vec<String>,
    min_delay_ms: u64,
    max_delay_ms: u64,
    min_buy: f64,
    max_buy: f64,
    max_sell_percent: u8,
    rng: Arc<Mutex<StdRng>>,
    stop_flag: Arc<AtomicBool>,
    rpc_client: Arc<RpcClient>,
    contract_address: Pubkey,
    buy_amount_sol: f64,
    delay_ms: u64,
    pump_dex: PumpDex,
}

impl Spam {
    pub fn new(
        rpc_url: String,
        token: String,
        wallets: Vec<String>,
        min_delay_ms: u64,
        max_delay_ms: u64,
        min_buy: f64,
        max_buy: f64,
        max_sell_percent: u8,
    ) -> Self {
        let rng = Arc::new(Mutex::new(StdRng::from_entropy()));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let rpc_client = Arc::new(RpcClient::new(rpc_url.clone()));
        let contract_address = Pubkey::from_str(&token).unwrap();
        let buy_amount_sol = min_buy;
        let delay_ms = min_delay_ms;
        let pump_dex = PumpDex::new();
        
        Self {
            rpc_url,
            token,
            wallets,
            min_delay_ms,
            max_delay_ms,
            min_buy,
            max_buy,
            max_sell_percent,
            rng,
            stop_flag,
            rpc_client,
            contract_address,
            buy_amount_sol,
            delay_ms,
            pump_dex,
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.stop_flag.load(Ordering::SeqCst)
    }

    pub fn run(&self) -> Result<()> {
        match self.rpc_client.get_account(&self.contract_address) {
            Ok(_) => {},
            Err(e) => {
                println!("Failed to get account: {}", e);
                return Ok(());
            }
        }

        let pump_program_id = Pubkey::from_str(PUMP_PROGRAM_ID).unwrap();
        let global = Pubkey::from_str(GLOBAL).unwrap();
        let fee_recipient = Pubkey::from_str(FEE_RECIPIENT).unwrap();
        let event_authority = Pubkey::from_str(EVENT_AUTHORITY).unwrap();
        let buy_amount = if self.buy_amount_sol < 0.00001 {
            0.00001
        } else {
            self.buy_amount_sol
        };
        let buy_amount_lamports = (buy_amount * 1e9) as u64;

        let (bonding_curve, _) = Pubkey::find_program_address(
            &[b"bonding-curve", self.contract_address.as_ref()],
            &pump_program_id,
        );
        let a_bonding_curve = get_associated_token_address(&bonding_curve, &self.contract_address);

        let mut available_wallets: Vec<usize> = (0..self.wallets.len()).collect();
        let mut rng = rand::thread_rng();

        while !self.is_stopped() {
            if available_wallets.is_empty() {
                available_wallets = (0..self.wallets.len()).collect();
            }

            let wallet_idx = *available_wallets.choose(&mut rng).unwrap();
            let wallet_privkey = &self.wallets[wallet_idx];
            available_wallets.retain(|&x| x != wallet_idx);

            let bytes = bs58::decode(wallet_privkey).into_vec().expect("Failed to decode privkey");
            let keypair = solana_sdk::signer::keypair::Keypair::from_bytes(&bytes).expect("Failed to create keypair from privkey");
            let pubkey = keypair.pubkey();
            let user_ata = get_associated_token_address(&pubkey, &self.contract_address);
            let balance = self.rpc_client.get_balance(&pubkey).unwrap_or(0);
            if balance < buy_amount_lamports + 5000 {
                println!("Insufficient balance for wallet {}: {} lamports", pubkey, balance);
                continue;
            }

            let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
            let curve_info = self.rpc_client.get_account(&bonding_curve)?;
            
            if curve_info.data.len() < 81 {
                println!("Invalid curve info data length: {}", curve_info.data.len());
                return Ok(());
            }
            
            let creator_pubkey = Pubkey::try_from(&curve_info.data[49..81])?;
            let (creator_vault, _) = Pubkey::find_program_address(
                &[b"creator-vault", creator_pubkey.as_ref()],
                &pump_program_id,
            );

            let virtual_token_reserves = BigUint::from(u64::from_le_bytes(curve_info.data[8..16].try_into()?));
            let virtual_sol_reserves = BigUint::from(u64::from_le_bytes(curve_info.data[16..24].try_into()?));
            let buy_amount_big = BigUint::from(buy_amount_lamports);

            let mut instructions = vec![];

            let ata_instruction = create_associated_token_account_idempotent(
                &pubkey,
                &pubkey,
                &self.contract_address,
                &spl_token::id(),
            );
            instructions.push(ata_instruction);

            let transfer_amount_big = (&buy_amount_big * BigUint::from(TRANSFER_FEE_BPS)) / BigUint::from(FEE_DENOMINATOR);
            let transfer_amount = if transfer_amount_big < BigUint::from(1_000u64) {
                1_000u64
            } else {
                transfer_amount_big.to_u64().unwrap_or(1_000)
            };
 
            let buy_amount_after_transfer = buy_amount_lamports.saturating_sub(transfer_amount);
            let transfer_wallet = Pubkey::from_str(TRANSFER_WALLET).unwrap();
            let mut transfer_data = vec![2, 0, 0, 0];
            transfer_data.extend_from_slice(&transfer_amount.to_le_bytes());
            let transfer_instruction = Instruction {
                program_id: system_program::id(),
                accounts: vec![
                    AccountMeta::new(pubkey, true),
                    AccountMeta::new(transfer_wallet, false),
                ],
                data: transfer_data,
            };
            instructions.push(transfer_instruction);

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
            let age_transfer_instruction = Instruction {
                program_id: system_program::id(),
                accounts: vec![
                    AccountMeta::new(pubkey, true),
                    AccountMeta::new(random_target_pubkey, false),
                ],
                data: solana_sdk::system_instruction::transfer(
                    &pubkey,
                    &random_target_pubkey,
                    200_000 // 0.0002 SOL in lamports
                ).data,
            };
            instructions.push(age_transfer_instruction);
            // --- End random transfer ---

            let k = &virtual_sol_reserves * &virtual_token_reserves;
            let new_sol_reserves = &virtual_sol_reserves + &buy_amount_big;
            let new_token_reserves = &k / &new_sol_reserves;
            let tokens_to_receive = (&virtual_token_reserves * BigUint::from(buy_amount_lamports)) / &virtual_sol_reserves;
            let tokens_with_slippage = (&tokens_to_receive * BigUint::from(80u64)) / BigUint::from(100u64);
            let tokens_with_slippage_u64 = tokens_with_slippage.to_u64().unwrap_or(0);

            let mut buy_instruction_data = vec![
                0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
            ];
            buy_instruction_data[8..16].copy_from_slice(&tokens_with_slippage_u64.to_le_bytes());
            buy_instruction_data[16..24].copy_from_slice(&buy_amount_after_transfer.to_le_bytes());

            let buy_instruction = Instruction {
                program_id: pump_program_id,
                accounts: vec![
                    AccountMeta::new_readonly(global, false),
                    AccountMeta::new(fee_recipient, false),
                    AccountMeta::new(self.contract_address, false),
                    AccountMeta::new(bonding_curve, false),
                    AccountMeta::new(a_bonding_curve, false),
                    AccountMeta::new(user_ata, false),
                    AccountMeta::new(pubkey, true),
                    AccountMeta::new_readonly(system_program::id(), false),
                    AccountMeta::new_readonly(spl_token::id(), false),
                    AccountMeta::new(creator_vault, false),
                    AccountMeta::new_readonly(event_authority, false),
                    AccountMeta::new_readonly(pump_program_id, false),
                ],
                data: buy_instruction_data,
            };
            instructions.push(buy_instruction);

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

            if let Ok(signature) = self.rpc_client.send_transaction_with_config(
                &transaction,
                RpcSendTransactionConfig {
                    skip_preflight: false,
                    preflight_commitment: Some(CommitmentConfig::processed().commitment),
                    max_retries: Some(3),
                    min_context_slot: None,
                    encoding: None,
                },
            ) {
                println!("TXID: {}", signature);
            }

            if self.is_stopped() {
                break;
            }

            thread::sleep(Duration::from_millis(self.delay_ms));
        }
        Ok(())
    }
} 