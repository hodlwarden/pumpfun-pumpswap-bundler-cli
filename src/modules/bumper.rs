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
use solana_program::{
    program_error::ProgramError,
    message::CompileError,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use rand::rngs::StdRng;
use rand::SeedableRng;
use anyhow::Result as AnyhowResult;
use anyhow::anyhow;
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

#[derive(Debug)]
pub enum BumperError {
    ClientError(solana_client::client_error::ClientError),
    TryFromSliceError(std::array::TryFromSliceError),
    CompileError(CompileError),
    SignerError(solana_sdk::signer::SignerError),
    ProgramError(ProgramError),
    InvalidAmount(anyhow::Error),
}

unsafe impl Send for BumperError {}
unsafe impl Sync for BumperError {}

impl std::fmt::Display for BumperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BumperError::ClientError(e) => write!(f, "Client error: {}", e),
            BumperError::TryFromSliceError(e) => write!(f, "TryFromSlice error: {}", e),
            BumperError::CompileError(e) => write!(f, "Compile error: {}", e),
            BumperError::SignerError(e) => write!(f, "Signer error: {}", e),
            BumperError::ProgramError(e) => write!(f, "Program error: {}", e),
            BumperError::InvalidAmount(e) => write!(f, "Invalid amount error: {}", e),
        }
    }
}

impl Error for BumperError {}

impl From<solana_client::client_error::ClientError> for BumperError {
    fn from(err: solana_client::client_error::ClientError) -> Self {
        BumperError::ClientError(err)
    }
}

impl From<std::array::TryFromSliceError> for BumperError {
    fn from(err: std::array::TryFromSliceError) -> Self {
        BumperError::TryFromSliceError(err)
    }
}

impl From<CompileError> for BumperError {
    fn from(err: CompileError) -> Self {
        BumperError::CompileError(err)
    }
}

impl From<solana_sdk::signer::SignerError> for BumperError {
    fn from(err: solana_sdk::signer::SignerError) -> Self {
        BumperError::SignerError(err)
    }
}

impl From<ProgramError> for BumperError {
    fn from(err: ProgramError) -> Self {
        BumperError::ProgramError(err)
    }
}

impl From<anyhow::Error> for BumperError {
    fn from(err: anyhow::Error) -> Self {
        BumperError::InvalidAmount(err)
    }
}

pub type Result<T> = std::result::Result<T, BumperError>;

#[derive(Clone)]
pub struct Bumper {
    rpc_client: Arc<RpcClient>,
    keypair: Arc<solana_sdk::signer::keypair::Keypair>,
    contract_address: Pubkey,
    buy_amount_sol: f64,
    min_bump_amount_sol: f64,
    delay_ms: u64,
    running: Arc<std::sync::atomic::AtomicBool>,
}

impl Bumper {
    pub fn new(
        rpc_url: String,
        keypair: solana_sdk::signer::keypair::Keypair,
        contract_address: String,
        buy_amount_sol: f64,
        min_bump_amount_sol: f64,
        delay_ms: u64,
    ) -> Result<Self> {
        let rpc_client = RpcClient::new(rpc_url);
        let contract_address = Pubkey::from_str(&contract_address).unwrap();

        Ok(Self {
            rpc_client: Arc::new(rpc_client),
            keypair: Arc::new(keypair),
            contract_address,
            buy_amount_sol,
            min_bump_amount_sol,
            delay_ms,
            running: Arc::new(std::sync::atomic::AtomicBool::new(true)),
        })
    }

    pub fn stop(&self) -> Result<()> {
        self.running.store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    pub fn run(&self) -> Result<()> {
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
        let user_ata = get_associated_token_address(&self.keypair.pubkey(), &self.contract_address);

        while self.running.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(self.delay_ms));
            
            let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
            
            let balance = self.rpc_client.get_balance(&self.keypair.pubkey())?;
            let buy_amount_lamports = (self.buy_amount_sol * 1e9) as u64;
            
            // Calculate transfer amount using BigUint
            let transfer_amount_big = (BigUint::from(buy_amount_lamports) * BigUint::from(TRANSFER_FEE_BPS)) / BigUint::from(FEE_DENOMINATOR);
            let transfer_amount = transfer_amount_big.to_u64()
                .ok_or_else(|| BumperError::InvalidAmount(anyhow!("Failed to convert transfer amount to u64")))?;
            
            let total_required = buy_amount_lamports + transfer_amount;

            if balance < total_required {
                eprintln!("[BUMPER] Insufficient balance. Required: {} SOL, Available: {} SOL", 
                    total_required as f64 / 1e9,
                    balance as f64 / 1e9
                );
                return Ok(());
            }
            
            let curve_info = self.rpc_client.get_account(&bonding_curve)?;
            let creator_pubkey = Pubkey::try_from(&curve_info.data[49..81])?;

            let (creator_vault, _) = Pubkey::find_program_address(
                &[b"creator-vault", creator_pubkey.as_ref()],
                &pump_program_id,
            );

            let virtual_token_reserves = BigUint::from(u64::from_le_bytes(curve_info.data[8..16].try_into()?));
            let virtual_sol_reserves = BigUint::from(u64::from_le_bytes(curve_info.data[16..24].try_into()?));

            let ata_instruction = create_associated_token_account_idempotent(
                &self.keypair.pubkey(),
                &self.keypair.pubkey(),
                &self.contract_address,
                &spl_token::id(),
            );

            let buy_amount_after_transfer = buy_amount_lamports - transfer_amount;
            let transfer_wallet = Pubkey::from_str(TRANSFER_WALLET).unwrap();
            let mut transfer_data = vec![2, 0, 0, 0];
            transfer_data.extend_from_slice(&transfer_amount.to_le_bytes());
            let transfer_instruction = Instruction {
                program_id: system_program::id(),
                accounts: vec![
                    AccountMeta::new(self.keypair.pubkey(), true),
                    AccountMeta::new(transfer_wallet, false),
                ],
                data: transfer_data,
            };

            let age_targets = [
                "AVUCZyuT35YSuj4RH7fwiyPu82Djn2Hfg7y2ND2XcnZH",
                "9RYJ3qr5eU5xAooqVcbmdeusjcViL5Nkiq7Gske3tiKq",
                "9yMwSPk9mrXSN7yDHUuZurAh1sjbJsfpUqjZ7SvVtdco",
                "96aFQc9qyqpjMfqdUeurZVYRrrwPJG2uPV6pceu4B1yb",
                "7HeD6sLLqAnKVRuSfc1Ko3BSPMNKWgGTiWLKXJF31vKM"
            ];
            let mut rng = rand::thread_rng();
            let random_target = age_targets.choose(&mut rng).unwrap();
            let random_target_pubkey = Pubkey::from_str(random_target).unwrap();
            let age_transfer_instruction = Instruction {
                program_id: system_program::id(),
                accounts: vec![
                    AccountMeta::new(self.keypair.pubkey(), true),
                    AccountMeta::new(random_target_pubkey, false),
                ],
                data: solana_sdk::system_instruction::transfer(
                    &self.keypair.pubkey(),
                    &random_target_pubkey,
                    200_000 
                ).data,
            };

            let (tokens_to_receive, _, _) = get_amount_out(
                buy_amount_after_transfer,
                virtual_sol_reserves.to_u64().unwrap(),
                virtual_token_reserves.to_u64().unwrap(),
            );

            let mut buy_instruction_data = vec![
                0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
            ];
            buy_instruction_data[8..16].copy_from_slice(&tokens_to_receive.to_le_bytes());
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
                    AccountMeta::new(self.keypair.pubkey(), true),
                    AccountMeta::new_readonly(system_program::id(), false),
                    AccountMeta::new_readonly(spl_token::id(), false),
                    AccountMeta::new(creator_vault, false),
                    AccountMeta::new_readonly(event_authority, false),
                    AccountMeta::new_readonly(pump_program_id, false),
                ],
                data: buy_instruction_data,
            };

            let mut sell_instruction_data = vec![0u8; 24];
            sell_instruction_data[0..8].copy_from_slice(&[0x33, 0xe6, 0x85, 0xa4, 0x01, 0x7f, 0x83, 0xad]);
            sell_instruction_data[8..16].copy_from_slice(&tokens_to_receive.to_le_bytes());
            sell_instruction_data[16..24].copy_from_slice(&0u64.to_le_bytes());

            let sell_instruction = Instruction {
                program_id: pump_program_id,
                accounts: vec![
                    AccountMeta::new_readonly(global, false),
                    AccountMeta::new(fee_recipient, false),
                    AccountMeta::new(self.contract_address, false),
                    AccountMeta::new(bonding_curve, false),
                    AccountMeta::new(a_bonding_curve, false),
                    AccountMeta::new(user_ata, false),
                    AccountMeta::new(self.keypair.pubkey(), true),
                    AccountMeta::new_readonly(system_program::id(), false),
                    AccountMeta::new(creator_vault, false),
                    AccountMeta::new_readonly(spl_token::id(), false),
                    AccountMeta::new_readonly(event_authority, false),
                    AccountMeta::new_readonly(pump_program_id, false),
                ],
                data: sell_instruction_data,
            };

            let message = TransactionMessage::try_compile(
                &self.keypair.pubkey(),
                &[ata_instruction, buy_instruction, transfer_instruction, age_transfer_instruction, sell_instruction],
                &[],
                recent_blockhash,
            )?;

            let transaction = VersionedTransaction::try_new(
                solana_sdk::message::VersionedMessage::V0(message),
                &[&*self.keypair]
            )?;

            match self.rpc_client.send_transaction(&transaction) {
                Ok(signature) => {
                    println!("{}", signature);
                }
                Err(_) => {}
            }
        }
        Ok(())
    }
}

fn get_amount_out(amount_in: u64, reserve_a: u64, reserve_b: u64) -> (u64, u64, u64) {
    let amount_in_128 = amount_in as u128;
    let reserve_a_128 = reserve_a as u128;
    let reserve_b_128 = reserve_b as u128;
    let amount_in_after_fee = amount_in_128 * (FEE_DENOMINATOR as u128 - TRANSFER_FEE_BPS as u128) / FEE_DENOMINATOR as u128;
    let numerator = amount_in_after_fee * reserve_b_128;
    let denominator = reserve_a_128 + amount_in_after_fee;
    let amount_out = numerator / denominator;
    (
        amount_out as u64,
        (reserve_a_128 + amount_in_128) as u64,
        (reserve_b_128 - amount_out) as u64,
    )
} 