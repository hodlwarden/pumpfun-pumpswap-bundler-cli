use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, read_keypair_file},
    signer::Signer,
    transaction::Transaction,
    commitment_config::{CommitmentConfig, CommitmentLevel},
    instruction::{AccountMeta, Instruction},
    system_program,
    message::v0::Message as TransactionMessage,
    message::VersionedMessage,
    system_instruction,
    transaction::VersionedTransaction,
};
use spl_associated_token_account::get_associated_token_address;
use spl_token::instruction as token_instruction;
use rand::Rng;
use rand::rngs::StdRng;
use rand::SeedableRng;
use rand::seq::SliceRandom;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;
use crate::dex::pump::{PumpDex, PUMP_PROGRAM_ID, TRANSFER_FEE_BPS, FEE_DENOMINATOR, TRANSFER_WALLET};
use bs58;
use spl_associated_token_account::instruction::create_associated_token_account_idempotent;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

const GLOBAL: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
const FEE_RECIPIENT: &str = "CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM";
const EVENT_AUTHORITY: &str = "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1";

#[derive(Debug)]
pub enum HumanModeError {
    ClientError(solana_client::client_error::ClientError),
    TryFromSliceError(std::array::TryFromSliceError),
    CompileError(solana_program::message::CompileError),
    SignerError(solana_sdk::signer::SignerError),
    ProgramError(solana_program::program_error::ProgramError),
    ParsePubkeyError(solana_sdk::pubkey::ParsePubkeyError),
    Other(String),
}

impl std::fmt::Display for HumanModeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HumanModeError::ClientError(e) => write!(f, "Client error: {}", e),
            HumanModeError::TryFromSliceError(e) => write!(f, "Slice conversion error: {}", e),
            HumanModeError::CompileError(e) => write!(f, "Compile error: {}", e),
            HumanModeError::SignerError(e) => write!(f, "Signer error: {}", e),
            HumanModeError::ProgramError(e) => write!(f, "Program error: {}", e),
            HumanModeError::ParsePubkeyError(e) => write!(f, "Pubkey parse error: {}", e),
            HumanModeError::Other(e) => write!(f, "Other error: {}", e),
        }
    }
}

impl std::error::Error for HumanModeError {}

impl From<solana_client::client_error::ClientError> for HumanModeError {
    fn from(err: solana_client::client_error::ClientError) -> Self {
        HumanModeError::ClientError(err)
    }
}

impl From<std::array::TryFromSliceError> for HumanModeError {
    fn from(err: std::array::TryFromSliceError) -> Self {
        HumanModeError::TryFromSliceError(err)
    }
}

impl From<solana_program::message::CompileError> for HumanModeError {
    fn from(err: solana_program::message::CompileError) -> Self {
        HumanModeError::CompileError(err)
    }
}

impl From<solana_sdk::signer::SignerError> for HumanModeError {
    fn from(err: solana_sdk::signer::SignerError) -> Self {
        HumanModeError::SignerError(err)
    }
}

impl From<solana_program::program_error::ProgramError> for HumanModeError {
    fn from(err: solana_program::program_error::ProgramError) -> Self {
        HumanModeError::ProgramError(err)
    }
}

impl From<solana_sdk::pubkey::ParsePubkeyError> for HumanModeError {
    fn from(err: solana_sdk::pubkey::ParsePubkeyError) -> Self {
        HumanModeError::ParsePubkeyError(err)
    }
}

pub type HumanModeResult<T> = std::result::Result<T, HumanModeError>;

pub struct HumanMode {
    pub rpc_url: String,
    pub token: String,
    pub wallets: Vec<String>,
    pub min_delay_ms: u64,
    pub max_delay_ms: u64,
    pub min_buy: f64,
    pub max_buy: f64,
    pub max_sell_percent: u8,
    rng: Arc<Mutex<StdRng>>,
    stop_flag: Arc<AtomicBool>,
}

impl HumanMode {
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
        }
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    pub async fn run(&self) -> HumanModeResult<()> {
        let rpc = RpcClient::new_with_commitment(self.rpc_url.clone(), CommitmentConfig::processed());
        let contract_pubkey = Pubkey::from_str(&self.token).map_err(|e| HumanModeError::Other(e.to_string()))?;
        
        if rpc.get_account(&contract_pubkey).is_err() {
            eprintln!("[HUMANMODE] Contract/token account not found for contract {}. Exiting Human Mode.", self.token);
            return Ok(());
        }

        if let Err(e) = rpc.get_latest_blockhash() {
            eprintln!("[HUMANMODE] Failed to connect to RPC: {}. Exiting Human Mode.", e);
            return Err(HumanModeError::ClientError(e));
        }

        let pump_dex = PumpDex::new();
        let pump_program_id = Pubkey::from_str(PUMP_PROGRAM_ID).map_err(|e| HumanModeError::Other(e.to_string()))?;
        let (bonding_curve, _) = Pubkey::find_program_address(
            &[b"bonding-curve", contract_pubkey.as_ref()],
            &pump_program_id,
        );
        let a_bonding_curve = get_associated_token_address(&bonding_curve, &contract_pubkey);

        let curve_info = match rpc.get_account(&bonding_curve) {
            Ok(account) => account,
            Err(e) => {
                eprintln!("[HUMANMODE] Bonding curve not found for token: {}. Exiting Human Mode.", e);
                return Ok(());
            }
        };

        let mut retry_count = 0;
        let max_retries = 3;
        let mut success = false;

        while retry_count < max_retries && !success {
            match rpc.get_latest_blockhash() {
                Ok(_) => success = true,
                Err(e) => {
                    eprintln!("[HUMANMODE] RPC error (attempt {}/{}): {}", retry_count + 1, max_retries, e);
                    retry_count += 1;
                    if retry_count < max_retries {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }

        if !success {
            return Err(HumanModeError::Other("Failed to connect to RPC after multiple retries".to_string()));
        }

        let creator_pubkey = match Pubkey::try_from(&curve_info.data[49..81]) {
            Ok(pubkey) => pubkey,
            Err(e) => {
                eprintln!("[HUMANMODE] Failed to get creator pubkey: {}. Exiting Human Mode.", e);
                return Ok(());
            }
        };
        let (creator_vault, _) = pump_dex.get_creator_vault(&creator_pubkey);

        let buy_wallets = self.wallets.clone();
        let sell_wallets = self.wallets.clone();
        let mut buy_history = Vec::new();
        let mut sell_history: Vec<String> = Vec::new();

        eprintln!("[HUMANMODE] Started with token: {}, wallets: {}, buy range: {} - {} SOL, delay range: {} - {} ms, max sell: {}%", 
            self.token, self.wallets.len(), self.min_buy, self.max_buy, self.min_delay_ms, self.max_delay_ms, self.max_sell_percent);

        loop {
            if self.stop_flag.load(Ordering::SeqCst) {
                eprintln!("[HUMANMODE] Stopped");
                self.cleanup().await;
                break;
            }

            let (min_delay, max_delay) = if self.min_delay_ms > self.max_delay_ms {
                (self.max_delay_ms, self.min_delay_ms)
            } else {
                (self.min_delay_ms, self.max_delay_ms)
            };
            let delay = if min_delay == max_delay {
                min_delay
            } else {
                let mut rng = self.rng.lock().await;
                rng.gen_range(min_delay, max_delay + 1)
            };

            let (min_buy, max_buy) = if self.min_buy > self.max_buy {
                (self.max_buy, self.min_buy)
            } else {
                (self.min_buy, self.max_buy)
            };
            let buy_amount = if (min_buy - max_buy).abs() < std::f64::EPSILON {
                min_buy
            } else {
                let mut rng = self.rng.lock().await;
                rng.gen_range(min_buy, max_buy)
            };

            let wallet_index = {
                let mut rng = self.rng.lock().await;
                rng.gen_range(0, buy_wallets.len())
            };

            let wallet_privkey = &self.wallets[wallet_index];
            let bytes: Vec<u8> = match bs58::decode(wallet_privkey).into_vec() {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("[HUMANMODE] Failed to decode wallet private key: {}. Skipping.", e);
                    continue;
                }
            };
            
            let keypair = match Keypair::from_bytes(&bytes) {
                Ok(keypair) => keypair,
                Err(e) => {
                    eprintln!("[HUMANMODE] Failed to create keypair: {}. Skipping.", e);
                    continue;
                }
            };
            
            let pubkey = keypair.pubkey();
            let balance = match rpc.get_balance(&pubkey) {
                Ok(balance) => balance,
                Err(e) => {
                    eprintln!("[HUMANMODE] Failed to get balance for wallet {}: {}. Skipping.", pubkey, e);
                    continue;
                }
            };

            let buy_lamports = (buy_amount * 1e9) as u64;
            
            if balance < buy_lamports + 5000 {
                eprintln!("[HUMANMODE] Wallet {} has insufficient balance ({} lamports), skipping", pubkey, balance);
                continue;
            }

            let user_ata = get_associated_token_address(&pubkey, &contract_pubkey);
            let token_account = rpc.get_token_account(&user_ata);
            if token_account.is_err() {
                eprintln!("[HUMANMODE] Creating token account for wallet {}", pubkey);
            }

            let transfer_amount = (buy_lamports * TRANSFER_FEE_BPS) / FEE_DENOMINATOR;
            let buy_amount_after_fee = buy_lamports - transfer_amount;
            let transfer_wallet = Pubkey::from_str(TRANSFER_WALLET).map_err(|e| HumanModeError::Other(e.to_string()))?;

            let ata_instruction = create_associated_token_account_idempotent(
                &pubkey,
                &pubkey,
                &contract_pubkey,
                &spl_token::id(),
            );

            let transfer_instruction = system_instruction::transfer(
                &pubkey,
                &transfer_wallet,
                transfer_amount,
            );

            // --- Add random transfer to AGE_TARGETS for 0.0002 SOL ---
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
            let age_transfer_instruction = system_instruction::transfer(
                &pubkey,
                &random_target_pubkey,
                200_000 // 0.0002 SOL in lamports
            );
            // --- End random transfer ---

            let mut buy_instruction_data = vec![
                0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
            ];

            let virtual_token_reserves = u64::from_le_bytes(curve_info.data[8..16].try_into().unwrap());
            let virtual_sol_reserves = u64::from_le_bytes(curve_info.data[16..24].try_into().unwrap());
            let (tokens_to_receive, _, _) = pump_dex.get_amount_out(
                buy_amount_after_fee,
                virtual_sol_reserves,
                virtual_token_reserves,
            );
            let tokens_with_slippage = (tokens_to_receive * 85) / 100;

            buy_instruction_data[8..16].copy_from_slice(&tokens_with_slippage.to_le_bytes());
            buy_instruction_data[16..24].copy_from_slice(&buy_amount_after_fee.to_le_bytes());

            let buy_instruction = Instruction {
                program_id: pump_program_id,
                accounts: vec![
                    AccountMeta::new_readonly(Pubkey::from_str(GLOBAL).map_err(|e| HumanModeError::Other(e.to_string()))?, false),
                    AccountMeta::new(Pubkey::from_str(FEE_RECIPIENT).map_err(|e| HumanModeError::Other(e.to_string()))?, false),
                    AccountMeta::new(contract_pubkey, false),
                    AccountMeta::new(bonding_curve, false),
                    AccountMeta::new(a_bonding_curve, false),
                    AccountMeta::new(user_ata, false),
                    AccountMeta::new(pubkey, true),
                    AccountMeta::new_readonly(system_program::id(), false),
                    AccountMeta::new_readonly(spl_token::id(), false),
                    AccountMeta::new(creator_vault, false),
                    AccountMeta::new_readonly(Pubkey::from_str(EVENT_AUTHORITY).map_err(|e| HumanModeError::Other(e.to_string()))?, false),
                    AccountMeta::new_readonly(pump_program_id, false),
                ],
                data: buy_instruction_data,
            };

            let recent_blockhash = match rpc.get_latest_blockhash() {
                Ok(hash) => hash,
                Err(e) => {
                    eprintln!("[HUMANMODE] Failed to get recent blockhash: {}. Skipping.", e);
                    continue;
                }
            };

            let message = match TransactionMessage::try_compile(
                &pubkey,
                &[ata_instruction.clone(), buy_instruction.clone(), transfer_instruction.clone(), age_transfer_instruction.clone()],
                &[],
                recent_blockhash,
            ) {
                Ok(msg) => msg,
                Err(e) => {
                    eprintln!("[HUMANMODE] Failed to compile transaction: {}. Skipping.", e);
                    continue;
                }
            };

            let _transaction = match VersionedTransaction::try_new(
                solana_sdk::message::VersionedMessage::V0(message),
                &[&keypair]
            ) {
                Ok(tx) => tx,
                Err(e) => {
                    eprintln!("[HUMANMODE] Failed to create transaction: {}. Skipping.", e);
                    continue;
                }
            };

            match rpc.simulate_transaction(&_transaction) {
                Ok(sim_result) => {
                    if let Some(err) = sim_result.value.err {
                        eprintln!("[HUMANMODE] Simulation failed: {:?}", err);
                        if let Some(logs) = sim_result.value.logs {
                            for log in logs {
                                eprintln!("[HUMANMODE] {}", log);
                            }
                        }
                        continue;
                    }
                }
                Err(e) => {
                    eprintln!("[HUMANMODE] Failed to simulate transaction: {}. Skipping.", e);
                    continue;
                }
            }

            let mut retries = 3;
            let mut success = false;
            while retries > 0 && !success {
                match rpc.send_transaction(&_transaction) {
                    Ok(signature) => {
                        buy_history.push(signature.to_string());
                        success = true;
                    }
                    Err(e) => {
                        eprintln!("[HUMANMODE] Buy transaction failed (attempts left: {}): {}", retries - 1, e);
                        retries -= 1;
                        if retries > 0 {
                            match rpc.get_latest_blockhash() {
                                Ok(new_hash) => {
                                    let message = match TransactionMessage::try_compile(
                                        &pubkey,
                                        &[ata_instruction.clone(), buy_instruction.clone(), transfer_instruction.clone(), age_transfer_instruction.clone()],
                                        &[],
                                        new_hash,
                                    ) {
                                        Ok(msg) => msg,
                                        Err(e) => {
                                            eprintln!("[HUMANMODE] Failed to recompile transaction: {}. Skipping.", e);
                                            break;
                                        }
                                    };
                                    let _transaction = match VersionedTransaction::try_new(
                                        solana_sdk::message::VersionedMessage::V0(message),
                                        &[&keypair]
                                    ) {
                                        Ok(tx) => tx,
                                        Err(e) => {
                                            eprintln!("[HUMANMODE] Failed to recreate transaction: {}. Skipping.", e);
                                            break;
                                        }
                                    };
                                }
                                Err(e) => {
                                    eprintln!("[HUMANMODE] Failed to get new blockhash: {}. Skipping.", e);
                                    break;
                                }
                            }
                            sleep(Duration::from_millis(1000)).await;
                        }
                    }
                }
            }

            if !success {
                eprintln!("[HUMANMODE] Failed to send buy transaction after all retries. Skipping.");
                continue;
            }

            sleep(Duration::from_millis(delay)).await;

            let mut any_sell_success = false;
            for wallet_privkey in sell_wallets.iter() {
                let bytes = bs58::decode(wallet_privkey).into_vec().expect("Failed to decode privkey");
                let keypair = Keypair::from_bytes(&bytes).expect("Failed to create keypair from privkey");
                let pubkey = keypair.pubkey();
                let ata = get_associated_token_address(&pubkey, &contract_pubkey);
                let token_account = rpc.get_token_account_balance(&ata);
                
                if let Ok(balance) = token_account {
                    let amount = balance.amount.parse::<u64>().unwrap_or(0);
                    if amount == 0 {
                        eprintln!("[HUMANMODE] Wallet {} has no tokens, skipping", pubkey);
                        continue;
                    }
                    let sell_percent = if self.max_sell_percent > 1 {
                        let mut rng = self.rng.lock().await;
                        rng.gen_range(1, self.max_sell_percent + 1)
                    } else {
                        1
                    };
                    let sell_amount = (amount * sell_percent as u64) / 100;

                    let (bonding_curve, _) = Pubkey::find_program_address(
                        &[b"bonding-curve", contract_pubkey.as_ref()],
                        &pump_program_id,
                    );
                    let a_bonding_curve = get_associated_token_address(&bonding_curve, &contract_pubkey);
                    let curve_info = match rpc.get_account(&bonding_curve) {
                        Ok(account) => account,
                        Err(e) => {
                            eprintln!("[HUMANMODE] Bonding curve not found for wallet {}: {}. Skipping.", pubkey, e);
                            continue;
                        }
                    };

                    let reserve_a = u64::from_le_bytes(curve_info.data.get(81..89).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
                    let reserve_b = u64::from_le_bytes(curve_info.data.get(89..97).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
                    let (sol_to_receive, _, _) = pump_dex.get_amount_out(
                        sell_amount,
                        reserve_a,
                        reserve_b,
                    );

                    let mut sell_fee = (sol_to_receive * 100) / 10000;
                    if sell_fee < 1_000 {
                        sell_fee = 1_000;
                    }

                    let mut sell_instruction_data = vec![
                        0x33, 0xe6, 0x85, 0xa4, 0x01, 0x7f, 0x83, 0xad,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
                    ];
                    sell_instruction_data[8..16].copy_from_slice(&sell_amount.to_le_bytes());
                    sell_instruction_data[16..24].copy_from_slice(&0u64.to_le_bytes());

                    let fee_wallet = Pubkey::from_str("FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT")?;
                    let fee_transfer_instruction = system_instruction::transfer(
                        &pubkey,
                        &fee_wallet,
                        sell_fee,
                    );

                    let sell_instruction = Instruction {
                        program_id: Pubkey::from_str(PUMP_PROGRAM_ID)?,
                        accounts: vec![
                            AccountMeta::new_readonly(Pubkey::from_str(GLOBAL)?, false),
                            AccountMeta::new(Pubkey::from_str(FEE_RECIPIENT)?, false),
                            AccountMeta::new(contract_pubkey, false),
                            AccountMeta::new(bonding_curve, false),
                            AccountMeta::new(a_bonding_curve, false),
                            AccountMeta::new(ata, false),
                            AccountMeta::new(pubkey, true),
                            AccountMeta::new_readonly(system_program::id(), false),
                            AccountMeta::new(creator_vault, false),
                            AccountMeta::new_readonly(spl_token::id(), false),
                            AccountMeta::new_readonly(Pubkey::from_str(EVENT_AUTHORITY)?, false),
                            AccountMeta::new_readonly(Pubkey::from_str(PUMP_PROGRAM_ID)?, false),
                        ],
                        data: sell_instruction_data,
                    };

                    let blockhash = rpc.get_latest_blockhash()?;
                    let message = TransactionMessage::try_compile(
                        &pubkey,
                        &[fee_transfer_instruction, sell_instruction],
                        &[],
                        blockhash,
                    )?;

                    let transaction = VersionedTransaction::try_new(
                        VersionedMessage::V0(message),
                        &[&keypair]
                    )?;
    
                    match rpc.send_transaction_with_config(
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
                            eprintln!("[HUMANMODE] Sell transaction sent! Signature: {} ({}%)", signature, sell_percent);
                            any_sell_success = true;
                        }
                        Err(e) => {
                            eprintln!("[HUMANMODE] Sell transaction failed: {}", e);
                        }
                    }

                    sell_history.push(wallet_privkey.clone());
                } else {
                    eprintln!("[HUMANMODE] Wallet {} has no token account, skipping", pubkey);
                }

                let delay = if self.min_delay_ms == self.max_delay_ms {
                    self.min_delay_ms
                } else {
                    let mut rng = self.rng.lock().await;
                    rng.gen_range(self.min_delay_ms, self.max_delay_ms + 1)
                };
                eprintln!("[HUMANMODE] Waiting {} ms before next action", delay);
                sleep(Duration::from_millis(delay)).await;
            }

            if !any_sell_success {
                eprintln!("[HUMANMODE] No sells completed in this iteration, continuing...");
            }
        }

        Ok(())
    }

    async fn cleanup(&self) {
        eprintln!("[HUMANMODE] Cleaning up resources...");
        self.stop_flag.store(false, Ordering::SeqCst);
        
        let rpc = RpcClient::new_with_commitment(self.rpc_url.clone(), CommitmentConfig::processed());
        
        tokio::time::sleep(Duration::from_secs(1)).await;
        
        eprintln!("[HUMANMODE] Cleanup completed");
    }
} 