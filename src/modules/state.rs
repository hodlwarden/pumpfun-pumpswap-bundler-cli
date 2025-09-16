use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::sync::Mutex;
use chrono::Utc;
use anyhow::Error as AnyhowError;
use solana_sdk::signature::Keypair;
use bs58;
use crate::AppError;

#[derive(Debug, Clone, PartialEq)]
pub enum UserState {
    MainMenu,
    WaitingForTokenAddress,
    WaitingForSpamTokenAddress,
    WaitingForFundAmount,
    WaitingForWalletIndex,
    WaitingForWithdrawAmount,
    WaitingForMinAmount,
    WaitingForMaxAmount,
    WaitingForMinBuyAmount,
    WaitingForMaxBuyAmount,
    WaitingForMinDelay,
    WaitingForMaxDelay,
    WaitingForMaxSellPercent,
    WaitingForDevDumpTokenAddress,
    WaitingForDevDumpAmount,
    WaitingForJitoTipAmount,
    WaitingForBundleBuyAmount,
    WaitingForBundleBuyWallets,
    WaitingForBundleBuyJitoTip,
    WaitingForWalletImport,
    WaitingForWalletCount,
    WaitingForHumanTokenAddress,
    WaitingForBumpTokenAddress,
    WaitingForBumpAmount,
    WaitingForBumpDelay,
    WaitingForBuyAmount,
    WaitingForSpamAmount,
    WaitingForDelay,
    WaitingForMinFundAmount,
    WaitingForMaxFundAmount,
    WaitingForMakerHolders,
    WaitingForMakerJitoTip,
    WaitingForMakerTokenMint,
    WaitingForAffiliateWallet,
    WaitingForAffiliateAddress,
    Idle,
}

impl Default for UserState {
    fn default() -> Self {
        UserState::Idle
    }
}

#[derive(Debug, Clone)]
pub struct UserCommandState {
    pub state: UserState,
    pub token_address: Option<String>,
    pub buy_amount: Option<f64>,
    pub delay: Option<u64>,
    pub min_amount: Option<f64>,
    pub max_amount: Option<f64>,
    pub min_buy: Option<f64>,
    pub max_buy: Option<f64>,
    pub min_delay: Option<u64>,
    pub max_delay: Option<u64>,
    pub max_sell_percent: Option<f64>,
    pub fund_amount: Option<f64>,
    pub wallet_index: Option<usize>,
    pub withdraw_amount: Option<f64>,
    pub dump_amount: Option<u8>,
    pub jito_tip: Option<f64>,
    pub bundle_buy_amount: Option<f64>,
    pub bundle_buy_wallets: Option<usize>,
    pub bundle_buy_jito_tip: Option<f64>,
    pub funder_privkey: Option<String>,
    pub min_fund_amount: Option<f64>,
    pub max_fund_amount: Option<f64>,
    pub wallet_count: Option<u32>,
    pub last_updated: String,
}

impl Default for UserCommandState {
    fn default() -> Self {
        Self {
            state: UserState::MainMenu,
            token_address: None,
            buy_amount: None,
            delay: None,
            min_amount: None,
            max_amount: None,
            min_buy: None,
            max_buy: None,
            min_delay: None,
            max_delay: None,
            max_sell_percent: None,
            fund_amount: None,
            wallet_index: None,
            withdraw_amount: None,
            dump_amount: None,
            jito_tip: None,
            bundle_buy_amount: None,
            bundle_buy_wallets: None,
            bundle_buy_jito_tip: None,
            funder_privkey: None,
            min_fund_amount: None,
            max_fund_amount: None,
            wallet_count: None,
            last_updated: Utc::now().to_rfc3339(),
        }
    }
}

pub struct StateManager {
    state: Mutex<UserCommandState>,
}

impl StateManager {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(UserCommandState::default()),
        }
    }

    pub fn get_state(&self) -> Result<UserCommandState, AppError> {
        let state = self.state.lock().unwrap();
        Ok(state.clone())
    }

    pub fn set_state(&self, new_state: UserState) {
        let mut state = self.state.lock().unwrap();
        state.state = new_state;
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_token_address(&self, token_address: String) {
        let mut state = self.state.lock().unwrap();
        state.token_address = Some(token_address);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_buy_amount(&self, amount: f64) {
        let mut state = self.state.lock().unwrap();
        state.buy_amount = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_delay(&self, delay: u64) {
        let mut state = self.state.lock().unwrap();
        state.delay = Some(delay);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_fund_amount(&self, amount: f64) {
        let mut state = self.state.lock().unwrap();
        state.fund_amount = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_wallet_index(&self, index: usize) {
        let mut state = self.state.lock().unwrap();
        state.wallet_index = Some(index);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_min_fund_amount(&self, amount: f64) {
        let mut state = self.state.lock().unwrap();
        state.min_fund_amount = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_max_fund_amount(&self, amount: f64) {
        let mut state = self.state.lock().unwrap();
        state.max_fund_amount = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_withdraw_amount(&self, amount: f64) {
        let mut state = self.state.lock().unwrap();
        state.withdraw_amount = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_min_buy(&self, amount: f64) {
        let mut state = self.state.lock().unwrap();
        state.min_buy = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_max_buy(&self, amount: f64) {
        let mut state = self.state.lock().unwrap();
        state.max_buy = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_min_delay(&self, delay: u64) {
        let mut state = self.state.lock().unwrap();
        state.min_delay = Some(delay);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_max_delay(&self, delay: u64) {
        let mut state = self.state.lock().unwrap();
        state.max_delay = Some(delay);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_max_sell_percent(&self, percent: f64) {
        let mut state = self.state.lock().unwrap();
        state.max_sell_percent = Some(percent);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_wallet_count(&self, count: u32) {
        let mut state = self.state.lock().unwrap();
        state.wallet_count = Some(count);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_min_amount(&self, amount: f64) {
        let mut state = self.state.lock().unwrap();
        state.min_amount = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_max_amount(&self, amount: f64) {
        let mut state = self.state.lock().unwrap();
        state.max_amount = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_funder_keypair(&self, privkey: String) {
        let mut state = self.state.lock().unwrap();
        state.funder_privkey = Some(privkey);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_dump_amount(&self, amount: u8) {
        let mut state = self.state.lock().unwrap();
        state.dump_amount = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn get_dump_amount(&self) -> Result<u8, AppError> {
        let state = self.state.lock().unwrap();
        state.dump_amount.ok_or_else(|| AppError::AnyhowError(AnyhowError::msg("Dump amount not set")))
    }

    pub fn update_jito_tip(&self, tip: f64) {
        let mut state = self.state.lock().unwrap();
        state.jito_tip = Some(tip);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn get_jito_tip(&self) -> Result<f64, AppError> {
        let state = self.state.lock().unwrap();
        state.jito_tip.ok_or_else(|| AppError::AnyhowError(AnyhowError::msg("Jito tip not set")))
    }

    pub fn reset_state(&self) {
        let mut state = self.state.lock().unwrap();
        *state = UserCommandState::default();
    }

    pub fn get_token_address(&self) -> Result<String, AppError> {
        let state = self.state.lock().unwrap();
        state.token_address.clone().ok_or_else(|| AppError::AnyhowError(AnyhowError::msg("Token address not set")))
    }

    pub fn get_buy_amount(&self) -> Result<f64, AppError> {
        let state = self.state.lock().unwrap();
        state.buy_amount.ok_or_else(|| AppError::AnyhowError(AnyhowError::msg("Buy amount not set")))
    }

    pub fn get_delay(&self) -> Result<u64, AppError> {
        let state = self.state.lock().unwrap();
        state.delay.ok_or_else(|| AppError::AnyhowError(AnyhowError::msg("Delay not set")))
    }

    pub fn get_funder_keypair(&self) -> Result<Keypair, AppError> {
        let state = self.state.lock().unwrap();
        let privkey = state.funder_privkey.clone().ok_or_else(|| AppError::AnyhowError(AnyhowError::msg("Funder keypair not set")))?;
        let bytes = bs58::decode(&privkey).into_vec().map_err(|e| AppError::AnyhowError(AnyhowError::msg(format!("Failed to decode keypair: {}", e))))?;
        Keypair::from_bytes(&bytes).map_err(|e| AppError::AnyhowError(AnyhowError::msg(format!("Failed to create keypair: {}", e))))
    }

    pub fn update_bundle_buy_amount(&self, amount: f64) {
        let mut state = self.state.lock().unwrap();
        state.bundle_buy_amount = Some(amount);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_bundle_buy_wallets(&self, count: usize) {
        let mut state = self.state.lock().unwrap();
        state.bundle_buy_wallets = Some(count);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn update_bundle_buy_jito_tip(&self, tip: f64) {
        let mut state = self.state.lock().unwrap();
        state.bundle_buy_jito_tip = Some(tip);
        state.last_updated = Utc::now().to_rfc3339();
    }

    pub fn get_user_state(&self) -> Result<UserCommandState, AppError> {
        let state = self.state.lock().unwrap();
        Ok(state.clone())
    }
} 