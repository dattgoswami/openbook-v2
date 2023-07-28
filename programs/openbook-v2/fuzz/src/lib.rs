pub mod accounts_state;
pub mod processor;

use accounts_state::*;
use anchor_spl::token::spl_token;
use arbitrary::{Arbitrary, Unstructured};
use fixed::types::I80F48;
use openbook_v2::state::*;
use processor::*;
use solana_program::{
    entrypoint::ProgramResult, instruction::AccountMeta, pubkey::Pubkey, system_program,
};
use spl_associated_token_account::get_associated_token_address;
use std::collections::{HashMap, HashSet};

pub const NUM_USERS: u8 = 8;
pub const INITIAL_BALANCE: u64 = 1_000_000_000;

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct UserId(u8);

impl Arbitrary<'_> for UserId {
    fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        let i: u8 = u.arbitrary()?;
        Ok(Self(i % NUM_USERS))
    }

    fn size_hint(_: usize) -> (usize, Option<usize>) {
        (1, Some(1))
    }
}

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct ReferrerId(u8);

impl Arbitrary<'_> for ReferrerId {
    fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
        let i: u8 = u.arbitrary()?;
        Ok(Self(i % NUM_USERS))
    }

    fn size_hint(_: usize) -> (usize, Option<usize>) {
        (1, Some(1))
    }
}

pub struct FuzzContext {
    pub payer: Pubkey,
    pub admin: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub market: Pubkey,
    pub oracle: Pubkey,
    pub bids: Pubkey,
    pub asks: Pubkey,
    pub event_queue: Pubkey,
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub collect_fee_admin: Pubkey,
    pub collect_fee_admin_quote_vault: Pubkey,
    pub users: HashMap<UserId, UserAccounts>,
    pub referrers: HashMap<ReferrerId, Pubkey>,
    pub state: AccountsState,
}

impl FuzzContext {
    pub fn new(market_index: MarketIndex) -> Self {
        let payer = Pubkey::new_unique();
        let admin = Pubkey::new_unique();
        let base_mint = Pubkey::new_unique();
        let quote_mint = Pubkey::new_unique();

        let market = Pubkey::find_program_address(
            &[
                b"Market".as_ref(),
                payer.as_ref(),
                market_index.to_le_bytes().as_ref(),
            ],
            &openbook_v2::ID,
        )
        .0;

        let oracle = Pubkey::find_program_address(
            &[b"StubOracle".as_ref(), admin.as_ref(), base_mint.as_ref()],
            &openbook_v2::ID,
        )
        .0;

        let bids = Pubkey::new_unique();
        let asks = Pubkey::new_unique();
        let event_queue = Pubkey::new_unique();

        let base_vault = get_associated_token_address(&market, &base_mint);
        let quote_vault = get_associated_token_address(&market, &quote_mint);

        let collect_fee_admin = Pubkey::new_unique();
        let collect_fee_admin_quote_vault =
            get_associated_token_address(&collect_fee_admin, &quote_mint);

        Self {
            payer,
            admin,
            base_mint,
            quote_mint,
            market,
            oracle,
            bids,
            asks,
            event_queue,
            base_vault,
            quote_vault,
            collect_fee_admin,
            collect_fee_admin_quote_vault,
            users: HashMap::new(),
            referrers: HashMap::new(),
            state: AccountsState::new(),
        }
    }

    pub fn initialize(&mut self) -> &mut Self {
        self.state
            .add_account_with_lamports(self.admin, INITIAL_BALANCE)
            .add_account_with_lamports(self.collect_fee_admin, 0)
            .add_account_with_lamports(self.payer, INITIAL_BALANCE)
            .add_mint(self.base_mint)
            .add_mint(self.quote_mint)
            .add_openbook_account::<BookSide>(self.asks)
            .add_openbook_account::<BookSide>(self.bids)
            .add_openbook_account::<EventQueue>(self.event_queue)
            .add_openbook_account::<Market>(self.market)
            .add_openbook_account::<StubOracle>(self.oracle)
            .add_program(openbook_v2::ID) // optional accounts use this pubkey
            .add_program(spl_token::ID)
            .add_program(system_program::ID)
            .add_token_account_with_lamports(self.base_vault, self.market, self.base_mint, 0)
            .add_token_account_with_lamports(self.quote_vault, self.market, self.quote_mint, 0)
            .add_token_account_with_lamports(
                self.collect_fee_admin_quote_vault,
                self.collect_fee_admin,
                self.quote_mint,
                0,
            );

        self.stub_oracle_create().unwrap();
        self
    }

    fn get_or_create_new_user(&mut self, user_id: &UserId) -> &UserAccounts {
        let create_new_user = || -> UserAccounts {
            let account_num = 0_u32;

            let owner = Pubkey::new_unique();
            let base_vault = Pubkey::new_unique();
            let quote_vault = Pubkey::new_unique();
            let open_orders = Pubkey::find_program_address(
                &[
                    b"OpenOrders".as_ref(),
                    owner.as_ref(),
                    self.market.as_ref(),
                    &account_num.to_le_bytes(),
                ],
                &openbook_v2::ID,
            )
            .0;

            self.state
                .add_account_with_lamports(owner, INITIAL_BALANCE)
                .add_account_with_lamports(owner, INITIAL_BALANCE)
                .add_token_account_with_lamports(base_vault, owner, self.base_mint, INITIAL_BALANCE)
                .add_token_account_with_lamports(
                    quote_vault,
                    owner,
                    self.quote_mint,
                    INITIAL_BALANCE,
                )
                .add_openbook_account::<OpenOrdersAccount>(open_orders);

            let accounts = openbook_v2::accounts::InitOpenOrders {
                open_orders_account: open_orders,
                owner,
                delegate_account: None,
                payer: self.payer,
                market: self.market,
                system_program: system_program::ID,
            };
            let data = openbook_v2::instruction::InitOpenOrders { account_num };
            process_instruction(&mut self.state, &data, &accounts, &[]).unwrap();

            UserAccounts {
                owner,
                open_orders,
                base_vault,
                quote_vault,
            }
        };

        self.users.entry(*user_id).or_insert_with(create_new_user)
    }

    fn get_or_create_new_referrer(&mut self, referrer_id: &ReferrerId) -> &Pubkey {
        let create_new_referrer = || -> Pubkey {
            let quote_vault = Pubkey::new_unique();

            self.state.add_token_account_with_lamports(
                quote_vault,
                Pubkey::new_unique(),
                self.quote_mint,
                0,
            );

            quote_vault
        };

        self.referrers
            .entry(*referrer_id)
            .or_insert_with(create_new_referrer)
    }

    fn stub_oracle_create(&mut self) -> ProgramResult {
        let accounts = openbook_v2::accounts::StubOracleCreate {
            oracle: self.oracle,
            owner: self.admin,
            mint: self.base_mint,
            payer: self.payer,
            system_program: system_program::ID,
        };
        let data = openbook_v2::instruction::StubOracleCreate { price: I80F48::ONE };
        process_instruction(&mut self.state, &data, &accounts, &[])
    }

    pub fn create_market(&mut self, data: openbook_v2::instruction::CreateMarket) -> ProgramResult {
        let accounts = openbook_v2::accounts::CreateMarket {
            market: self.market,
            bids: self.bids,
            asks: self.asks,
            event_queue: self.event_queue,
            payer: self.payer,
            base_vault: self.base_vault,
            quote_vault: self.quote_vault,
            base_mint: self.base_mint,
            quote_mint: self.quote_mint,
            oracle_a: Some(self.oracle),
            oracle_b: None,
            system_program: system_program::ID,
            collect_fee_admin: self.collect_fee_admin,
            open_orders_admin: None,
            consume_events_admin: None,
            close_market_admin: None,
        };
        process_instruction(&mut self.state, &data, &accounts, &[])
    }

    pub fn deposit(
        &mut self,
        user_id: &UserId,
        data: &openbook_v2::instruction::Deposit,
    ) -> ProgramResult {
        let user = self.get_or_create_new_user(user_id);

        let accounts = openbook_v2::accounts::Deposit {
            owner: user.owner,
            token_base_account: user.base_vault,
            token_quote_account: user.quote_vault,
            open_orders_account: user.open_orders,
            market: self.market,
            base_vault: self.base_vault,
            quote_vault: self.quote_vault,
            token_program: spl_token::ID,
            system_program: system_program::ID,
        };

        process_instruction(&mut self.state, data, &accounts, &[])
    }

    pub fn place_order(
        &mut self,
        user_id: &UserId,
        data: &openbook_v2::instruction::PlaceOrder,
        makers: Option<&HashSet<UserId>>,
    ) -> ProgramResult {
        let market_vault = match data.args.side {
            Side::Ask => self.base_vault,
            Side::Bid => self.quote_vault,
        };

        let user = self.get_or_create_new_user(user_id);
        let token_deposit_account = match data.args.side {
            Side::Ask => user.base_vault,
            Side::Bid => user.quote_vault,
        };

        let accounts = openbook_v2::accounts::PlaceOrder {
            open_orders_account: user.open_orders,
            signer: user.owner,
            token_deposit_account,
            open_orders_admin: None,
            market: self.market,
            bids: self.bids,
            asks: self.asks,
            event_queue: self.event_queue,
            market_vault,
            oracle_a: Some(self.oracle),
            oracle_b: None,
            token_program: spl_token::ID,
            system_program: system_program::ID,
        };

        let remaining = makers.map_or_else(Vec::new, |makers| {
            makers
                .iter()
                .filter(|id| id != &user_id)
                .filter_map(|id| self.users.get(id))
                .map(|user| AccountMeta {
                    pubkey: user.open_orders,
                    is_signer: false,
                    is_writable: true,
                })
                .collect::<Vec<_>>()
        });

        process_instruction(&mut self.state, data, &accounts, &remaining)
    }

    pub fn place_order_pegged(
        &mut self,
        user_id: &UserId,
        data: &openbook_v2::instruction::PlaceOrderPegged,
        makers: Option<&HashSet<UserId>>,
    ) -> ProgramResult {
        let market_vault = match data.args.side {
            Side::Ask => self.base_vault,
            Side::Bid => self.quote_vault,
        };

        let user = self.get_or_create_new_user(user_id);
        let token_deposit_account = match data.args.side {
            Side::Ask => user.base_vault,
            Side::Bid => user.quote_vault,
        };

        let accounts = openbook_v2::accounts::PlaceOrder {
            open_orders_account: user.open_orders,
            signer: user.owner,
            token_deposit_account,
            open_orders_admin: None,
            market: self.market,
            bids: self.bids,
            asks: self.asks,
            event_queue: self.event_queue,
            market_vault,
            oracle_a: Some(self.oracle),
            oracle_b: None,
            token_program: spl_token::ID,
            system_program: system_program::ID,
        };

        let remaining = makers.map_or_else(Vec::new, |makers| {
            makers
                .iter()
                .filter(|id| id != &user_id)
                .filter_map(|id| self.users.get(id))
                .map(|user| AccountMeta {
                    pubkey: user.open_orders,
                    is_signer: false,
                    is_writable: true,
                })
                .collect::<Vec<_>>()
        });

        process_instruction(&mut self.state, data, &accounts, &remaining)
    }

    pub fn place_take_order(
        &mut self,
        user_id: &UserId,
        data: &openbook_v2::instruction::PlaceTakeOrder,
        referrer_id: Option<&ReferrerId>,
        makers: Option<&HashSet<UserId>>,
    ) -> ProgramResult {
        let referrer = referrer_id.map(|id| *self.get_or_create_new_referrer(id));
        let user = self.get_or_create_new_user(user_id);

        let (token_deposit_account, token_receiver_account) = match data.args.side {
            Side::Ask => (user.base_vault, user.quote_vault),
            Side::Bid => (user.quote_vault, user.base_vault),
        };

        let accounts = openbook_v2::accounts::PlaceTakeOrder {
            signer: user.owner,
            market: self.market,
            bids: self.bids,
            asks: self.asks,
            token_deposit_account,
            token_receiver_account,
            base_vault: self.base_vault,
            quote_vault: self.quote_vault,
            event_queue: self.event_queue,
            oracle_a: Some(self.oracle),
            oracle_b: None,
            token_program: spl_token::ID,
            system_program: system_program::ID,
            open_orders_admin: None,
            referrer,
        };

        let remaining = makers.map_or_else(Vec::new, |makers| {
            makers
                .iter()
                .filter(|id| id != &user_id)
                .filter_map(|id| self.users.get(id))
                .map(|user| AccountMeta {
                    pubkey: user.open_orders,
                    is_signer: false,
                    is_writable: true,
                })
                .collect::<Vec<_>>()
        });

        process_instruction(&mut self.state, data, &accounts, &remaining)
    }

    pub fn consume_events(
        &mut self,
        user_ids: &HashSet<UserId>,
        data: &openbook_v2::instruction::ConsumeEvents,
    ) -> ProgramResult {
        let accounts = openbook_v2::accounts::ConsumeEvents {
            consume_events_admin: None,
            market: self.market,
            event_queue: self.event_queue,
        };

        let remaining = user_ids
            .iter()
            .filter_map(|user_id| self.users.get(user_id))
            .map(|user| AccountMeta {
                pubkey: user.open_orders,
                is_signer: false,
                is_writable: true,
            })
            .collect::<Vec<_>>();

        process_instruction(&mut self.state, data, &accounts, &remaining)
    }

    pub fn consume_given_events(
        &mut self,
        user_ids: &HashSet<UserId>,
        data: &openbook_v2::instruction::ConsumeGivenEvents,
    ) -> ProgramResult {
        let accounts = openbook_v2::accounts::ConsumeEvents {
            consume_events_admin: None,
            market: self.market,
            event_queue: self.event_queue,
        };

        let remaining = user_ids
            .iter()
            .filter_map(|user_id| self.users.get(user_id))
            .map(|user| AccountMeta {
                pubkey: user.open_orders,
                is_signer: false,
                is_writable: true,
            })
            .collect::<Vec<_>>();

        process_instruction(&mut self.state, data, &accounts, &remaining)
    }

    pub fn cancel_order(
        &mut self,
        user_id: &UserId,
        data: &openbook_v2::instruction::CancelOrder,
    ) -> ProgramResult {
        let Some(user) = self.users.get(user_id) else {
            return Ok(());
        };

        let accounts = openbook_v2::accounts::CancelOrder {
            signer: user.owner,
            open_orders_account: user.open_orders,
            market: self.market,
            asks: self.asks,
            bids: self.bids,
        };

        process_instruction(&mut self.state, data, &accounts, &[])
    }

    pub fn cancel_order_by_client_order_id(
        &mut self,
        user_id: &UserId,
        data: &openbook_v2::instruction::CancelOrderByClientOrderId,
    ) -> ProgramResult {
        let Some(user) = self.users.get(user_id) else {
            return Ok(());
        };

        let accounts = openbook_v2::accounts::CancelOrder {
            signer: user.owner,
            open_orders_account: user.open_orders,
            market: self.market,
            asks: self.asks,
            bids: self.bids,
        };

        process_instruction(&mut self.state, data, &accounts, &[])
    }

    pub fn cancel_all_orders(
        &mut self,
        user_id: &UserId,
        data: &openbook_v2::instruction::CancelAllOrders,
    ) -> ProgramResult {
        let Some(user) = self.users.get(user_id) else {
            return Ok(());
        };

        let accounts = openbook_v2::accounts::CancelOrder {
            signer: user.owner,
            open_orders_account: user.open_orders,
            market: self.market,
            asks: self.asks,
            bids: self.bids,
        };

        process_instruction(&mut self.state, data, &accounts, &[])
    }

    pub fn settle_funds(
        &mut self,
        user_id: &UserId,
        data: &openbook_v2::instruction::SettleFunds,
        referrer_id: Option<&ReferrerId>,
    ) -> ProgramResult {
        let referrer = referrer_id.map(|id| *self.get_or_create_new_referrer(id));
        let Some(user) = self.users.get(user_id) else {
            return Ok(());
        };

        let accounts = openbook_v2::accounts::SettleFunds {
            owner: user.owner,
            open_orders_account: user.open_orders,
            token_base_account: user.base_vault,
            token_quote_account: user.quote_vault,
            market: self.market,
            base_vault: self.base_vault,
            quote_vault: self.quote_vault,
            token_program: spl_token::ID,
            system_program: system_program::ID,
            referrer,
        };

        process_instruction(&mut self.state, data, &accounts, &[])
    }

    pub fn sweep_fees(&mut self, data: &openbook_v2::instruction::SweepFees) -> ProgramResult {
        let accounts = openbook_v2::accounts::SweepFees {
            collect_fee_admin: self.collect_fee_admin,
            token_receiver_account: self.collect_fee_admin_quote_vault,
            market: self.market,
            quote_vault: self.quote_vault,
            token_program: spl_token::ID,
            system_program: system_program::ID,
        };

        process_instruction(&mut self.state, data, &accounts, &[])
    }

    pub fn stub_oracle_set(
        &mut self,
        data: &openbook_v2::instruction::StubOracleSet,
    ) -> ProgramResult {
        let accounts = openbook_v2::accounts::StubOracleSet {
            oracle: self.oracle,
            owner: self.admin,
        };

        process_instruction(&mut self.state, data, &accounts, &[])
    }
}