//! Level-1 run of the parity scenarios, with real iroh endpoints on loopback
//! in one process. `headless/tests/integration/parity.rs` runs the same scenarios against
//! real headless processes. The package name `core` shadows std `core`, which
//! breaks `#[tokio::test]`, so tests run through `run`.

use std::time::Duration;

use core::discovery::testing::scenarios::{
    decline_leaves_no_trust, presence_off_persists_and_stays_dialable,
    unknown_prompts_once_then_known_reconnects_silently, InProcessDriver,
};

const LIMIT: Duration = Duration::from_secs(120);

fn run<F: std::future::Future>(fut: F) {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            tokio::time::timeout(LIMIT, fut)
                .await
                .expect("scenario timed out");
        });
}

async fn pair() -> (InProcessDriver, InProcessDriver) {
    (
        InProcessDriver::new("Alice").await,
        InProcessDriver::new("Bob").await,
    )
}

#[test]
fn unknown_prompts_once_then_known_reconnects_silently_in_process() {
    run(async {
        let (mut a, mut b) = pair().await;
        unknown_prompts_once_then_known_reconnects_silently(&mut a, &mut b).await;
    });
}

#[test]
fn decline_leaves_no_trust_in_process() {
    run(async {
        let (mut a, mut b) = pair().await;
        decline_leaves_no_trust(&mut a, &mut b).await;
    });
}

#[test]
fn presence_off_persists_and_stays_dialable_in_process() {
    run(async {
        let (mut a, mut b) = pair().await;
        presence_off_persists_and_stays_dialable(&mut a, &mut b).await;
    });
}
