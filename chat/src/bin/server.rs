use std::collections::HashSet;
use tokio::sync::RwLock;
use tonic::{transport::Server, Code, Request, Response, Status};

pub mod chatter {
    tonic::include_proto!("chatter");
}

use chatter::chat_service_server::{ChatService, ChatServiceServer};
use chatter::{JoinRequest, JoinResponse};

#[derive(Debug, Default)]
pub struct ChatServer {
    usernames: RwLock<HashSet<String>>,
}

#[tonic::async_trait]
impl ChatService for ChatServer {
    async fn join_room(
        &self,
        request: Request<JoinRequest>,
    ) -> Result<Response<JoinResponse>, Status> {
        let username = &request.get_ref().username;

        if self.usernames.read().await.contains(username) {
            return Err(Status::new(Code::AlreadyExists, "username already exists"));
        }

        self.usernames.write().await.insert(username.clone());

        Ok(Response::new(JoinResponse::default()))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:8080".parse()?;
    let server = ChatServer::default();

    println!("Listening on {addr} ...");

    Server::builder()
        .add_service(ChatServiceServer::new(server))
        .serve(addr)
        .await?;

    Ok(())
}
