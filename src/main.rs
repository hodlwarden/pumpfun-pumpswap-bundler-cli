#[macro_use]
extern crate lazy_static;

use solana_program::program_error::ProgramError;
use std::error::Error;
use dotenv::dotenv;
use std::env;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::collections::HashMap;
use anyhow::anyhow;
use solana_sdk::pubkey::Pubkey;
use std::fs::File;
use std::io::{self, Write};
use std::str::FromStr;
use tokio::task::JoinHandle;
use rand::seq::SliceRandom;
use serde_json::Value;
use std::sync::OnceLock;
use std::path::Path;
use solana_client::rpc_client::RpcClient;
use solana_sdk::signature::Keypair;
use bs58;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    message::Message,
    signer::Signer,
    system_program,
    system_instruction,
    transaction::VersionedTransaction,
};
use rand::rngs::StdRng;
use rand::SeedableRng;
use solana_sdk::signature::read_keypair_file;
use crate::modules::maker::MakerError;
use crate::modules::spam_create::SpamCreate;
use crate::dex::pump::PumpDex;
use crate::modules::sellSPL::SellSPL;
use crate::modules::staggerBundle::Bundler as StaggerBundler;
use solana_sdk::message::{VersionedMessage};
use solana_sdk::message::v0::Message as V0Message;
use rand::Rng;
use iocraft::prelude::*;

mod dex;
mod modules;

use modules::{
    spam::Spam,
    bumper::Bumper,
    wallet_gen::{WalletGenerator, WalletConfig},
    cleanup::Cleanup,
    human_mode::HumanMode,
    state::StateManager,
    devDump::DevDump,
    bundleBuy::BundleBuy,
    maker::MakerBot,
    walletManager::WalletManager,
    snipeBundle::Bundler,
    spoofer::buy_token,
};

use modules::devSell::dev_sell_token;
use modules::warmUp::WarmUp;
use crate::modules::mixer;

// Color constants
const GREEN: &str = "\x1b[32m";
const BRIGHT_GREEN: &str = "\x1b[92m";
const CYAN: &str = "\x1b[36m";
const BRIGHT_CYAN: &str = "\x1b[96m";
const YELLOW: &str = "\x1b[33m";
const BRIGHT_YELLOW: &str = "\x1b[93m";
const MAGENTA: &str = "\x1b[35m";
const BRIGHT_MAGENTA: &str = "\x1b[95m";
const RED: &str = "\x1b[31m";
const BRIGHT_RED: &str = "\x1b[91m";
const BLUE: &str = "\x1b[34m";
const BRIGHT_BLUE: &str = "\x1b[94m";
const PURPLE: &str = "\x1b[38;5;129m";
const ORANGE: &str = "\x1b[38;5;208m";
const PINK: &str = "\x1b[38;5;199m";
const TEAL: &str = "\x1b[38;5;45m";
const LIME: &str = "\x1b[38;5;154m";
const BLACK: &str = "\x1b[30m";
const TIFFANY: &str = "\x1b[38;2;10;186;181m";
const BLACK_BG: &str = "\x1b[40m";
const RESET: &str = "\x1b[0m";
const CUSTOM_PURPLE: &str = "\x1b[38;2;255;0;155m";
const GRAY: &str = "\x1b[38;2;128;128;128m";
const JITO_UUID: &str = "751f7390-2f50-11f0-858a-6bee29fce9c1";

#[derive(Debug)]
enum AppError {
    ClientError(solana_client::client_error::ClientError),
    TryFromSliceError(std::array::TryFromSliceError),
    CompileError(solana_program::message::CompileError),
    SignerError(solana_sdk::signature::SignerError),
    ProgramError(ProgramError),
    IoError(std::io::Error),
    BumperError(crate::modules::bumper::BumperError),
    SpamError(crate::modules::spam::SpamError),
    CleanupError(crate::modules::cleanup::CleanupError),
    DevDumpError(crate::modules::devDump::DevDumpError),
    AnyhowError(anyhow::Error),
    BoxedError(Box<dyn std::error::Error + Send + Sync>),
    Bs58Error(bs58::decode::Error),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::ClientError(e) => write!(f, "{}Client error: {}{}", RED, e, RESET),
            AppError::TryFromSliceError(e) => write!(f, "{}Slice conversion error: {}{}", RED, e, RESET),
            AppError::CompileError(e) => write!(f, "{}Compile error: {}{}", RED, e, RESET),
            AppError::SignerError(e) => write!(f, "{}Signer error: {}{}", RED, e, RESET),
            AppError::ProgramError(e) => write!(f, "{}Program error: {}{}", RED, e, RESET),
            AppError::IoError(e) => write!(f, "{}IO error: {}{}", RED, e, RESET),
            AppError::BumperError(e) => write!(f, "{}Bumper error: {}{}", RED, e, RESET),
            AppError::SpamError(e) => write!(f, "{}Spam error: {}{}", RED, e, RESET),
            AppError::CleanupError(e) => write!(f, "{}Cleanup error: {}{}", RED, e, RESET),
            AppError::DevDumpError(e) => write!(f, "{}DevDump error: {}{}", RED, e, RESET),
            AppError::AnyhowError(e) => write!(f, "{}Error: {}{}", RED, e, RESET),
            AppError::BoxedError(e) => write!(f, "{}Error: {}{}", RED, e, RESET),
            AppError::Bs58Error(e) => write!(f, "{}Base58 decode error: {}{}", RED, e, RESET),
        }
    }
}

impl std::error::Error for AppError {}

impl From<solana_client::client_error::ClientError> for AppError {
    fn from(err: solana_client::client_error::ClientError) -> Self {
        AppError::ClientError(err)
    }
}

impl From<std::array::TryFromSliceError> for AppError {
    fn from(err: std::array::TryFromSliceError) -> Self {
        AppError::TryFromSliceError(err)
    }
}

impl From<solana_program::message::CompileError> for AppError {
    fn from(err: solana_program::message::CompileError) -> Self {
        AppError::CompileError(err)
    }
}

impl From<solana_sdk::signature::SignerError> for AppError {
    fn from(err: solana_sdk::signature::SignerError) -> Self {
        AppError::SignerError(err)
    }
}

impl From<ProgramError> for AppError {
    fn from(err: ProgramError) -> Self {
        AppError::ProgramError(err)
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::IoError(err)
    }
}

impl From<crate::modules::bumper::BumperError> for AppError {
    fn from(err: crate::modules::bumper::BumperError) -> Self {
        AppError::BumperError(err)
    }
}

impl From<crate::modules::spam::SpamError> for AppError {
    fn from(err: crate::modules::spam::SpamError) -> Self {
        AppError::SpamError(err)
    }
}

impl From<crate::modules::cleanup::CleanupError> for AppError {
    fn from(err: crate::modules::cleanup::CleanupError) -> Self {
        AppError::CleanupError(err)
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::AnyhowError(err)
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for AppError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        AppError::BoxedError(err)
    }
}

impl From<crate::modules::devDump::DevDumpError> for AppError {
    fn from(err: crate::modules::devDump::DevDumpError) -> Self {
        AppError::DevDumpError(err)
    }
}

impl From<MakerError> for AppError {
    fn from(err: MakerError) -> Self {
        AppError::BoxedError(Box::new(err))
    }
}

impl From<crate::modules::bundleBuy::BundleBuyError> for AppError {
    fn from(err: crate::modules::bundleBuy::BundleBuyError) -> Self {
        AppError::BoxedError(Box::new(err))
    }
}

impl From<bs58::decode::Error> for AppError {
    fn from(err: bs58::decode::Error) -> Self {
        AppError::Bs58Error(err)
    }
}

unsafe impl Send for AppError {}
unsafe impl Sync for AppError {}

const MIN_BUMP_AMOUNT_SOL: f64 = 0.01;

type Result<T> = std::result::Result<T, AppError>;

fn clear_screen() {
    print!("\x1B[2J\x1B[1;1H");
}

fn print_banner() {
    element! {
        View(
            border_style: BorderStyle::Round,
            border_color: Color::Grey,
            padding: 2,
        ) {
            Text(
                color: Color::Grey,
                content: ""
            )
            Text(
                color: Color::Grey,
                content: ""
            )
        }
    }
    .print();
}

fn prompt_input(prompt: &str) -> Result<String> {
    print!("\x1b[36m{} --> \x1b[0m", prompt);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    Ok(input.trim().to_string())
}

fn load_config() -> Result<WalletConfig> {
    let generator = WalletGenerator::new();
    generator.load_wallets().map_err(|e| AppError::AnyhowError(anyhow!("Failed to load wallets: {}", e)))
}

async fn handle_wallet_generation() -> Result<()> {
    clear_screen();
    println!("\n{}{}=== Generate Wallets ==={}", BLACK_BG, RED, RESET);
    
    loop {
        print!("{}Enter number of wallets to generate (1-20) --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
        let input = input.trim();
        
        if input.is_empty() {
            println!("{}❌ No input provided. Press Enter to try again...{}", RED, RESET);
            io::stdout().flush().unwrap();
            io::stdin().read_line(&mut String::new()).unwrap();
            continue;
        }
        
        match input.parse::<usize>() {
            Ok(count) if count > 0 && count <= 20 => {
    let generator = WalletGenerator::new();
    generator.generate_wallets(count)?;
    println!("\n{}Successfully generated {} wallets. Press Enter to continue...{}", GREEN, count, RESET);
    io::stdout().flush().unwrap();
    io::stdin().read_line(&mut String::new()).unwrap();
    let config = generator.load_wallets().map_err(|e| AppError::AnyhowError(anyhow!("Failed to load wallets: {}", e)))?;
    
                println!("\n{}Generated Wallets:{}", GREEN, RESET);
    for (i, wallet) in config.wallets.iter().enumerate() {
                    println!("{}{}. Public Key: {}{}", BRIGHT_GREEN, i + 1, wallet.pubkey, RESET);
                    println!("   {}Private Key: {}{}", BRIGHT_BLUE, wallet.privkey, RESET);
                }
                break;
            }
            Ok(_) => {
                println!("{}❌ Please enter a number between 1 and 20. Press Enter to try again...{}", RED, RESET);
                io::stdout().flush().unwrap();
                io::stdin().read_line(&mut String::new()).unwrap();
            }
            Err(_) => {
                println!("{}❌ Invalid input. Please enter a valid number. Press Enter to try again...{}", RED, RESET);
                io::stdout().flush().unwrap();
                io::stdin().read_line(&mut String::new()).unwrap();
            }
        }
    }
    
    Ok(())
}

async fn handle_token_spam() -> Result<()> {
    clear_screen();
    println!("\n{}{}=== Spam Token ==={}", BLACK_BG, BLUE, RESET);
    
    let token = prompt_input("Enter token address")?;
    let buy_amount: f64 = prompt_input("Enter buy amount in SOL")?.parse().map_err(|e| AppError::AnyhowError(anyhow!("{}Invalid amount: {}{}", RED, e, RESET)))?;
    let delay: u64 = prompt_input("Enter delay in milliseconds")?.parse().map_err(|e| AppError::AnyhowError(anyhow!("{}Invalid delay: {}{}", RED, e, RESET)))?;
    
    let config = load_config()?;
    if config.wallets.is_empty() {
        return Err(AppError::AnyhowError(anyhow!("{}No wallets found in wallets.json{}", RED, RESET)));
    }

    let wallets: Vec<String> = config.wallets.iter().map(|w| w.privkey.clone()).collect();
    
    let spam = Spam::new(
        get_rpc_url(),
        token,
        wallets,
        delay, 
        delay, 
        buy_amount, 
        buy_amount, 
        100, 
    );

    clear_screen();
    println!("{}Starting spam bot...{}", GREEN, RESET);
    println!("{}Transaction IDs:{}", BRIGHT_CYAN, RESET);
    spam.run()?;
    Ok(())
}

async fn handle_token_bump() -> Result<()> {
    clear_screen();
    println!("\n{}{}=== Bump Token ==={}", BLACK_BG, YELLOW, RESET);
    
    let token = prompt_input("Enter token address")?;
    let buy_amount: f64 = prompt_input("Enter buy amount in SOL (min 0.01)")?.parse().map_err(|e| AppError::AnyhowError(anyhow!("{}Invalid amount: {}{}", RED, e, RESET)))?;
    let delay: u64 = prompt_input("Enter delay in milliseconds")?.parse().map_err(|e| AppError::AnyhowError(anyhow!("{}Invalid delay: {}{}", RED, e, RESET)))?;
    
    let config = load_config()?;
    if config.wallets.is_empty() {
        return Err(AppError::AnyhowError(anyhow!("{}No wallets found in wallets.json{}", RED, RESET)));
    }

    let wallet = &config.wallets[0];
    let bytes = bs58::decode(&wallet.privkey).into_vec().map_err(|e| AppError::AnyhowError(anyhow!("{}Invalid private key: {}{}", RED, e, RESET)))?;
    let keypair = solana_sdk::signer::keypair::Keypair::from_bytes(&bytes).map_err(|e| AppError::AnyhowError(anyhow!("{}Failed to create keypair: {}{}", RED, e, RESET)))?;
    
    let bumper = Bumper::new(
        get_rpc_url(),
        keypair,
        token,
        buy_amount,
        0.01, 
        delay,
    )?;

    clear_screen();
    println!("{}Starting bump bot...{}", GREEN, RESET);
    println!("{}Transaction IDs:{}", BRIGHT_CYAN, RESET);
    bumper.run()?;

    println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
    io::stdout().flush()?;
    io::stdin().read_line(&mut String::new())?;
    
    Ok(())
}

async fn handle_bundle_buy() -> Result<()> {
    clear_screen();
    println!("\n{}{}=== Bundle Buy ==={}", BLACK_BG, BRIGHT_MAGENTA, RESET);

    print!("{}Enter token address --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut token = String::new();
    io::stdin().read_line(&mut token).unwrap();
    let token = token.trim().to_string();
    
    print!("{}Enter total buy amount in SOL --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut amount = String::new();
    io::stdin().read_line(&mut amount).unwrap();
    let buy_amount: f64 = match amount.trim().parse() {
        Ok(amount) => amount,
        Err(_) => {
            println!("\n{}❌ Invalid amount. Press Enter to return to main menu...{}", RED, RESET);
            io::stdout().flush()?;
            io::stdin().read_line(&mut String::new())?;
            return Ok(());
        }
    };
    
    print!("{}Enter Jito tip amount in SOL --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut tip = String::new();
    io::stdin().read_line(&mut tip).unwrap();
    let jito_tip: f64 = match tip.trim().parse() {
        Ok(tip) => tip,
        Err(_) => {
            println!("\n{}❌ Invalid tip amount. Press Enter to return to main menu...{}", RED, RESET);
            io::stdout().flush()?;
            io::stdin().read_line(&mut String::new())?;
            return Ok(());
        }
    };
    
    let generator = WalletGenerator::new();
    let config = generator.load_wallets().map_err(|e| AppError::AnyhowError(anyhow!("Failed to load wallets: {}", e)))?;
    if config.wallets.is_empty() {
        println!("\n{}❌ No wallets found. Press Enter to return to main menu...{}", RED, RESET);
        io::stdout().flush()?;
        io::stdin().read_line(&mut String::new())?;
        return Ok(());
    }
    
    let mut keypairs = Vec::new();
    for wallet in &config.wallets {
        let bytes = bs58::decode(&wallet.privkey).into_vec()
            .map_err(|e| AppError::AnyhowError(anyhow!("Invalid private key: {}", e)))?;
        let keypair = Keypair::from_bytes(&bytes)
            .map_err(|e| AppError::AnyhowError(anyhow!("Failed to create keypair: {}", e)))?;
        keypairs.push(keypair);
    }
    
    let rpc_client = RpcClient::new(get_rpc_url());
    let dex = PumpDex::new();
    let bundle_buy = BundleBuy::new(get_rpc_url(), token)?;
    
    println!("\n{}Starting bundle buy with {} wallets...{}", GREEN, keypairs.len(), RESET);
    
    match bundle_buy.buy_tokens(keypairs, buy_amount, jito_tip).await {
        Ok(results) => {
    println!("\n{}Bundle Buy Results:{}", BRIGHT_CYAN, RESET);
    for (status, message) in results {
        if status == 0 {
            eprintln!("{}❌ {}{}", RED, message, RESET);
        } else {
            println!("{}✅ {}{}", GREEN, message, RESET);
                }
            }
        }
        Err(e) => {
            eprintln!("\n{}❌ Error: {}{}", RED, e, RESET);
        }
    }

    println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
    io::stdout().flush()?;
    io::stdin().read_line(&mut String::new())?;
    
    clear_screen();
    Ok(())
}

async fn handle_dev_dump() -> Result<()> {
    clear_screen();
    println!("\n{}{}=== Dev Dump ==={}", BLACK_BG, BRIGHT_RED, RESET);
    
    print!("{}Enter token address --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut token = String::new();
    io::stdin().read_line(&mut token).unwrap();
    let token = token.trim().to_string();
    
    print!("{}Enter dump percentage (1-100) --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut percentage = String::new();
    io::stdin().read_line(&mut percentage).unwrap();
    let dump_percentage: u8 = match percentage.trim().parse() {
        Ok(p) => p,
        Err(_) => {
            println!("\n{}❌ Invalid percentage. Press Enter to return to main menu...{}", RED, RESET);
            io::stdout().flush()?;
            io::stdin().read_line(&mut String::new())?;
            return Ok(());
        }
    };
    
    if dump_percentage > 100 {
        println!("\n{}❌ Dump percentage cannot exceed 100. Press Enter to return to main menu...{}", RED, RESET);
        io::stdout().flush()?;
        io::stdin().read_line(&mut String::new())?;
        return Ok(());
    }
    
    print!("{}Enter Jito tip amount in SOL --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut tip = String::new();
    io::stdin().read_line(&mut tip).unwrap();
    let jito_tip: f64 = match tip.trim().parse() {
        Ok(t) => t,
        Err(_) => {
            println!("\n{}❌ Invalid tip amount. Press Enter to return to main menu...{}", RED, RESET);
            io::stdout().flush()?;
            io::stdin().read_line(&mut String::new())?;
            return Ok(());
        }
    };
    
    let generator = WalletGenerator::new();
    let config = match generator.load_wallets() {
        Ok(c) => c,
        Err(e) => {
            println!("\n{}❌ Failed to load wallets: {}. Press Enter to return to main menu...{}", RED, e, RESET);
            io::stdout().flush()?;
            io::stdin().read_line(&mut String::new())?;
            return Ok(());
        }
    };
    
    if config.wallets.is_empty() {
        println!("\n{}❌ No wallets found. Press Enter to return to main menu...{}", RED, RESET);
        io::stdout().flush()?;
        io::stdin().read_line(&mut String::new())?;
        return Ok(());
    }
    
    println!("\nFound {} wallets in wallets.json\n", config.wallets.len());
    println!("{}Starting dev dump with {} wallets...{}", GREEN, config.wallets.len(), RESET);
    
    let mut keypairs = Vec::new();
    for wallet in &config.wallets {
        let bytes = match bs58::decode(&wallet.privkey).into_vec() {
            Ok(b) => b,
            Err(e) => {
                println!("\n{}❌ Invalid private key: {}. Press Enter to return to main menu...{}", RED, e, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
                return Ok(());
            }
        };
        let keypair = match Keypair::from_bytes(&bytes) {
            Ok(k) => k,
            Err(e) => {
                println!("\n{}❌ Failed to create keypair: {}. Press Enter to return to main menu...{}", RED, e, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
                return Ok(());
            }
        };
        keypairs.push(keypair);
    }
    
    let dev_dump = match DevDump::new(get_rpc_url(), Keypair::from_bytes(&keypairs[0].to_bytes()).unwrap(), token, dump_percentage, jito_tip) {
        Ok(d) => d,
        Err(e) => {
            println!("\n{}❌ Failed to create dev dump: {}. Press Enter to return to main menu...{}", RED, e, RESET);
            io::stdout().flush()?;
            io::stdin().read_line(&mut String::new())?;
            return Ok(());
        }
    };
    
    match dev_dump.dump_tokens(keypairs, JITO_UUID, (jito_tip * 1e9) as u64).await {
        Ok(results) => {
    println!("\n{}Dev Dump Results:{}", BRIGHT_CYAN, RESET);
    for (status, message) in results {
        if status == 0 {
            eprintln!("{}❌ {}{}", RED, message, RESET);
        } else {
            println!("{}✅ {}{}", GREEN, message, RESET);
                }
            }
        }
        Err(e) => {
            eprintln!("\n{}❌ Error: {}{}", RED, e, RESET);
            println!("\n{}Press Enter to return to main menu...{}", TIFFANY, RESET);
            io::stdout().flush()?;
            io::stdin().read_line(&mut String::new())?;
            return Ok(());
        }
    }

    println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
    io::stdout().flush()?;
    io::stdin().read_line(&mut String::new())?;
    
    clear_screen();
    Ok(())
}

async fn handle_maker_bot() -> Result<()> {
    loop {
        clear_screen();
        println!("\n{}{}=== Maker Bot ==={}", BLACK_BG, PURPLE, RESET);
        
        println!("{}Select DEX:{}", BRIGHT_CYAN, RESET);
        println!("{}{}1. PumpFun{}", BRIGHT_GREEN, BRIGHT_GREEN, RESET);
        println!("{}{}2. PumpSwap{}", BRIGHT_BLUE, BRIGHT_BLUE, RESET);
        println!("{}{}3. Back to Main Menu{}", BRIGHT_CYAN, BRIGHT_CYAN, RESET);
        print!("\n{}--> {}", TIFFANY, RESET);
        io::stdout().flush()?;
        
        let mut dex_choice = String::new();
        io::stdin().read_line(&mut dex_choice)?;
        
        match dex_choice.trim() {
            "1" | "2" => {
                let use_pumpswap = dex_choice.trim() == "2";
                let use_pumpfun = dex_choice.trim() == "1";
                
                let num_holders: usize = match prompt_input("Enter number of makers")?.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        println!("{}Invalid number. Please try again.{}", RED, RESET);
                        continue;
                    }
                };
                let jito_tip_sol: f64 = match prompt_input("Enter Jito tip amount in SOL")?.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        println!("{}Invalid tip amount. Please try again.{}", RED, RESET);
                        continue;
                    }
                };
                let token_mint = prompt_input("Enter token mint address")?;
                let delay_ms: u64 = match prompt_input("Enter delay between bundles in milliseconds")?.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        println!("{}Invalid delay. Please try again.{}", RED, RESET);
                        continue;
                    }
                };
                
                let mut maker = MakerBot::new(get_rpc_url())?;
                maker.set_dex(use_pumpswap, use_pumpfun);

                clear_screen();
                println!("{}Starting maker bot on {}...{}", GREEN, if use_pumpswap { "PumpSwap" } else { "PumpFun" }, RESET);
                println!("{}Bundle IDs:{}", BRIGHT_CYAN, RESET);
                let results = maker.run_maker(
                    num_holders,
                    jito_tip_sol,
                    &token_mint,
                    delay_ms,
                ).await?;
                
                for (status, message) in results {
                    if status == 0 {
                        eprintln!("{}❌ {}{}", RED, message, RESET);
                    } else {
                        println!("{}✅ {}{}", GREEN, message, RESET);
                    }
                }
                
                println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
            }
            "3" => break,
            _ => {
                io::stdout().flush()?;
            }
        }
    }
    Ok(())
}

async fn handle_human_mode() -> Result<()> {
    clear_screen();
    println!("\n{}{}=== Human Mode ==={}", BLACK_BG, ORANGE, RESET);
    
    print!("{}Enter token address --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut token = String::new();
    io::stdin().read_line(&mut token).unwrap();
    let token = token.trim().to_string();
    
    print!("{}Enter minimum buy amount in SOL --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut min_buy = String::new();
    io::stdin().read_line(&mut min_buy).unwrap();
    let min_buy: f64 = min_buy.trim().parse().map_err(|e| AppError::AnyhowError(anyhow!("Invalid amount: {}", e)))?;
    
    print!("{}Enter maximum buy amount in SOL --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut max_buy = String::new();
    io::stdin().read_line(&mut max_buy).unwrap();
    let max_buy: f64 = max_buy.trim().parse().map_err(|e| AppError::AnyhowError(anyhow!("Invalid amount: {}", e)))?;
    
    print!("{}Enter minimum delay between transactions (ms) --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut min_delay = String::new();
    io::stdin().read_line(&mut min_delay).unwrap();
    let min_delay: u64 = min_delay.trim().parse().map_err(|e| AppError::AnyhowError(anyhow!("Invalid delay: {}", e)))?;
    
    print!("{}Enter maximum delay between transactions (ms) --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut max_delay = String::new();
    io::stdin().read_line(&mut max_delay).unwrap();
    let max_delay: u64 = max_delay.trim().parse().map_err(|e| AppError::AnyhowError(anyhow!("Invalid delay: {}", e)))?;
    
    print!("{}Enter maximum sell percentage (1-100) --> {}", TIFFANY, RESET);
    io::stdout().flush().unwrap();
    let mut max_sell = String::new();
    io::stdin().read_line(&mut max_sell).unwrap();
    let max_sell: u8 = max_sell.trim().parse().map_err(|e| AppError::AnyhowError(anyhow!("Invalid percentage: {}", e)))?;
    
    if max_sell > 100 {
        return Err(AppError::AnyhowError(anyhow!("{}Maximum sell percentage cannot exceed 100{}", RED, RESET)));
    }
    
    let generator = WalletGenerator::new();
    let config = generator.load_wallets().map_err(|e| AppError::AnyhowError(anyhow!("Failed to load wallets: {}", e)))?;
    let wallets: Vec<String> = config.wallets.iter().map(|w| w.privkey.clone()).collect();
    
    if wallets.is_empty() {
        return Err(AppError::AnyhowError(anyhow!("{}No wallets found. Please generate or import wallets first.{}", RED, RESET)));
    }
    
    let rpc_url = std::env::var("RPC").map_err(|_| AppError::AnyhowError(anyhow!("{}RPC not set{}", RED, RESET)))?;
    let human_mode = HumanMode::new(
        rpc_url,
        token,
        wallets,
        min_delay,
        max_delay,
        min_buy,
        max_buy,
        max_sell,
    );
    
    println!("\n{}Starting human mode...{}", GREEN, RESET);
    println!("{}Press Ctrl+C to stop{}", BRIGHT_CYAN, RESET);
    
    if let Err(e) = human_mode.run().await {
        println!("{}Human mode error: {}{}", RED, e, RESET);
    }
    
    Ok(())
}

async fn handle_cleanup() -> Result<()> {
    clear_screen();
    println!("\n{}{}=== Cleanup ==={}", BLACK_BG, PINK, RESET);
    
    let rpc_url = std::env::var("RPC").map_err(|_| AppError::AnyhowError(anyhow!("{}RPC not set{}", RED, RESET)))?;
    let cleanup = Cleanup::new(
        rpc_url,
        "user_data/users.json".to_string(),
    );
    
    println!("\n{}Starting cleanup...{}", GREEN, RESET);
    
    match cleanup.run().await {
        Ok(_) => {
            println!("\n{}Cleanup completed successfully{}", GREEN, RESET);
            println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
            io::stdout().flush()?;
            io::stdin().read_line(&mut String::new())?;
        }
        Err(e) => {
            eprintln!("\n{}Error during cleanup: {}{}", RED, e, RESET);
            println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
            io::stdout().flush()?;
            io::stdin().read_line(&mut String::new())?;
        }
    }
    
    Ok(())
}

async fn handle_wallet_manager() -> Result<()> {
    let rpc_url = get_rpc_url();
    let wallet_manager = WalletManager::new(rpc_url.clone()).await?;
    
    loop {
        clear_screen();
        println!("\n{}{}=== Wallet Manager ==={}", BLACK_BG, GREEN, RESET);
        println!("{}{}1. Check Balances{}", BRIGHT_GREEN, BRIGHT_GREEN, RESET);
        println!("{}{}2. Fund Wallets{}", BRIGHT_BLUE, BRIGHT_BLUE, RESET);
        println!("{}{}3. Withdraw from Wallet{}", BRIGHT_CYAN, BRIGHT_CYAN, RESET);
        println!("{}{}4. Withdraw from All Wallets{}", BRIGHT_YELLOW, BRIGHT_YELLOW, RESET);
        println!("{}{}5. Sell SPL Token{}", BRIGHT_MAGENTA, BRIGHT_MAGENTA, RESET);
        println!("{}{}6. Close LUT{}", PINK, PINK, RESET);
        println!("{}{}7. Dev Fund{}", YELLOW, YELLOW, RESET);
        println!("{}{}8. Return to Main Menu{}", PURPLE, PURPLE, RESET);
        print!("\n{}--> {}", TIFFANY, RESET);
        io::stdout().flush()?;
        
        let mut choice = String::new();
        io::stdin().read_line(&mut choice)?;
        let choice = choice.trim();
        
        match choice {
            "1" => {
                clear_screen();
                match wallet_manager.get_balances_string().await {
                    Ok(balances) => println!("\n{}", balances),
                    Err(e) => println!("\n{}❌ Error checking balances: {}{}", RED, e, RESET),
                }
                print!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
            }
            "2" => {
                clear_screen();
                println!("\n{}{}=== Fund Wallets ==={}", BLACK_BG, GREEN, RESET);
                println!("{}{}1. Fund All Wallets (Same Amount){}", BRIGHT_GREEN, BRIGHT_GREEN, RESET);
                println!("{}{}2. Fund All Wallets (Random Range){}", BRIGHT_BLUE, BRIGHT_BLUE, RESET);
                println!("{}{}3. Fund Individual Wallet{}", BRIGHT_CYAN, BRIGHT_CYAN, RESET);
                println!("{}{}4. Back to Main Menu{}", BRIGHT_YELLOW, BRIGHT_YELLOW, RESET);
                print!("\n{}--> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                
                let mut fund_choice = String::new();
                io::stdin().read_line(&mut fund_choice)?;
                
                match fund_choice.trim() {
                    "1" => {
                        print!("{}Enter amount in SOL --> {}", TIFFANY, RESET);
                        io::stdout().flush()?;
                        let mut amount = String::new();
                        io::stdin().read_line(&mut amount)?;
                        let amount = amount.trim().parse::<f64>()
                            .map_err(|e| AppError::AnyhowError(anyhow!("Invalid amount: {}", e)))?;
                        let amount_lamports = (amount * 1e9) as u64;
                        
                        match wallet_manager.fund_wallets(amount_lamports).await {
                            Ok(result) => {
                                println!("\n{}", result);
                                print!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                                io::stdout().flush()?;
                                io::stdin().read_line(&mut String::new())?;
                            }
                            Err(e) => println!("\n{}❌ Error funding wallets: {}{}", RED, e, RESET),
                        }
                    }
                    "2" => {
                        print!("{}Enter minimum amount in SOL --> {}", TIFFANY, RESET);
                        io::stdout().flush()?;
                        let mut min_amount = String::new();
                        io::stdin().read_line(&mut min_amount)?;
                        let min_amount = min_amount.trim().parse::<f64>()
                            .map_err(|e| AppError::AnyhowError(anyhow!("Invalid amount: {}", e)))?;
                        let min_amount_lamports = (min_amount * 1e9) as u64;
                        
                        print!("{}Enter maximum amount in SOL --> {}", TIFFANY, RESET);
                        io::stdout().flush()?;
                        let mut max_amount = String::new();
                        io::stdin().read_line(&mut max_amount)?;
                        let max_amount = max_amount.trim().parse::<f64>()
                            .map_err(|e| AppError::AnyhowError(anyhow!("Invalid amount: {}", e)))?;
                        let max_amount_lamports = (max_amount * 1e9) as u64;
                        
                        if max_amount_lamports <= min_amount_lamports {
                            println!("\n{}❌ Maximum amount must be greater than minimum amount{}", RED, RESET);
                            continue;
                        }
                        
                        match wallet_manager.fund_wallets_range(min_amount_lamports, max_amount_lamports).await {
                            Ok(result) => {
                                println!("\n{}", result);
                                print!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                                io::stdout().flush()?;
                                io::stdin().read_line(&mut String::new())?;
                            }
                            Err(e) => println!("\n{}❌ Error funding wallets: {}{}", RED, e, RESET),
                        }
                    }
                    "3" => {
                        let balances = wallet_manager.get_balances_string().await?;
                        println!("{}Current Balances:\n{}{}", BRIGHT_CYAN, balances, RESET);
                        
                        let contents = std::fs::read_to_string("wallets/wallets.json")
                            .map_err(|e| AppError::AnyhowError(anyhow!("Failed to read wallets.json: {}", e)))?;
                        
                        let data = serde_json::from_str::<serde_json::Value>(&contents)
                            .map_err(|e| AppError::AnyhowError(anyhow!("Failed to parse wallets.json: {}", e)))?;
                        
                        let wallets = data["wallets"].as_array()
                            .ok_or_else(|| anyhow!("No wallets found in wallets.json"))?;

                        let mut funding_instructions = Vec::new();
                        let mut total_amount = 0u64;
                        let mut funded_wallets = Vec::new();

                        for (i, wallet) in wallets.iter().enumerate() {
                            if let (Some(pubkey), Some(_)) = (wallet["pubkey"].as_str(), wallet["privkey"].as_str()) {
                                let pubkey = Pubkey::from_str(pubkey)
                                    .map_err(|e| AppError::AnyhowError(anyhow!("Invalid pubkey: {}", e)))?;
                                
                                let wallet_balances = wallet_manager.get_balances_string().await?;
                                let balance = wallet_balances.lines()
                                    .find(|line| line.contains(&pubkey.to_string()))
                                    .and_then(|line| {
                                        line.split(": ").nth(1)
                                            .and_then(|s| s.split(" SOL").next())
                                            .and_then(|s| s.parse::<f64>().ok())
                                    })
                                    .unwrap_or(0.0);
                
                                print!("\n{}Wallet {}: {} (Current Balance: {:.6} SOL){}", 
                                    BRIGHT_CYAN, 
                                    i + 1, 
                                    pubkey,
                                    balance,
                                    RESET
                                );
                                print!("\n{}Enter amount to fund (or press Enter to skip) --> {}", TIFFANY, RESET);
                                io::stdout().flush()?;
                                
                                let mut amount = String::new();
                                io::stdin().read_line(&mut amount)?;
                                let amount = amount.trim();
                                
                                if amount.is_empty() {
                                    println!("{}Skipping wallet {}{}", BRIGHT_YELLOW, i + 1, RESET);
                                    continue;
                                }
                                
                                let amount = amount.parse::<f64>()
                                    .map_err(|e| AppError::AnyhowError(anyhow!("Invalid amount: {}", e)))?;
                                let amount_lamports = (amount * 1e9) as u64;
                
                                funding_instructions.push(system_instruction::transfer(
                                    &wallet_manager.get_payer_pubkey(),
                                    &pubkey,
                                    amount_lamports,
                                ));
                                total_amount += amount_lamports;
                                funded_wallets.push((pubkey, amount_lamports));
                            }
                        }

                        if funding_instructions.is_empty() {
                            println!("\n{}No wallets selected for funding.{}", BRIGHT_YELLOW, RESET);
                            continue;
                        }

                        match wallet_manager.fund_wallets_batch_with_instructions(funding_instructions).await {
                            Ok(signature) => {
                                println!("\n{}✅ Successfully funded {} wallets{}", GREEN, funded_wallets.len(), RESET);
                                println!("{}Total sent: {:.6} SOL{}", BRIGHT_CYAN, total_amount as f64 / 1e9, RESET);
                                println!("{}Transaction: {}{}", BRIGHT_BLUE, signature, RESET);
                                println!("\n{}Wallet Details:{}", BRIGHT_CYAN, RESET);
                                for (i, (pubkey, amount)) in funded_wallets.iter().enumerate() {
                                    println!("{}{}. {}: {:.6} SOL{}", BRIGHT_GREEN, i + 1, pubkey, *amount as f64 / 1e9, RESET);
                                }
                                print!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                                io::stdout().flush()?;
                                io::stdin().read_line(&mut String::new())?;
                            }
                            Err(e) => println!("\n{}❌ Error funding wallets: {}{}", RED, e, RESET),
                        }
                    }
                    "4" => continue,
                    _ => (), 
                }
                io::stdout().flush()?;
            }
            "3" => {
                clear_screen();
                println!("\n{}{}=== Withdraw from Wallet ==={}", BLACK_BG, GREEN, RESET);
                let balances = wallet_manager.get_balances_string().await?;
                println!("{}Current Balances:\n{}{}", BRIGHT_CYAN, balances, RESET);
                
                print!("\n{}Enter wallet index --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut index = String::new();
                io::stdin().read_line(&mut index)?;
                let wallet_index = index.trim().parse::<usize>()
                    .map_err(|e| AppError::AnyhowError(anyhow!("Invalid index: {}", e)))? - 1;
                
                print!("{}Enter amount in SOL --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut amount = String::new();
                io::stdin().read_line(&mut amount)?;
                let amount = amount.trim().parse::<f64>()
                    .map_err(|e| AppError::AnyhowError(anyhow!("Invalid amount: {}", e)))?;
                let amount_lamports = (amount * 1e9) as u64;
                
                match wallet_manager.withdraw_from_wallet(wallet_index, amount_lamports).await {
                    Ok(signature) => println!("\n{}✅ Withdrawn successfully!\nTransaction: {}{}", GREEN, signature, RESET),
                    Err(e) => println!("\n{}❌ Error withdrawing: {}{}", RED, e, RESET),
                }
                print!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
            }
            "4" => {
                clear_screen();
                println!("\n{}{}=== Withdraw from All Wallets ==={}", BLACK_BG, GREEN, RESET);
                print!("{}Are you sure you want to withdraw from all wallets? (y/n): {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut confirm = String::new();
                io::stdin().read_line(&mut confirm)?;
                
                if confirm.trim().to_lowercase() == "y" {
                    match wallet_manager.withdraw_from_all().await {
                        Ok(signatures) => {
                            println!("\n{}✅ Withdrawn from all wallets successfully!{}", GREEN, RESET);
                            println!("\n{}Transactions:{}", BRIGHT_CYAN, RESET);
                            for (i, sig) in signatures.iter().enumerate() {
                                println!("{}Wallet {}: {}{}", BRIGHT_GREEN, i + 1, sig, RESET);
                            }
                        }
                        Err(e) => println!("\n{}❌ Error withdrawing from all wallets: {}{}", RED, e, RESET),
                    }
                }
                print!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
            }
            "5" => {
                clear_screen();
                println!("\n{}{}=== Sell SPL Token ==={}", BLACK_BG, GREEN, RESET);
                let dex = crate::dex::pump::PumpDex::new();
                let rpc_client = RpcClient::new(get_rpc_url());
                let sell_spl = SellSPL::new(rpc_client, dex);
                match sell_spl.execute_sell().await {
                    Ok(_) => println!("\n{}✅ SPL token sell operation completed successfully{}", GREEN, RESET),
                    Err(e) => println!("\n{}❌ Error selling SPL token: {}{}", RED, e, RESET),
                }
                print!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
            }
            "6" => {
                clear_screen();
                println!("\n{}{}=== Close LUT ==={}", BLACK_BG, GREEN, RESET);
                print!("{}Enter LUT address --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut lut_address = String::new();
                io::stdin().read_line(&mut lut_address)?;
                let lut_address = lut_address.trim();
                
                println!("{}Closing LUT...{}", BRIGHT_CYAN, RESET);
                match wallet_manager.close_lut(&lut_address).await {
                    Ok(result) => println!("{}Success: {}{}", GREEN, result, RESET),
                    Err(e) => println!("{}Error: {}{}", RED, e, RESET),
                }
            }
            "7" => {
                clear_screen();
                println!("\n{}{}=== Dev Fund ==={}", BLACK_BG, YELLOW, RESET);
                match wallet_manager.dev_fund().await {
                    Ok(msg) => println!("\n{}{}{}\n", GREEN, msg, RESET),
                    Err(e) => println!("\n{}Error: {}{}\n", RED, e, RESET),
                }
                print!("\n{}Press Enter to return to menu...{}", TIFFANY, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
            }
            "8" => {
                println!("\n{}Returning to main menu...{}", BRIGHT_CYAN, RESET);
                break;
            }
            _ => {
                println!("{}Invalid choice. Please try again.{}", RED, RESET);
                continue;
            }
        }
    }
    
    Ok(())
}

async fn show_bundler_menu(rpc_url: String) -> Result<()> {
    loop {
        clear_screen();
        println!("\n{}{}=== Bundler Menu ==={}", BLACK_BG, LIME, RESET);
        println!("{}{}1. Spam Create{}", BRIGHT_GREEN, BRIGHT_GREEN, RESET);
        println!("{}{}2. Create & Bundle{}", BRIGHT_BLUE, BRIGHT_BLUE, RESET);
        println!("{}{}3. Block 1 Snipe{}", BRIGHT_YELLOW, BRIGHT_YELLOW, RESET);
        println!("{}{}4. Manual Bundle{}", BRIGHT_MAGENTA, BRIGHT_MAGENTA, RESET);
        println!("{}{}5. Stagger Bundle{}", BRIGHT_YELLOW, BRIGHT_YELLOW, RESET);
        println!("{}{}6. Back to Main Menu{}", BRIGHT_CYAN, BRIGHT_CYAN, RESET);
        print!("\n{}--> {}", TIFFANY, RESET);
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let choice = input.trim();

        match choice {
            "1" => {
                print!("\n{}Enter delay between creations (in milliseconds) --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                
                let mut delay_input = String::new();
                io::stdin().read_line(&mut delay_input).unwrap();
                
                match delay_input.trim().parse::<u64>() {
                    Ok(delay) => {
                        let spam_create = SpamCreate::new(rpc_url.clone())?;
                        spam_create.run_spam_create(delay).await?;
                    }
                    Err(_) => {
                        println!("\n{}❌ Invalid delay value. Please enter a number.{}", RED, RESET);
                        println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                        io::stdout().flush()?;
                        io::stdin().read_line(&mut String::new()).unwrap();
                    }
                }
            }
            "2" => {
                clear_screen();
                println!("\n{}{}=== Bundler ==={}", BLACK_BG, GREEN, RESET);
                
                print!("{}Enter total buy amount in SOL --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut amount = String::new();
                io::stdin().read_line(&mut amount)?;
                let total_buy_amount: f64 = match amount.trim().parse() {
                    Ok(amount) => amount,
                    Err(_) => {
                        io::stdout().flush()?;
                        continue;
                    }
                };
                
                print!("{}Enter Jito tip amount in SOL --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut tip = String::new();
                io::stdin().read_line(&mut tip)?;
                let jito_tip: f64 = match tip.trim().parse() {
                    Ok(tip) => tip,
                    Err(_) => {
                        println!("\n{}❌ Invalid tip amount. Returning to menu...{}", RED, RESET);
                        println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                        io::stdout().flush()?;
                        io::stdin().read_line(&mut String::new())?;
                        continue;
                    }
                };

                print!("{}Enter dev buy amount in SOL (optional, press enter to skip) --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut dev_buy = String::new();
                io::stdin().read_line(&mut dev_buy)?;
                let dev_buy: Option<f64> = if dev_buy.trim().is_empty() {
                    None
                } else {
                    match dev_buy.trim().parse() {
                        Ok(amount) => Some(amount),
                        Err(_) => {
                            println!("\n{}❌ Invalid dev buy amount. Skipping dev buy...{}", RED, RESET);
                            None
                        }
                    }
                };
                
                let generator = WalletGenerator::new();
                let config = generator.load_wallets()
                    .map_err(|e| AppError::AnyhowError(anyhow!("Failed to load wallets: {}", e)))?;
                
                if config.wallets.is_empty() {
                    return Err(AppError::AnyhowError(anyhow!("{}No wallets found. Please generate or import wallets first.{}", RED, RESET)));
                }
                
                let mut keypairs = Vec::new();
                for wallet in config.wallets {
                    let bytes = bs58::decode(&wallet.privkey).into_vec()
                        .map_err(|e| AppError::AnyhowError(anyhow!("Invalid private key: {}", e)))?;
                    let keypair = solana_sdk::signer::keypair::Keypair::from_bytes(&bytes)
                        .map_err(|e| AppError::AnyhowError(anyhow!("Failed to create keypair: {}", e)))?;
                    keypairs.push(keypair);
                }
                
                let rpc_client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::processed());
                let dex = PumpDex::new();
                
                dotenv().ok();
                let dev_privkey = env::var("DEV").map_err(|_| AppError::AnyhowError(anyhow!("{}DEV not set in .env file{}", RED, RESET)))?;
                let bytes = bs58::decode(&dev_privkey).into_vec()
                    .map_err(|e| AppError::AnyhowError(anyhow!("Failed to decode dev private key: {}", e)))?;
                let payer = Keypair::from_bytes(&bytes)
                    .map_err(|e| AppError::AnyhowError(anyhow!("Failed to create dev keypair: {}", e)))?;
                
                let bundler = Bundler::new(rpc_client, dex, payer);
                
                println!("\n{}Starting bundle buy with {} wallets...{}", GREEN, keypairs.len(), RESET);
                
                let results = bundler.execute_create_and_bundle(
                    total_buy_amount,
                    jito_tip,
                    dev_buy
                ).await
                .map_err(|e| AppError::AnyhowError(anyhow::anyhow!(e)))?;
                
                println!("{}✅ Bundle execution completed successfully{}", GREEN, RESET);

                println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
            }
            "3" => {
                clear_screen();
                println!("\n{}{}=== Block 1 Snipe ==={}", BLACK_BG, YELLOW, RESET);
                
                print!("{}Enter total buy amount in SOL --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut amount = String::new();
                io::stdin().read_line(&mut amount)?;
                let total_buy_amount: f64 = match amount.trim().parse() {
                    Ok(amount) => amount,
                    Err(_) => {
                        io::stdout().flush()?;
                        continue;
                    }
                };
                
                print!("{}Enter Jito tip amount in SOL --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut tip = String::new();
                io::stdin().read_line(&mut tip)?;
                let jito_tip: f64 = match tip.trim().parse() {
                    Ok(tip) => tip,
                    Err(_) => {
                        println!("\n{}❌ Invalid tip amount. Returning to menu...{}", RED, RESET);
                        println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                        io::stdout().flush()?;
                        io::stdin().read_line(&mut String::new())?;
                        continue;
                    }
                };

                print!("{}Enter dev buy amount in SOL (optional, press enter to skip) --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut dev_buy = String::new();
                io::stdin().read_line(&mut dev_buy)?;
                let dev_buy: Option<f64> = if dev_buy.trim().is_empty() {
                    None
                } else {
                    match dev_buy.trim().parse() {
                        Ok(amount) => Some(amount),
                        Err(_) => {
                            println!("\n{}❌ Invalid dev buy amount. Skipping dev buy...{}", RED, RESET);
                            None
                        }
                    }
                };
                
                let generator = WalletGenerator::new();
                let config = generator.load_wallets()
                    .map_err(|e| AppError::AnyhowError(anyhow!("Failed to load wallets: {}", e)))?;
                
                if config.wallets.is_empty() {
                    return Err(AppError::AnyhowError(anyhow!("{}No wallets found. Please generate or import wallets first.{}", RED, RESET)));
                }
                
                let mut keypairs = Vec::new();
                for wallet in config.wallets {
                    let bytes = bs58::decode(&wallet.privkey).into_vec()
                        .map_err(|e| AppError::AnyhowError(anyhow!("Invalid private key: {}", e)))?;
                    let keypair = solana_sdk::signer::keypair::Keypair::from_bytes(&bytes)
                        .map_err(|e| AppError::AnyhowError(anyhow!("Failed to create keypair: {}", e)))?;
                    keypairs.push(keypair);
                }
                
                let rpc_client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::processed());
                let dex = PumpDex::new();
                
                dotenv().ok();
                let dev_privkey = env::var("DEV").map_err(|_| AppError::AnyhowError(anyhow!("{}DEV not set in .env file{}", RED, RESET)))?;
                let bytes = bs58::decode(&dev_privkey).into_vec()
                    .map_err(|e| AppError::AnyhowError(anyhow!("Failed to decode dev private key: {}", e)))?;
                let payer = Keypair::from_bytes(&bytes)
                    .map_err(|e| AppError::AnyhowError(anyhow!("Failed to create dev keypair: {}", e)))?;
                
                let bundler = Bundler::new(rpc_client, dex, payer);
                
                println!("\n{}Starting block 1 snipe with {} wallets...{}", YELLOW, keypairs.len(), RESET);
                
                let results = bundler.execute_create_and_bundle(
                    total_buy_amount,
                    jito_tip,
                    dev_buy
                ).await
                .map_err(|e| AppError::AnyhowError(anyhow::anyhow!(e)))?;
                
                println!("{}✅ Block 1 snipe completed successfully{}", GREEN, RESET);

                println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
            }
            "4" => {
                clear_screen();
                println!("\n{}{}=== Manual Bundle ==={}", BLACK_BG, BRIGHT_MAGENTA, RESET);
                
                print!("{}Enter total buy amount in SOL --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut amount = String::new();
                io::stdin().read_line(&mut amount)?;
                let total_buy_amount: f64 = match amount.trim().parse() {
                    Ok(amount) => amount,
                    Err(_) => {
                        println!("\n{}❌ Invalid amount. Returning to menu...{}", RED, RESET);
                        println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                        io::stdout().flush()?;
                        io::stdin().read_line(&mut String::new())?;
                        continue;
                    }
                };
                
                print!("{}Enter Jito tip amount in SOL --> {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut tip = String::new();
                io::stdin().read_line(&mut tip)?;
                let jito_tip: f64 = match tip.trim().parse() {
                    Ok(tip) => tip,
                    Err(_) => {
                        println!("\n{}❌ Invalid tip amount. Returning to menu...{}", RED, RESET);
                        println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                        io::stdout().flush()?;
                        io::stdin().read_line(&mut String::new())?;
                        continue;
                    }
                };

                let dev_buy_amount = None;

                println!("\n{}Preparing bundle (creating LUT and token)...{}", BRIGHT_MAGENTA, RESET);
                
                let rpc_client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
                let dex = PumpDex::new();
                
                dotenv().ok();
                let dev_privkey = env::var("DEV").map_err(|_| AppError::AnyhowError(anyhow!("{}DEV not set in .env file{}", RED, RESET)))?;
                let bytes = bs58::decode(&dev_privkey).into_vec()
                    .map_err(|e| AppError::AnyhowError(anyhow!("Failed to decode dev private key: {}", e)))?;
                let payer = Keypair::from_bytes(&bytes)
                    .map_err(|e| AppError::AnyhowError(anyhow!("Failed to create dev keypair: {}", e)))?;
                
                let bundler = crate::modules::manualBundle::Bundler::new(rpc_client, dex, payer);
                
                // Phase 1: Prepare bundle
                let bundle_transactions = match bundler.prepare_bundle(total_buy_amount, jito_tip, dev_buy_amount).await {
                    Ok(transactions) => {
                        println!("{}✅ Bundle prepared successfully!{}", GREEN, RESET);
                        transactions
                    }
                    Err(e) => {
                        println!("{}❌ Error preparing bundle: {}{}", RED, e, RESET);
                        println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                        io::stdout().flush()?;
                        io::stdin().read_line(&mut String::new())?;
                        continue;
                    }
                };

                println!("{}Ready to send the bundle. Type 'y' and press Enter to send, or anything else to abort:{}", TIFFANY, RESET);
                let mut confirm = String::new();
                io::stdin().read_line(&mut confirm)?;
                
                if confirm.trim().to_lowercase() == "y" {
                    // Phase 2: Send bundle
                    println!("{}Sending bundle...{}", BRIGHT_MAGENTA, RESET);
                    match bundler.send_bundle(bundle_transactions).await {
                        Ok(_) => {
                            println!("{}✅ Bundle sent successfully!{}", GREEN, RESET);
                        }
                        Err(e) => {
                            println!("{}❌ Error sending bundle: {}{}", RED, e, RESET);
                        }
                    }
                } else {
                    println!("{}Aborted. Bundle was not sent.{}", YELLOW, RESET);
                }

                println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
                io::stdout().flush()?;
                io::stdin().read_line(&mut String::new())?;
            }
            "5" => {
                clear_screen();
                println!("\n{}{}=== Stagger Bundle ==={}", BLACK_BG, BRIGHT_YELLOW, RESET);
                // Prompt for buy amounts and delays
                print!("{}Enter total buy amount (SOL): {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut total_buy_amount = String::new();
                io::stdin().read_line(&mut total_buy_amount)?;
                let total_buy_amount: f64 = match total_buy_amount.trim().parse() {
                    Ok(val) => val,
                    Err(_) => {
                        println!("Invalid amount. Returning to menu...");
                        continue;
                    }
                };

                print!("{}Enter dev buy amount (SOL, or press Enter to skip): {}", TIFFANY, RESET);
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

                print!("{}Enter minimum delay between buys (ms): {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut min_delay = String::new();
                io::stdin().read_line(&mut min_delay)?;
                let min_delay: u64 = match min_delay.trim().parse() {
                    Ok(val) => val,
                    Err(_) => {
                        println!("Invalid min delay. Returning to menu...");
                        continue;
                    }
                };

                print!("{}Enter maximum delay between buys (ms): {}", TIFFANY, RESET);
                io::stdout().flush()?;
                let mut max_delay = String::new();
                io::stdin().read_line(&mut max_delay)?;
                let max_delay: u64 = match max_delay.trim().parse() {
                    Ok(val) => val,
                    Err(_) => {
                        println!("Invalid max delay. Returning to menu...");
                        continue;
                    }
                };

                // Set up the bundler
                let rpc_url = std::env::var("RPC").expect("RPC must be set");
                let payer_key = std::env::var("PAYER").expect("PAYER must be set");
                let payer_bytes = match bs58::decode(&payer_key).into_vec() {
                    Ok(bytes) => bytes,
                    Err(e) => {
                        println!("Failed to decode payer key: {}", e);
                        continue;
                    }
                };
                let payer = match Keypair::from_bytes(&payer_bytes) {
                    Ok(p) => p,
                    Err(e) => {
                        println!("Failed to create payer keypair: {}", e);
                        continue;
                    }
                };
                let rpc_client = RpcClient::new_with_commitment(rpc_url.clone(), CommitmentConfig::confirmed());
                let dex = PumpDex::new();
                let bundler = StaggerBundler::new(rpc_client, dex, payer);

                // Use prepare_bundle for token creation + dev buy
                let (token_instructions, mint_keypair, mint_pubkey, bonding_curve) = match bundler.prepare_bundle(
                    total_buy_amount,
                    0.0,
                    dev_buy_amount,
                ).await {
                    Ok(res) => res,
                    Err(e) => {
                        println!("Failed to prepare bundle: {}", e);
                        continue;
                    }
                };

                // Send the token creation (+ dev buy) transaction
                let recent_blockhash = match bundler.rpc_client.get_latest_blockhash() {
                    Ok(b) => b,
                    Err(e) => {
                        println!("Failed to get latest blockhash: {}", e);
                        continue;
                    }
                };
                let message = match V0Message::try_compile(
                    &bundler.payer.pubkey(),
                    &token_instructions,
                    &[],
                    recent_blockhash,
                ) {
                    Ok(m) => m,
                    Err(e) => {
                        println!("Failed to compile transaction message: {}", e);
                        continue;
                    }
                };
                let tx = match VersionedTransaction::try_new(
                    VersionedMessage::V0(message),
                    &[&bundler.payer, &mint_keypair],
                ) {
                    Ok(t) => t,
                    Err(e) => {
                        println!("Failed to create transaction: {}", e);
                        continue;
                    }
                };
                // Use send_and_confirm_transaction for token creation
                let sig = match bundler.rpc_client.send_and_confirm_transaction(&tx) {
                    Ok(s) => s,
                    Err(e) => {
                        println!("Failed to send and confirm token creation: {}", e);
                        continue;
                    }
                };
                println!("Token created and confirmed: {}", sig);

                // Staggered wallet buys
                let wallets = match bundler.load_wallets() {
                    Ok(w) => w,
                    Err(e) => {
                        println!("Failed to load wallets: {}", e);
                        continue;
                    }
                };
                let num_wallets = wallets.len() as u64;
                if num_wallets == 0 {
                    println!("No wallets found. Exiting.");
                    continue;
                }
                let buy_lamports_per_wallet = ((total_buy_amount * 1_000_000_000.0) as u64) / num_wallets;
                for wallet in wallets {
                    let delay = if max_delay > min_delay {
                        rand::thread_rng().gen_range(min_delay, max_delay + 1)
                    } else {
                        min_delay
                    };
                    println!("Waiting {} ms before next buy...", delay);
                    std::thread::sleep(std::time::Duration::from_millis(delay));
                    match bundler.perform_buy(&mint_pubkey, &bonding_curve, buy_lamports_per_wallet, &wallet) {
                        Ok(sig) => println!("Buy sent for wallet {}: {}", wallet.pubkey(), sig),
                        Err(e) => println!("Buy failed for wallet {}: {}", wallet.pubkey(), e),
                    }
                }
                // Add TIFFANY prompt after all buys
                println!("{0}All transactions sent! Press Enter to return to the menu.{1}", TIFFANY, RESET);
                io::stdout().flush().unwrap();
                io::stdin().read_line(&mut String::new()).unwrap();
            }
            "6" => {
                println!("\n{}Returning to main menu...{}", BRIGHT_CYAN, RESET);
                break;
            }
            _ => {
                println!("{}Invalid choice. Please try again.{}", RED, RESET);
                continue;
            }
        }
    }
    Ok(())
}

async fn handle_spoofer() -> Result<()> {
    clear_screen();
    println!("\n{}{}=== Spoofer ==={}", BLACK_BG, CUSTOM_PURPLE, RESET);
    
    let token = prompt_input("Enter token address")?;
    let buy_amount: f64 = prompt_input("Enter buy amount in SOL")?.parse().map_err(|e| AppError::AnyhowError(anyhow!("Invalid amount: {}", e)))?;
    let spoof_from = prompt_input("Enter address to spoof from")?;
    
    let rpc_url = get_rpc_url();
    let rpc_client = RpcClient::new_with_commitment(
        rpc_url,
        CommitmentConfig::confirmed(),
    );

    let payer_key = std::env::var("PAYER").map_err(|_| AppError::AnyhowError(anyhow!("PAYER must be set")))?;
    let payer_bytes = bs58::decode(&payer_key).into_vec().map_err(|e| AppError::BoxedError(Box::new(e)))?;
    let keypair = Keypair::from_bytes(&payer_bytes).map_err(|e| AppError::BoxedError(Box::new(e)))?;

    let pump_dex = PumpDex::new();

    println!("\n{}Starting spoofer...{}", GREEN, RESET);
    println!("{}Transaction IDs:{}", CUSTOM_PURPLE, RESET);
    
    match buy_token(&pump_dex, &rpc_client, &keypair, &token, buy_amount, &spoof_from).await {
        Ok(_) => {
            println!("\n{}Spoof completed successfully{}", GREEN, RESET);
        }
        Err(e) => {
            eprintln!("\n{}❌ Error during spoof: {}{}", RED, e, RESET);
        }
    }

    println!("\n{}Press Enter to continue...{}", TIFFANY, RESET);
    io::stdout().flush()?;
    io::stdin().read_line(&mut String::new())?;
    
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    clear_screen();
    print_banner();
    println!("{}Starting LUNR...{}", GREEN, RESET);
    
    let rpc_url = get_rpc_url();
    let _wallet_manager = WalletManager::new(rpc_url.clone()).await?;
    
    loop {
        clear_screen();
        print_banner();
        element! {
            View(
                border_style: BorderStyle::Round,
                border_color: Color::Green,
                padding: 2,
                margin: 2,
            ) {
                Text(
                    color: Color::Green,
                    content: "=== Main Menu ===\n\n1. Generate Wallets\n2. Spam Token\n3. Bump Token\n4. Bundle Buy\n5. Dev Dump\n6. Maker Bot\n7. Human Mode\n8. Cleanup\n9. Wallet Manager\n10. Bundler\n11. Spoofer\n12. Dev Sell\n13. WarmUp\n14. Mixer\n0. Exit"
                )
            }
        }.print();
        print!("\n{}--> {}", TIFFANY, RESET);
        io::stdout().flush().unwrap();
        
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        let choice = input.trim();
        
        match choice {
            "1" => handle_wallet_generation().await?,
            "2" => handle_token_spam().await?,
            "3" => handle_token_bump().await?,
            "4" => handle_bundle_buy().await?,
            "5" => handle_dev_dump().await?,
            "6" => handle_maker_bot().await?,
            "7" => handle_human_mode().await?,
            "8" => handle_cleanup().await?,
            "9" => handle_wallet_manager().await?,
            "10" => show_bundler_menu(rpc_url.clone()).await?,
            "11" => handle_spoofer().await?,
            "12" => {
                clear_screen();
                println!("\n{}{}=== Dev Sell ==={}", BLACK_BG, YELLOW, RESET);
                let rpc_url = get_rpc_url();
                let client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());
                let dex = PumpDex::new();
                match dev_sell_token(&client, &dex).await {
                    Ok(_) => {},
                    Err(e) => println!("Error: {}", e),
                }
                print!("\n{}Press Enter to return to menu...{}", TIFFANY, RESET);
                io::stdout().flush().unwrap();
                io::stdin().read_line(&mut String::new()).unwrap();
            }
            "13" => {
                clear_screen();
                element! { Text(color: Color::Cyan, content: "=== WarmUp ===") }.print();
                let rpc_url = get_rpc_url();
                let warmup = WarmUp::new(rpc_url);
                if let Err(e) = warmup.run().await {
                    element! { Text(color: Color::Red, content: format!("Error: {}", e)) }.print();
                }
                element! { Text(color: Color::Cyan, content: "Press Enter to return to menu...") }.print();
                io::stdout().flush().unwrap();
                io::stdin().read_line(&mut String::new()).unwrap();
            }
            "14" => {
                clear_screen();
                println!("\n{}{}=== Mixer ==={}", BLACK_BG, TEAL, RESET);
                if let Err(e) = mixer::main().await {
                    println!("{}Mixer error: {}{}", RED, e, RESET);
                }
                println!("\n{}Press Enter to return to menu...{}", TIFFANY, RESET);
                io::stdout().flush().unwrap();
                io::stdin().read_line(&mut String::new()).unwrap();
            }
            "0" => {
                println!("\n{}Goodbye!{}", GREEN, RESET);
                break;
            }
            _ => {
                continue;
            }
        }
    }
    
    Ok(())
}

fn get_rpc_url() -> String {
    dotenv().ok();
    let rpc_env = std::env::var("RPC").expect(&format!("{}RPC must be set in .env file{}", RED, RESET));
    if let Ok(val) = serde_json::from_str::<Value>(&rpc_env) {
        if let Some(arr) = val.as_array() {
            let rpcs: Vec<String> = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
            if !rpcs.is_empty() {
                let mut rng = rand::thread_rng();
                if let Some(rpc) = rpcs.choose(&mut rng) {
                    return rpc.clone();
                }
            }
        }
    }
    if rpc_env.starts_with("http://") || rpc_env.starts_with("https://") {
        return rpc_env;
    }
    panic!("{}No valid RPC found in .env file. Set RPC as a JSON array or a single URL string.{}", RED, RESET);
}
