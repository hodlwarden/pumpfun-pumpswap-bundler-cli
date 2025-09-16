use solana_client::rpc_client::RpcClient;
use solana_program::{
    pubkey::Pubkey,
    instruction::{AccountMeta, Instruction},
    system_program,
};
use solana_sdk::{
    commitment_config::CommitmentConfig, compute_budget::ComputeBudgetInstruction, message::{v0::Message, VersionedMessage}, signature::read_keypair_file, signer::{keypair::Keypair, Signer}, system_instruction::transfer, transaction::{Transaction, VersionedTransaction}
};
use spl_associated_token_account::{
    get_associated_token_address,
    instruction::create_associated_token_account_idempotent,
};
use spl_token::{
    instruction::{close_account, sync_native},
    native_mint::ID as NATIVE_MINT,
};
use std::str::FromStr;
use std::error::Error;
use dotenv::dotenv;
use std::env;
use std::time::Duration;
use tokio::time::sleep;

const PUMP_AMM_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";
const PUMP_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const WSOL_MINT: &str = "So11111111111111111111111111111111111111112";
const GLOBAL_CONFIG: &str = "ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw";
const PROTOCOL_FEE_RECIPIENT: &str = "62qc2CNXwrYqQScmEdiZFFAnJR262PxWEuNQtxfafNgV";
const PROTOCOL_FEE_ACCOUNT: &str = "7xQYoUjUJF1Kg6WVczoTAkaNhn5syQYcbvjmFrhjWpx";
const EVENT_AUTH: &str = "GS4CU59F31iL7aR2Q8zVS8DRrcRnXX1yjQ66TqNVQnaR";
const TOKEN_PROGRAM_ID: Pubkey = spl_token::id();
const SYSTEM_PROGRAM_ID: Pubkey = system_program::id();
const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey = spl_associated_token_account::id();

const COMPUTE_UNIT_LIMIT_POWER_BUMP: u32 = 1_400_000;
const ATA_CREATE_FEE: f64 = 0.00203928;
const LP_FEE_BASIS_POINTS: u64 = 30;
const PROTOCOL_FEE_BASIS_POINTS: u64 = 10;
const COIN_CREATOR_FEE_BASIS_POINTS: u64 = 10;

pub struct PumpSwap {
    pub rpc_client: RpcClient,
    payer: Keypair,
    program_id: Pubkey,
}

impl PumpSwap {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        dotenv().ok();
        let rpc_url = env::var("RPC").expect("RPC not set");
        let private_key = env::var("PAYER").expect("PRIVATE_KEY not set");
        
        let payer = Keypair::from_base58_string(&private_key);
        let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::processed());
        let program_id = Pubkey::from_str(PUMP_AMM_PROGRAM_ID)?;

        Ok(PumpSwap { 
            rpc_client, 
            payer,
            program_id,
        })
    }

    pub fn get_pool_address(&self, token_address: &str) -> Result<Pubkey, Box<dyn Error>> {
        let token_address = Pubkey::from_str(token_address)?;
        let pump_program_id = Pubkey::from_str(PUMP_PROGRAM_ID)?;
        
        let (pump_pool_authority, _) = Pubkey::find_program_address(
            &[b"pool-authority", token_address.as_ref()],
            &pump_program_id
        );
        
        let (pool_address, _) = Pubkey::find_program_address(
            &[
                b"pool",
                &0u16.to_le_bytes()[..2], 
                pump_pool_authority.as_ref(), 
                token_address.as_ref(),
                &NATIVE_MINT.as_ref()
            ],
            &self.program_id
        );

        match self.rpc_client.get_account(&pool_address) {
            Ok(_) => (),
            Err(_) => (),
        }

        Ok(pool_address)
    }

    pub async fn swap(&self, pool_address: &str, base_mint: &str, amount: f64, slippage: u8) -> Result<(u8, u64), Box<dyn Error>> {
        let buy_in_lamports = (amount * 1e9) as u64;
        let denominator = 10000 + LP_FEE_BASIS_POINTS + PROTOCOL_FEE_BASIS_POINTS + COIN_CREATOR_FEE_BASIS_POINTS;
        let actual_swap_amount = ((buy_in_lamports as u128) * 10000) / (denominator as u128);
        let max_sol_cost = buy_in_lamports + (buy_in_lamports * slippage as u64) / 100 + 2;

        let token_address = Pubkey::from_str(base_mint)?;
        let pool = Pubkey::from_str(pool_address)?;
        let user = &self.payer.pubkey();
        let protocol_fee_recipient = Pubkey::from_str(PROTOCOL_FEE_RECIPIENT)?;

        let required_amount = amount + ATA_CREATE_FEE;
        let required_lamports = (required_amount * 1e9) as u64;
        let wallet_balance = self.rpc_client.get_balance(user)?;
        
        if wallet_balance < required_lamports {
            return Ok((10, 0));
        }

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

        let fund_wrapped_sol = transfer(
            user,
            &quote_token_account,
            max_sol_cost
        );

        let wrapped_sol_sync = sync_native(
            &TOKEN_PROGRAM_ID,
            &quote_token_account
        )?;

        let close_wrapped_sol = close_account(
            &TOKEN_PROGRAM_ID,
            &quote_token_account,
            user,
            user,
            &[]
        )?;

        let accounts = self.rpc_client.get_multiple_accounts(&[pool, pool_base_account, pool_quote_account])?;
        
        if accounts[0].is_none() || accounts[1].is_none() || accounts[2].is_none() {
            return Ok((8, 0));
        }

        let pool_data = accounts[0].as_ref().ok_or("Failed to fetch pool data")?;
        let coin_creator = Pubkey::new_from_array(pool_data.data[211..243].try_into()?);
        
        let (vault_authority, _) = Pubkey::find_program_address(
            &[b"creator_vault", coin_creator.as_ref()],
            &self.program_id
        );
        
        let vault_ata = get_associated_token_address(
            &vault_authority,
            &NATIVE_MINT
        );

        let pool_base_reserves = u64::from_le_bytes(accounts[1].as_ref().ok_or("Failed to fetch pool base account")?.data[64..72].try_into()?) as u128;
        let pool_quote_reserves = u64::from_le_bytes(accounts[2].as_ref().ok_or("Failed to fetch pool quote account")?.data[64..72].try_into()?) as u128;
        
        let k = pool_base_reserves * pool_quote_reserves;
        let new_quote_reserves = pool_quote_reserves + actual_swap_amount - 1;
        let new_base_reserves = k / new_quote_reserves;
        let base_amount_out = (pool_base_reserves - new_base_reserves) as u64;

        let mut buy_data = vec![0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea];
        buy_data.extend_from_slice(&base_amount_out.to_le_bytes());
        buy_data.extend_from_slice(&max_sol_cost.to_le_bytes());

        let buy_instruction = Instruction {
            program_id: self.program_id,
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
                AccountMeta::new_readonly(self.program_id, false),
                AccountMeta::new(vault_ata, false),
                AccountMeta::new_readonly(vault_authority, false),
            ],
            data: buy_data,
        };

        instructions.push(wrapped_sol_instruction);
        instructions.push(fund_wrapped_sol);
        instructions.push(wrapped_sol_sync);
        instructions.push(base_token_instruction);
        instructions.push(protocol_fee_token_instruction);
        instructions.push(buy_instruction);
        instructions.push(close_wrapped_sol);

        let blockhash = self.rpc_client.get_latest_blockhash()?;
        let message = Message::try_compile(
            user,
            &instructions,
            &[],
            blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[&self.payer]
        )?;

        match self.rpc_client.send_transaction_with_config(
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
                println!("Buy: {}", signature);
                
                let max_retries = 21;
                let mut retries_count = 0;
                
                loop {
                    if retries_count >= max_retries {
                        return Ok((7, 0));
                    }
                    
                    match self.rpc_client.get_signature_status_with_commitment(&signature, CommitmentConfig::processed()) {
                        Ok(status) => {
                            match status {
                                Some(tx_status) => {
                                    match tx_status {
                                        Ok(()) => return Ok((0, base_amount_out)),
                                        Err(_) => {
                                            return Ok((6, 0));
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
                            sleep(Duration::from_millis(500)).await;
                            continue;
                        }
                    }
                }
            },
            Err(_) => {
                return Ok((6, 0));
            }
        }
    }

    pub async fn sell(&self, pool_address: &str, base_mint: &str, amount: f64, slippage: u8) -> Result<u8, Box<dyn Error>> {
        let base_amount = (amount * 1e9) as u64;
        let token_address = Pubkey::from_str(base_mint)?;
        let pool = Pubkey::from_str(pool_address)?;
        let user = &self.payer.pubkey();
        let protocol_fee_recipient = Pubkey::from_str(PROTOCOL_FEE_RECIPIENT)?;

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

        let accounts = self.rpc_client.get_multiple_accounts(&[pool, pool_base_account, pool_quote_account])?;
        
        if accounts[0].is_none() || accounts[1].is_none() || accounts[2].is_none() {
            return Ok(8);
        }

        let pool_data = accounts[0].as_ref().ok_or("Failed to fetch pool data")?;
        let coin_creator = Pubkey::new_from_array(pool_data.data[211..243].try_into()?);
        
        let (vault_authority, _) = Pubkey::find_program_address(
            &[b"creator_vault", coin_creator.as_ref()],
            &self.program_id
        );
        
        let vault_ata = get_associated_token_address(
            &vault_authority,
            &NATIVE_MINT
        );

        let pool_base_reserves = u64::from_le_bytes(accounts[1].as_ref().ok_or("Failed to fetch pool base account")?.data[64..72].try_into()?) as u128;
        let pool_quote_reserves = u64::from_le_bytes(accounts[2].as_ref().ok_or("Failed to fetch pool quote account")?.data[64..72].try_into()?) as u128;
        
        let k = pool_base_reserves * pool_quote_reserves;
        let new_base_reserves = pool_base_reserves + base_amount as u128;
        let new_quote_reserves = k / new_base_reserves;
        let quote_amount_out = (pool_quote_reserves - new_quote_reserves) as u64;
        
        let min_quote_out = if quote_amount_out < 10 {
            1
        } else {
            quote_amount_out - (quote_amount_out * slippage as u64) / 100
        };

        let mut sell_data = vec![51, 230, 133, 164, 1, 127, 131, 173];
        sell_data.extend_from_slice(&base_amount.to_le_bytes());
        sell_data.extend_from_slice(&min_quote_out.to_le_bytes());

        let sell_instruction = Instruction {
            program_id: self.program_id,
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
                AccountMeta::new_readonly(self.program_id, false),
                AccountMeta::new(vault_ata, false),
                AccountMeta::new_readonly(vault_authority, false),
            ],
            data: sell_data,
        };

        instructions.push(wrapped_sol_instruction);
        instructions.push(base_token_instruction);
        instructions.push(protocol_fee_token_instruction);
        instructions.push(sell_instruction);

        let blockhash = self.rpc_client.get_latest_blockhash()?;
        let message = Message::try_compile(
            user,
            &instructions,
            &[],
            blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[&self.payer]
        )?;

        match self.rpc_client.send_transaction_with_config(
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
                println!("Sell: {}", signature);
                
                let max_retries = 21;
                let mut retries_count = 0;
                
                loop {
                    if retries_count >= max_retries {
                        return Ok(7);
                    }
                    
                    match self.rpc_client.get_signature_status_with_commitment(&signature, CommitmentConfig::processed()) {
                        Ok(status) => {
                            match status {
                                Some(tx_status) => {
                                    match tx_status {
                                        Ok(()) => return Ok(0),
                                        Err(_) => {
                                            return Ok(6);
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
                            sleep(Duration::from_millis(500)).await;
                            continue;
                        }
                    }
                }
            },
            Err(_) => {
                return Ok(6);
            }
        }
    }
} 