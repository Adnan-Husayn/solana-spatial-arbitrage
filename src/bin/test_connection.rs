use futures::StreamExt;
use solana_client::nonblocking::pubsub_client::PubsubClient;
use spatial_arbitrage_bot::load_env_variables;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (rpc_url, _) = load_env_variables()?;
    let ws_url = rpc_url.replace("https", "wss");

    println!("TESTING CONNECTION TO: {}", ws_url);
    let pubsub_client = PubsubClient::new(&ws_url).await?;

    let (mut stream, _unsub) = pubsub_client.slot_subscribe().await?;

    println!("Connected. Waiting for heartbeat/slots ..");

    while let Some(slot_info) = stream.next().await {
        println!("Pulse: Slot {}", slot_info.slot);
    }

    Ok(())
}
