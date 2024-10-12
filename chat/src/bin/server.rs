use chatter_lib::chatter::chat_service_server::ChatServiceServer;
use chatter_lib::{relay, ChatServer};
use std::env;
use tonic::transport::Server;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = match env::var("SERVER_HOST") {
        Ok(h) => h,
        Err(_) => "0.0.0.0".to_string(),
    };
    let port = match env::var("SERVER_PORT") {
        Ok(p) => p,
        Err(_) => "8080".to_string(),
    };
    let addr = format!("{host}:{port}").parse()?;

    let (tx, rx) = tokio::sync::mpsc::channel(10);
    let server = ChatServer {
        messages_sender: tx,
        user_streams: Default::default(),
        user_mappings: Default::default(),
    };

    // Set up internal streams.
    let client_streams = server.user_streams.clone();
    let server_handle = tokio::spawn(async move {
        relay(rx, client_streams).await;
    });

    println!("Listening on {addr} ...");

    Server::builder()
        .add_service(ChatServiceServer::new(server))
        .serve(addr)
        .await?;

    let _ = server_handle.await;

    Ok(())
}
