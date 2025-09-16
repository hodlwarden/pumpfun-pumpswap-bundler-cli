use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    message::Message,
    pubkey::Pubkey,
    signer::Signer,
    system_program,
    transaction::VersionedTransaction,
    message::v0::Message as TransactionMessage,
    message::VersionedMessage,
};
use std::str::FromStr;
use crate::dex::pump::PumpDex;
use anyhow::{anyhow, Result};
use std::io::{self, Write};
use std::time::Duration;
use tokio::time::sleep;
use std::path::Path;
use std::fs;
use solana_sdk::signature::Keypair;
use spl_associated_token_account::get_associated_token_address;
use bs58;
use spl_token::{
    instruction as token_instruction,
    state::Account as TokenAccount,
};
use spl_associated_token_account::{
    instruction::create_associated_token_account_idempotent,
};
use serde_json;

pub struct SellSPL {
    client: RpcClient,
    dex: PumpDex,
}

impl SellSPL {
    pub fn new(client: RpcClient, dex: PumpDex) -> Self {
        Self {
            client,
            dex,
        }
    }

    pub async fn execute_sell(&self) -> Result<()> {
        print!("Enter token mint address --> ");
        io::stdout().flush()?;
        let mut mint = String::new();
        io::stdin().read_line(&mut mint)?;
        let mint = mint.trim();

        let wallets = self.load_wallets()?;
        if wallets.is_empty() {
            return Err(anyhow!("No wallets found"));
        }

        println!("\nWallets with token balance:");
        let mut wallets_with_balance = Vec::new();
        for (i, wallet) in wallets.iter().enumerate() {
            let token_address = Pubkey::from_str(mint)?;
            let user_ata = get_associated_token_address(
                &wallet.pubkey(),
                &token_address,
            );

            if let Ok(account) = self.client.get_token_account_balance(&user_ata) {
                let balance = account.ui_amount.unwrap_or(0.0);
                if balance > 0.0 {
                    println!("[{}] {} - Balance: {}", i, wallet.pubkey(), balance);
                    wallets_with_balance.push((i, wallet.clone(), balance));
                }
            }
        }

        if wallets_with_balance.is_empty() {
            return Err(anyhow!("No wallets found with token balance"));
        }

        print!("\nSelect wallet index --> ");
        io::stdout().flush()?;
        let mut index = String::new();
        io::stdin().read_line(&mut index)?;
        let wallet_index = index.trim().parse::<usize>()?;

        let selected_wallet = wallets_with_balance
            .iter()
            .find(|(i, _, _)| *i == wallet_index)
            .ok_or_else(|| anyhow!("Invalid wallet index"))?;

        print!("Enter sell percentage (1-100) --> ");
        io::stdout().flush()?;
        let mut percentage = String::new();
        io::stdin().read_line(&mut percentage)?;
        let percentage = percentage.trim().parse::<u8>()?;

        if percentage > 100 {
            return Err(anyhow!("Percentage cannot exceed 100"));
        }

        let token_address = Pubkey::from_str(mint)?;
        let user_ata = get_associated_token_address(
            &selected_wallet.1.pubkey(),
            &token_address,
        );

        let account = self.client.get_token_account_balance(&user_ata)?;
        let balance = account.amount.parse::<u64>()?;
        let sell_amount = (balance as f64 * (percentage as f64 / 100.0)) as u64;

        let (bonding_curve, _) = Pubkey::find_program_address(
            &[b"bonding-curve", token_address.as_ref()],
            &self.dex.program_id,
        );

        let curve_info = self.client.get_account(&bonding_curve)?;
        let creator_pubkey = Pubkey::try_from(&curve_info.data[49..81])?;
        let (creator_vault, _) = Pubkey::find_program_address(
            &[b"creator-vault", creator_pubkey.as_ref()],
            &self.dex.program_id,
        );

        let a_bonding_curve = get_associated_token_address(
            &bonding_curve,
            &token_address,
        );

        let reserve_a = u64::from_le_bytes(curve_info.data.get(81..89).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
        let reserve_b = u64::from_le_bytes(curve_info.data.get(89..97).and_then(|slice| slice.try_into().ok()).unwrap_or([0u8; 8]));
        let (sol_to_receive, _, _) = self.dex.get_amount_out(
            sell_amount,
            reserve_a,
            reserve_b,
        );
        let mut fee_amount = (sol_to_receive * 100) / 10000;
        if fee_amount < 1_000 {
            fee_amount = 1_000;
        }
        let fee_recipient = Pubkey::from_str("FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT")?;

        let sell_instruction = self.dex.create_sell_instruction(
            &token_address,
            &bonding_curve,
            &a_bonding_curve,
            &user_ata,
            &selected_wallet.1.pubkey(),
            &creator_vault,
            sell_amount.into(),
        );

        let fee_instruction = solana_sdk::system_instruction::transfer(
            &selected_wallet.1.pubkey(),
            &fee_recipient,
            fee_amount,
        );

        let blockhash = self.client.get_latest_blockhash()?;
        let message = TransactionMessage::try_compile(
            &selected_wallet.1.pubkey(),
            &[sell_instruction, fee_instruction],
            &[],
            blockhash,
        )?;

        let transaction = VersionedTransaction::try_new(
            VersionedMessage::V0(message),
            &[selected_wallet.1]
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
                        return Err(anyhow!("Transaction failed to confirm"));
                    }
                    
                    match self.client.get_signature_status_with_commitment(&signature, CommitmentConfig::processed()) {
                        Ok(status) => {
                            match status {
                                Some(tx_status) => {
                                    match tx_status {
                                        Ok(()) => return Ok(()),
                                        Err(_) => {
                                            return Err(anyhow!("Transaction failed"));
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
                return Err(anyhow!("Error: {}", e));
            }
        }
    }

    fn load_wallets(&self) -> Result<Vec<Keypair>> {
        let wallet_path = Path::new("wallets/wallets.json");
        if !wallet_path.exists() {
            return Err(anyhow!("wallets.json not found"));
        }

        let contents = fs::read_to_string(wallet_path)?;
        let data: serde_json::Value = serde_json::from_str(&contents)?;

        let wallets = data["wallets"].as_array()
            .ok_or_else(|| anyhow!("No wallets found in file"))?;

        let mut keypairs = Vec::new();
        for wallet in wallets {
            if let (Some(pubkey), Some(privkey)) = (wallet["pubkey"].as_str(), wallet["privkey"].as_str()) {
                let bytes = bs58::decode(privkey).into_vec()?;
                let keypair = Keypair::from_bytes(&bytes)?;
                keypairs.push(keypair);
            }
        }

        if keypairs.is_empty() {
            return Err(anyhow!("No valid wallets found"));
        }

        Ok(keypairs)
    }
} 