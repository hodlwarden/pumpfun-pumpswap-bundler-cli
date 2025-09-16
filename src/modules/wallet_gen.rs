use solana_sdk::{
    signature::Keypair,
    signer::Signer,
};
use std::{
    fs,
    path::Path,
};
use serde::{Serialize, Deserialize};
use anyhow::Result;

#[derive(Debug, Serialize, Deserialize)]
pub struct Wallet {
    pub pubkey: String,
    pub privkey: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletConfig {
    pub wallets: Vec<Wallet>,
}

pub struct WalletGenerator {
    config_path: String,
}

impl WalletGenerator {
    pub fn new() -> Self {
        let wallets_dir = "wallets";
        if !Path::new(wallets_dir).exists() {
            fs::create_dir(wallets_dir).expect("Failed to create wallets directory");
        }

        Self {
            config_path: format!("{}/wallets.json", wallets_dir),
        }
    }

    pub fn generate_wallets(&self, count: usize) -> Result<()> {
        if count > 20 {
            return Err(anyhow::anyhow!("Error: Maximum 20 wallets can be generated"));
        }

        let config = WalletConfig { wallets: Vec::new() };
        let json = serde_json::to_string_pretty(&config)?;
        fs::write(&self.config_path, json)?;
        
        let mut config = WalletConfig { wallets: Vec::new() };
        for _ in 0..count {
            let keypair = Keypair::new();
            let pubkey = keypair.pubkey().to_string();
            let privkey = bs58::encode(keypair.to_bytes()).into_string();

            let wallet = Wallet {
                pubkey,
                privkey,
            };

            config.wallets.push(wallet);
        }

        let json = serde_json::to_string_pretty(&config)?;
        fs::write(&self.config_path, json)?;
        println!("Generated {} wallets", count);
        Ok(())
    }

    pub fn load_wallets(&self) -> Result<WalletConfig> {
        if !Path::new(&self.config_path).exists() {
            return Ok(WalletConfig { wallets: Vec::new() });
        }

        let contents = fs::read_to_string(&self.config_path)?;
        let config: WalletConfig = serde_json::from_str(&contents)?;
        Ok(config)
    }

    pub fn get_keypair(&self, pubkey: &str) -> Result<Keypair> {
        let config = self.load_wallets()?;
        let wallet = config.wallets.iter()
            .find(|w| w.pubkey == pubkey)
            .ok_or_else(|| anyhow::anyhow!("Error: Wallet not found"))?;

        let bytes = bs58::decode(&wallet.privkey).into_vec()?;
        Ok(Keypair::from_bytes(&bytes)?)
    }
} 