pub mod chatter {
    tonic::include_proto!("chatter");
}

use chatter::chat_service_client::ChatServiceClient;
use chatter::JoinRequest;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = ChatServiceClient::connect("http://127.0.0.1:8080").await?;

    let request = tonic::Request::new(JoinRequest {
        username: "foobar".into(),
    });

    let response = client.join_room(request).await?;

    println!("Response: {:?}", response);

    Ok(())
}
