pub mod chatter {
    tonic::include_proto!("chatter");
}

use chatter::chat_service_client::ChatServiceClient;
use chatter::{JoinRequest, LeaveRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = ChatServiceClient::connect("http://127.0.0.1:8080").await?;
    let username = String::from("foobar");

    let request = tonic::Request::new(JoinRequest { username });

    let response = client.join_room(request).await?;

    println!("Response: {:?}", response);

    std::thread::sleep(std::time::Duration::from_secs(3));

    let id = &response.get_ref().id;

    let request = tonic::Request::new(LeaveRequest { id: id.to_string() });

    let response = client.leave_room(request).await?;

    println!("Response: {:?}", response);

    Ok(())
}
