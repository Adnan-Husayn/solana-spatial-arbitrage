use futures::StreamExt;
use solana_client::{
    nonblocking::pubsub_client::PubsubClient,
    rpc_config::{RpcTransactionLogsConfig, RpcTransactionLogsFilter},
};
use solana_sdk::commitment_config::CommitmentConfig;
use spatial_arbitrage_bot::load_env_variables;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let (rpc_url, _) = load_env_variables()?;
    let ws_url = rpc_url.replace("https", "wss");

    println!("Connecting to Log Stream: {}", ws_url);
    let pubsub_client = PubsubClient::new(&ws_url).await?;

    let pool_id = "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2";

    println!("Listening for trades on Pool: {}", pool_id);

    let filter = RpcTransactionLogsFilter::Mentions(vec![pool_id.to_string()]);
    let config = RpcTransactionLogsConfig {
        commitment: Some(CommitmentConfig::processed()),
    };

    let (mut stream, _unsub) = pubsub_client.logs_subscribe(filter, config).await?;

    while let Some(response) = stream.next().await {
        println!("TRADE DETECTED!! Tx: {}", response.value.signature);
    }

    Ok(())
}
