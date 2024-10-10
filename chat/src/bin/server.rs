use std::collections::{HashMap, HashSet};
use tokio::sync::RwLock;
use tonic::{transport::Server, Code, Request, Response, Status};
use uuid::Uuid;

pub mod chatter {
    tonic::include_proto!("chatter");
}

use chatter::chat_service_server::{ChatService, ChatServiceServer};
use chatter::{JoinRequest, JoinResponse, LeaveRequest, LeaveResponse};

#[derive(Debug, Default)]
pub struct UserMappings {
    usernames: HashSet<String>,
    uuid_to_usernames: HashMap<Uuid, String>,
}

#[derive(Debug, Default)]
pub struct ChatServer {
    user_mappings: RwLock<UserMappings>,
}

#[tonic::async_trait]
impl ChatService for ChatServer {
    async fn join_room(
        &self,
        request: Request<JoinRequest>,
    ) -> Result<Response<JoinResponse>, Status> {
        let username = &request.get_ref().username;

        if self.user_mappings.read().await.usernames.contains(username) {
            println!("Could not add {username} ...");
            return Err(Status::new(Code::AlreadyExists, "username already exists"));
        }

        let user_id = Uuid::new_v4();

        println!("{username} is joining");

        {
            let mut guard = self.user_mappings.write().await;

            guard.usernames.insert(username.clone());
            guard.uuid_to_usernames.insert(user_id, username.clone());
        }

        Ok(Response::new(JoinResponse { id: user_id.into() }))
    }

    async fn leave_room(
        &self,
        request: Request<LeaveRequest>,
    ) -> Result<Response<LeaveResponse>, Status> {
        let id_string = &request.get_ref().id;
        let id = Uuid::try_parse(id_string);

        if id.is_err() {
            return Err(Status::invalid_argument(format!(
                "Invalid uuid: {id_string}"
            )));
        }

        let id = id.unwrap();

        if !self
            .user_mappings
            .read()
            .await
            .uuid_to_usernames
            .contains_key(&id)
        {
            println!("Invalid uuid");
            return Err(Status::not_found(format!("UUID not found: {id_string}")));
        }

        {
            let mut guard = self.user_mappings.write().await;

            let (_, username) = guard
                .uuid_to_usernames
                .remove_entry(&id)
                .expect("UUID Unexpectedly deleted");

            println!("User {username} is leaving ...");

            guard.usernames.remove(&username);
        }

        Ok(Response::new(LeaveResponse::default()))
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
