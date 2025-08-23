use lib::atomic::{Atomic, Transaction};
use rand::Rng;
use std::sync::Arc;
use std::thread;

#[derive(Debug)]
struct Customer {
    funds: u32,
}

#[test]
pub fn atomic_transaction() {
    let customer_01 = Arc::new(Atomic::new_cas(Customer { funds: u32::MAX }, u16::MAX));
    let customer_02 = Arc::new(Atomic::new_cas(Customer { funds: 0 }, u16::MAX));

    let total_funds = (0..5)
        .map(|_| {
            thread::spawn({
                let customer_01 = customer_01.clone();
                let customer_02 = customer_02.clone();
                move || {
                    let mut total_funds = 0;
                    loop {
                        let rnd_funds: u32 = rand::rng().random_range(0..1000);

                        let transaction = Transaction::new(2)
                            .add(customer_02.as_ref(), move |customer| {
                                Some(Customer {
                                    funds: customer.funds + rnd_funds,
                                })
                            })
                            .add(customer_01.as_ref(), move |customer| {
                                if customer.funds < rnd_funds {
                                    None
                                } else {
                                    Some(Customer {
                                        funds: customer.funds - rnd_funds,
                                    })
                                }
                            });
                        if !transaction.execute() {
                            break;
                        }

                        total_funds += rnd_funds;
                    }
                    total_funds
                }
            })
        })
        .filter_map(|handle| handle.join().ok())
        .reduce(|base, other| base + other)
        .unwrap();

    assert_eq!(total_funds, customer_02.as_ref().read().get().funds);
    assert_eq!(
        u32::MAX - total_funds,
        customer_01.as_ref().read().get().funds
    );
}
