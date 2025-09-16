use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::pubkey::Pubkey;
use solana_client::rpc_client::RpcClient;
use solana_sdk::transaction::Transaction;
use solana_sdk::system_instruction;
use solana_sdk::commitment_config::CommitmentConfig;
use std::fs::{File};
use std::io::{Write, Read};
use std::env;
use rand::prelude::*;
use serde::{Serialize, Deserialize};
use std::path::Path;
use std::time::Duration;
use std::thread;

const TRANSACTION_FEE: u64 = 5_000;
const FEE_ADDRESS: &str = "FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT";

#[derive(Serialize, Deserialize, Clone)]
struct WalletInfo {
    pubkey: String,
    #[serde(alias = "privkey", alias = "private_key")]
    private_key: String,
}

fn prompt_input(prompt: &str) -> String {
    print!("\x1b[36m{} --> \x1b[0m", prompt);
    std::io::stdout().flush().unwrap();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).unwrap();
    input.trim().to_string()
}

fn generate_wallets(count: usize) -> Vec<WalletInfo> {
    (0..count)
        .map(|_| {
            let keypair = Keypair::new();
            WalletInfo {
                pubkey: keypair.pubkey().to_string(),
                private_key: keypair.to_base58_string(),
            }
        })
        .collect()
}

fn save_wallets(wallets: &[WalletInfo], filename: &str) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(wallets)?;
    let mut file = File::create(filename)?;
    file.write_all(json.as_bytes())?;
    Ok(())
}

fn load_wallets(filename: &str) -> std::io::Result<Vec<WalletInfo>> {
    let mut file = File::open(filename)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    if let Ok(obj) = serde_json::from_str::<serde_json::Value>(&contents) {
        if let Some(wallets) = obj.get("wallets").and_then(|w| w.as_array()) {
            let wallets: Vec<WalletInfo> = serde_json::from_value(wallets.clone().into())?;
            return Ok(wallets);
        }
    }
    let wallets: Vec<WalletInfo> = serde_json::from_str(&contents)?;
    Ok(wallets)
}

async fn transfer_sol(
    from_keypair: &Keypair, 
    to_pubkey: &Pubkey, 
    amount: u64, 
    rpc_url: &str
) -> Result<String, Box<dyn std::error::Error>> {
    let client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
    let sender_balance = client.get_balance(&from_keypair.pubkey())?;
    if sender_balance < amount + TRANSACTION_FEE {
        return Err(format!(
            "Insufficient balance. Balance: {}, Required: {}",
            sender_balance as f64 / 1_000_000_000.0,
            (amount + TRANSACTION_FEE) as f64 / 1_000_000_000.0
        ).into());
    }
    let recent_blockhash = client.get_latest_blockhash()?;
    let transfer_ix = system_instruction::transfer(
        &from_keypair.pubkey(), 
        to_pubkey, 
        amount
    );
    let transaction = Transaction::new_signed_with_payer(
        &[transfer_ix],
        Some(&from_keypair.pubkey()),
        &[from_keypair],
        recent_blockhash
    );
    let signature = client.send_and_confirm_transaction(&transaction)?;
    Ok(signature.to_string())
}

pub async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let wallet_count: usize = loop {
        let input = prompt_input("How many wallets to use for mixing?");
        match input.parse() {
            Ok(n) if n > 1 => break n,
            _ => println!("Invalid number. Enter an integer > 1."),
        }
    };

    let sol_amount: f64 = loop {
        let input = prompt_input("How much SOL to mix?");
        match input.parse() {
            Ok(val) if val > 0.0 => break val,
            _ => println!("Invalid amount. Enter a positive number."),
        }
    };
    let lamports_to_mix = (sol_amount * 1_000_000_000.0) as u64;

    let mixer_wallets = generate_wallets(wallet_count);
    let mixer_path = "wallets/mixer.json";
    std::fs::create_dir_all("wallets").ok();
    save_wallets(&mixer_wallets, mixer_path)?;
    println!("Generated {} mixer wallets and saved to {}", wallet_count, mixer_path);

    let recipient_wallets = load_wallets("wallets/wallets.json")?;
    if recipient_wallets.is_empty() {
        println!("No recipient wallets found in wallets/wallets.json");
        return Ok(());
    }

    let payer_key = std::env::var("PAYER").expect("PAYER must be set in .env");
    let payer = Keypair::from_base58_string(&payer_key);
    let rpc_url = std::env::var("RPC").expect("RPC must be set in .env");
    let client = RpcClient::new_with_commitment(&rpc_url, CommitmentConfig::confirmed());

    let payer_balance = client.get_balance(&payer.pubkey())?;
    if payer_balance < lamports_to_mix + TRANSACTION_FEE * (wallet_count as u64 + 2) {
        println!("Insufficient balance to mix the requested amount and cover transaction fees");
        return Ok(());
    }
    println!("PAYER balance: {:.6} SOL", payer_balance as f64 / 1e9);

    let mut current_keypair = payer;
    let mut current_pubkey = mixer_wallets[0].pubkey.parse::<Pubkey>()?;
    let mut tx_amount = lamports_to_mix;
    let sig = transfer_sol(&current_keypair, &current_pubkey, tx_amount, &rpc_url).await?;
    println!("{} -> {} ({})", current_keypair.pubkey(), current_pubkey, sig);
    thread::sleep(Duration::from_millis(rand::thread_rng().gen_range(200, 800)));

    current_keypair = Keypair::from_base58_string(&mixer_wallets[0].private_key);
    let next_pubkey = mixer_wallets[1].pubkey.parse::<Pubkey>()?;
    let fee_amount = (tx_amount as f64 * 0.01).round() as u64;
    let after_fee = tx_amount - TRANSACTION_FEE - fee_amount;
    let fee_pubkey = FEE_ADDRESS.parse::<Pubkey>()?;
    let recent_blockhash = client.get_latest_blockhash()?;
    let ix1 = system_instruction::transfer(&current_keypair.pubkey(), &next_pubkey, after_fee);
    let ix2 = system_instruction::transfer(&current_keypair.pubkey(), &fee_pubkey, fee_amount);
    let tx = Transaction::new_signed_with_payer(
        &[ix1, ix2],
        Some(&current_keypair.pubkey()),
        &[&current_keypair],
        recent_blockhash
    );
    let sig = client.send_and_confirm_transaction(&tx)?;
    println!("{} -> {} ({})", current_keypair.pubkey(), next_pubkey, sig);
    thread::sleep(Duration::from_millis(rand::thread_rng().gen_range(200, 800)));
    tx_amount = after_fee;
    current_pubkey = next_pubkey;

    for i in 2..wallet_count {
        let from_keypair = Keypair::from_base58_string(&mixer_wallets[i-1].private_key);
        let to_pubkey = mixer_wallets[i].pubkey.parse::<Pubkey>()?;
        let send_amount = tx_amount - TRANSACTION_FEE;
        let sig = transfer_sol(&from_keypair, &to_pubkey, send_amount, &rpc_url).await?;
        println!("{} -> {} ({})", from_keypair.pubkey(), to_pubkey, sig);
        thread::sleep(Duration::from_millis(rand::thread_rng().gen_range(200, 800)));
        tx_amount = send_amount;
        current_pubkey = to_pubkey;
    }

    let last_keypair = Keypair::from_base58_string(&mixer_wallets[wallet_count-1].private_key);
    let rent_exempt_min = client.get_minimum_balance_for_rent_exemption(0)?;
    let last_balance = client.get_balance(&last_keypair.pubkey())?;
    let recipients = &recipient_wallets;
    let mut available = last_balance;
    let mut funded = 0;
    let mut total_fee = 0u64;
    let mut per_wallet = 0u64;
    for n in (1..=recipients.len()).rev() {
        let fee = TRANSACTION_FEE * n as u64;
        let max_send = available.saturating_sub(rent_exempt_min + fee);
        if max_send >= n as u64 {
            per_wallet = max_send / n as u64;
            if per_wallet > 0 {
                funded = n;
                total_fee = fee;
                break;
            }
        }
    }
    if funded == 0 || per_wallet == 0 {
        return Ok(());
    }
    let mut ixs = Vec::with_capacity(funded);
    for w in recipients.iter().take(funded) {
        let to_pubkey = w.pubkey.parse::<Pubkey>()?;
        ixs.push(system_instruction::transfer(&last_keypair.pubkey(), &to_pubkey, per_wallet));
    }
    let recent_blockhash = client.get_latest_blockhash()?;
    let tx = Transaction::new_signed_with_payer(
        &ixs,
        Some(&last_keypair.pubkey()),
        &[&last_keypair],
        recent_blockhash
    );
    let sig = client.send_and_confirm_transaction(&tx)?;
    println!("{} -> [{} wallets] ({})", last_keypair.pubkey(), funded, sig);
    Ok(())
}