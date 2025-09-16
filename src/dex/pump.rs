use solana_program::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    system_program,
};
use spl_associated_token_account::{
    instruction::create_associated_token_account_idempotent,
};
use std::str::FromStr;

pub const PUMP_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
pub const GLOBAL: &str = "4wTV1YmiEkRvAtNtsSGPtUrqRYQMe5SKy2uB4Jjaxnjf";
pub const FEE_RECIPIENT: &str = "CebN5WGQ4jvEPvsVU4EoHEpgzq1VV7AbicfhtW4xC9iM";
pub const EVENT_AUTHORITY: &str = "Ce6TQqeHC9p8KetsN6JsjHK7UTZk7nasjjnr7XxXp9F1";
pub const TRANSFER_WALLET: &str = "FEExX798hpCjB4CGpkbojm3uCrMGSfByhd8drPUNNbxT";
pub const FEE_BPS: u64 = 100; // 1% fee
pub const TRANSFER_FEE_BPS: u64 = 100; // 1% transfer fee
pub const FEE_DENOMINATOR: u64 = 10000;

#[derive(Clone)]
pub struct PumpDex {
    pub program_id: Pubkey,
    pub global: Pubkey,
    pub fee_recipient: Pubkey,
    pub event_authority: Pubkey,
}      

impl PumpDex {
    pub fn new() -> Self {
        Self {
            program_id: Pubkey::from_str(PUMP_PROGRAM_ID).unwrap(),
            global: Pubkey::from_str(GLOBAL).unwrap(),
            fee_recipient: Pubkey::from_str(FEE_RECIPIENT).unwrap(),
            event_authority: Pubkey::from_str(EVENT_AUTHORITY).unwrap(),
        }
    }

    pub fn get_bonding_curve(&self, token_mint: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"bonding-curve", token_mint.as_ref()],
            &self.program_id,
        )
    }

    pub fn get_creator_vault(&self, creator_pubkey: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[b"creator-vault", creator_pubkey.as_ref()],
            &self.program_id,
        )
    }

    pub fn get_amount_out(&self, amount_in: u64, reserve_a: u64, reserve_b: u64) -> (u64, u64, u64) {
        let amount_in_128 = amount_in as u128;
        let reserve_a_128 = reserve_a as u128;
        let reserve_b_128 = reserve_b as u128;
        let amount_in_after_fee = amount_in_128 * (FEE_DENOMINATOR as u128 - FEE_BPS as u128) / FEE_DENOMINATOR as u128;
        let numerator = amount_in_after_fee * reserve_b_128;
        let denominator = reserve_a_128 + amount_in_after_fee;
        let amount_out = numerator / denominator;
        (
            amount_out as u64,
            (reserve_a_128 + amount_in_128) as u64,
            (reserve_b_128 - amount_out) as u64,
        )
    }

    pub fn get_amount_in(&self, desired_out: u64, reserve_a: u64, reserve_b: u64) -> u64 {
        let desired_out_128 = desired_out as u128;
        let reserve_a_128 = reserve_a as u128;
        let reserve_b_128 = reserve_b as u128;
        if desired_out_128 >= reserve_b_128 {
            return u64::MAX; // not possible
        }
        let amount_in_after_fee = (desired_out_128 * reserve_a_128) / (reserve_b_128 - desired_out_128);
        let amount_in = (amount_in_after_fee * FEE_DENOMINATOR as u128 + (FEE_DENOMINATOR as u128 - FEE_BPS as u128 - 1)) / (FEE_DENOMINATOR as u128 - FEE_BPS as u128);
        amount_in as u64
    }

    pub fn create_ata_instruction(
        &self,
        payer: &Pubkey,
        owner: &Pubkey,
        mint: &Pubkey,
    ) -> Instruction {
        create_associated_token_account_idempotent(
            payer,
            owner,
            mint,
            &spl_token::id(),
        )
    }

    pub fn create_transfer_instruction(
        &self,
        from: &Pubkey,
        amount: u64,
    ) -> Instruction {
        let mut transfer_data = vec![2, 0, 0, 0];
        transfer_data.extend_from_slice(&amount.to_le_bytes());
        Instruction {
            program_id: system_program::id(),
            accounts: vec![
                AccountMeta::new(*from, true),
                AccountMeta::new(Pubkey::from_str(TRANSFER_WALLET).unwrap(), false),
            ],
            data: transfer_data,
        }
    }

    pub fn create_buy_instruction(
        &self,
        contract_address: &Pubkey,
        bonding_curve: &Pubkey,
        a_bonding_curve: &Pubkey,
        user_ata: &Pubkey,
        user: &Pubkey,
        creator_vault: &Pubkey,
        tokens_to_receive: u64,
        buy_amount_lamports: u64,
    ) -> Instruction {
        let mut instruction_data = vec![
            0x66, 0x06, 0x3d, 0x12, 0x01, 0xda, 0xeb, 0xea,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
        ];
        instruction_data[8..16].copy_from_slice(&tokens_to_receive.to_le_bytes());
        instruction_data[16..24].copy_from_slice(&buy_amount_lamports.to_le_bytes());

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.global, false),
                AccountMeta::new(self.fee_recipient, false),
                AccountMeta::new(*contract_address, false),
                AccountMeta::new(*bonding_curve, false),
                AccountMeta::new(*a_bonding_curve, false),
                AccountMeta::new(*user_ata, false),
                AccountMeta::new(*user, true),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new(*creator_vault, false),
                AccountMeta::new_readonly(self.event_authority, false),
                AccountMeta::new_readonly(self.program_id, false),
            ],
            data: instruction_data,
        }
    }

    pub fn create_sell_instruction(
        &self,
        contract_address: &Pubkey,
        bonding_curve: &Pubkey,
        a_bonding_curve: &Pubkey,
        user_ata: &Pubkey,
        user: &Pubkey,
        creator_vault: &Pubkey,
        tokens_to_sell: u128,
    ) -> Instruction {
        let mut instruction_data = vec![
            0x33, 0xe6, 0x85, 0xa4, 0x01, 0x7f, 0x83, 0xad,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
        ];
        let tokens_to_sell_u64 = if tokens_to_sell > u64::MAX as u128 {
            u64::MAX
        } else {
            tokens_to_sell as u64
        };
        instruction_data[8..16].copy_from_slice(&tokens_to_sell_u64.to_le_bytes());
        instruction_data[16..24].copy_from_slice(&0u64.to_le_bytes());

        Instruction {
            program_id: self.program_id,
            accounts: vec![
                AccountMeta::new_readonly(self.global, false),
                AccountMeta::new(self.fee_recipient, false),
                AccountMeta::new(*contract_address, false),
                AccountMeta::new(*bonding_curve, false),
                AccountMeta::new(*a_bonding_curve, false),
                AccountMeta::new(*user_ata, false),
                AccountMeta::new(*user, true),
                AccountMeta::new_readonly(system_program::id(), false),
                AccountMeta::new(*creator_vault, false),
                AccountMeta::new_readonly(spl_token::id(), false),
                AccountMeta::new_readonly(self.event_authority, false),
                AccountMeta::new_readonly(self.program_id, false),
            ],
            data: instruction_data,
        }
    }

    pub fn get_program_id(&self) -> Pubkey {
        self.program_id
    }
} 
