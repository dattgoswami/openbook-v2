use super::*;

#[tokio::test]
async fn test_fees_acrued() -> Result<(), TransportError> {
    let context = TestContext::new().await;
    let solana = &context.solana.clone();

    let admin = TestKeypair::new();
    let owner = context.users[0].key;
    let payer = context.users[1].key;
    let mints = &context.mints[0..=2];

    let owner_token_0 = context.users[0].token_accounts[0];
    let owner_token_1 = context.users[0].token_accounts[1];

    let tokens = Token::create(mints.to_vec(), solana, admin, payer).await;

    //
    // TEST: Create a market
    //

    let market = get_market_address_by_index(1);
    let base_vault = solana
        .create_associated_token_account(&market, mints[0].pubkey)
        .await;
    let quote_vault = solana
        .create_associated_token_account(&market, mints[1].pubkey)
        .await;

    let openbook_v2::accounts::CreateMarket {
        market,
        base_vault,
        quote_vault,
        ..
    } = send_tx(
        solana,
        CreateMarketInstruction {
            admin,
            payer,
            market_index: 1,
            quote_lot_size: 10,
            base_lot_size: 100,
            maker_fee: 0.0001,
            taker_fee: 0.0002, // 2bps
            base_mint: mints[0].pubkey,
            quote_mint: mints[1].pubkey,
            base_vault,
            quote_vault,
            ..CreateMarketInstruction::with_new_book_and_queue(solana, &tokens[1]).await
        },
    )
    .await
    .unwrap();

    let account_0 = create_funded_account(solana, owner, market, 0, &context.users[1]).await;
    let account_1 = create_funded_account(solana, owner, market, 1, &context.users[1]).await;

    let price_lots = {
        let market = solana.get_account::<Market>(market).await;
        market.native_price_to_lot(I80F48::from(1000))
    };

    // Set the initial oracle price
    set_stub_oracle_price(solana, &tokens[1], admin, 1000.0).await;

    send_tx(
        solana,
        PlaceOrderInstruction {
            open_orders_account: account_0,
            market,
            owner,
            payer: owner_token_1,
            base_vault,
            quote_vault,
            side: Side::Bid,
            price_lots,
            max_base_lots: 1,
            max_quote_lots_including_fees: 10000,
            reduce_only: false,
            client_order_id: 0,
            expiry_timestamp: 0,
        },
    )
    .await
    .unwrap();

    send_tx(
        solana,
        PlaceOrderInstruction {
            open_orders_account: account_1,
            market,
            owner,
            payer: owner_token_0,
            base_vault,
            quote_vault,
            side: Side::Ask,
            price_lots,
            max_base_lots: 1,
            max_quote_lots_including_fees: 10000,
            reduce_only: false,
            client_order_id: 0,
            expiry_timestamp: 0,
        },
    )
    .await
    .unwrap();

    {
        let open_orders_account_0 = solana.get_account::<OpenOrdersAccount>(account_0).await;
        let open_orders_account_1 = solana.get_account::<OpenOrdersAccount>(account_1).await;

        println!(
            "base_position_lots {}",
            open_orders_account_0.position.base_position_lots
        );
        println!(
            "quote_position_native {}",
            open_orders_account_0.position.quote_position_native
        );

        assert_eq!(open_orders_account_0.position.base_position_lots(), 0);
        assert_eq!(open_orders_account_1.position.base_position_lots(), 0);
        assert_eq!(
            open_orders_account_0
                .position
                .quote_position_native()
                .round(),
            0
        );
        // assert_eq!(open_orders_account_1.position.quote_position_native(), 0);
        assert_eq!(open_orders_account_0.position.bids_base_lots, 1);
        assert_eq!(open_orders_account_1.position.bids_base_lots, 0);
        assert_eq!(open_orders_account_0.position.asks_base_lots, 0);
        assert_eq!(open_orders_account_1.position.asks_base_lots, 0);
        assert_eq!(open_orders_account_0.position.taker_base_lots, 0);
        assert_eq!(open_orders_account_1.position.taker_quote_lots, 10000);
        assert_eq!(open_orders_account_0.position.base_free_native, 0);
        assert_eq!(open_orders_account_1.position.base_free_native, 0);
        assert_eq!(open_orders_account_0.position.quote_free_native, 0);
        assert_eq!(
            open_orders_account_1.position.quote_free_native.round(),
            99980
        );
    }

    {
        let market_acc = solana.get_account::<Market>(market).await;
        println!("xxx {}", market_acc.fees_accrued);
    }

    send_tx(
        solana,
        ConsumeEventsInstruction {
            market,
            open_orders_accounts: vec![account_0, account_1],
        },
    )
    .await
    .unwrap();

    {
        let open_orders_account_0 = solana.get_account::<OpenOrdersAccount>(account_0).await;
        let open_orders_account_1 = solana.get_account::<OpenOrdersAccount>(account_1).await;

        println!(
            "0 base_position_lots {}",
            open_orders_account_0.position.base_position_lots
        );
        println!(
            "0 quote_position_native {}",
            open_orders_account_0.position.quote_position_native
        );

        println!(
            "1 base_position_lots {}",
            open_orders_account_1.position.base_position_lots
        );
        println!(
            "1 quote_position_native {}",
            open_orders_account_1.position.quote_position_native
        );

        assert_eq!(open_orders_account_0.position.base_position_lots(), 1);
        assert_eq!(open_orders_account_1.position.base_position_lots(), -1);
        assert_eq!(
            open_orders_account_0
                .position
                .quote_position_native()
                .round(),
            -100_010
        );
        // assert_eq!(
        //     open_orders_account_1.position.quote_position_native(),
        //     100_000
        // );
        assert_eq!(open_orders_account_0.position.bids_base_lots, 0);
        assert_eq!(open_orders_account_1.position.bids_base_lots, 0);
        assert_eq!(open_orders_account_0.position.asks_base_lots, 0);
        assert_eq!(open_orders_account_1.position.asks_base_lots, 0);
        assert_eq!(open_orders_account_0.position.taker_base_lots, 0);
        assert_eq!(open_orders_account_1.position.taker_quote_lots, 0);
        assert_eq!(open_orders_account_0.position.base_free_native, 100);
        assert_eq!(open_orders_account_1.position.base_free_native, 0);
        assert_eq!(open_orders_account_0.position.quote_free_native.round(), 10);
        assert_eq!(
            open_orders_account_1.position.quote_free_native.round(),
            99980
        );
    }

    Ok(())
}