use futures::future;
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_stream::Stream;
use tonic::{transport::Server, Code, Request, Response, Status, Streaming};
use uuid::Uuid;

pub mod chatter {
    tonic::include_proto!("chatter");
}

use chatter::chat_service_server::{ChatService, ChatServiceServer};
use chatter::{
    ClientMessage, JoinRequest, JoinResponse, LeaveRequest, LeaveResponse, ServerMessage,
};

#[derive(Debug, Default)]
pub struct UserMappings {
    usernames: HashSet<String>,
    uuid_to_usernames: HashMap<Uuid, String>,
}

#[derive(Debug)]
pub struct ChatServer {
    user_mappings: RwLock<UserMappings>,
    user_streams: Arc<RwLock<HashMap<Uuid, Sender<Result<ServerMessage, Status>>>>>,
    messages_sender: Sender<(Uuid, ServerMessage)>,
}

#[tonic::async_trait]
impl ChatService for ChatServer {
    async fn join_room(
        &self,
        request: Request<JoinRequest>,
    ) -> Result<Response<JoinResponse>, Status> {
        let username = &request.get_ref().username;
        let mut username = username.to_string();
        username.truncate(10);

        if self
            .user_mappings
            .read()
            .await
            .usernames
            .contains(&username)
        {
            eprintln!("Could not add {username} ...");
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

        if let Err(reason) = self.remove_user(id).await {
            return Err(reason);
        }

        Ok(Response::new(LeaveResponse::default()))
    }

    type MessageStream =
        Pin<Box<dyn Stream<Item = Result<ServerMessage, Status>> + Send + 'static>>;

    async fn message(
        &self,
        request: Request<Streaming<ClientMessage>>,
    ) -> Result<Response<Self::MessageStream>, Status> {
        let mut stream = request.into_inner();

        let Ok(Some(msg)) = stream.message().await else {
            return Err(Status::unknown("Unkown error"));
        };

        let (username, id, message) = {
            let guard = self.user_mappings.read().await;

            let id = Uuid::try_parse(&msg.id);

            if id.is_err() {
                return Err(Status::invalid_argument(format!(
                    "Invalid uuid: {}",
                    msg.id
                )));
            }

            let id = id.unwrap();

            let username = guard.uuid_to_usernames.get(&id);

            if username.is_none() {
                return Err(Status::not_found(format!("UUID not found: {}", msg.id)));
            }

            (username.unwrap().clone(), id, msg.message)
        };

        let (tx, mut rx) = tokio::sync::mpsc::channel(10);

        let timestamp = chrono::Utc::now().to_rfc3339();

        println!("{} {}: {}", timestamp.clone(), username, message);

        let output_stream = async_stream::stream! {
            while let Some(msg) = rx.recv().await {
                yield msg;
            }
        };

        // TODO: Add stream to server

        // {
        //     // let mut guard = self.user_streams.write().await;
        //     //
        //     // guard.insert(id, tx.into());
        // }
        self.user_streams.write().await.insert(id, tx.into());

        let sender = self.messages_sender.clone();

        let _ = tokio::spawn(async move {
            while let Ok(msg) = stream.message().await {
                println!("Looping");
                if msg.is_none() {
                    println!("no msg");
                    break;
                }

                let timestamp = chrono::Utc::now().to_rfc3339();

                let message = msg.unwrap().message;
                let username = username.clone();

                println!("{} {}: {}", timestamp.clone(), username, message);

                let relay_result = sender
                    .send((
                        id,
                        ServerMessage {
                            username,
                            message,
                            timestamp,
                        },
                    ))
                    .await;
                if relay_result.is_err() {
                    eprintln!("ERROR: {:?}", relay_result);
                }
            }
        });

        Ok(Response::new(Box::pin(output_stream) as Self::MessageStream))
    }
}

impl ChatServer {
    async fn remove_user(&self, uuid: Uuid) -> Result<(), Status> {
        println!("Removing");
        if !self
            .user_mappings
            .read()
            .await
            .uuid_to_usernames
            .contains_key(&uuid)
        {
            println!("Id not found");
            return Err(Status::not_found(format!("UUID not found: {uuid}")));
        }

        let mut guard = self.user_mappings.write().await;
        let mut stream_guard = self.user_streams.write().await;

        let (_, username) = guard
            .uuid_to_usernames
            .remove_entry(&uuid)
            .expect("UUID Unexpectedly deleted");

        println!("User {username} is leaving ...");

        guard.usernames.remove(&username);
        stream_guard.remove_entry(&uuid);

        println!("User {username} has left.");

        Ok(())
    }
}

async fn relay(
    mut server_receiver: Receiver<(Uuid, ServerMessage)>,
    client_streams: Arc<RwLock<HashMap<Uuid, Sender<Result<ServerMessage, Status>>>>>,
) {
    while let Some((sender_id, msg)) = server_receiver.recv().await {
        {
            let guard = client_streams.read().await;

            let mut handles: Vec<JoinHandle<()>> = Vec::with_capacity(guard.len());

            for (_uuid, sender) in guard.iter().filter(|(&u, _)| u != sender_id) {
                let message = msg.clone();
                let sender = sender.clone();

                let handle = tokio::spawn(async move {
                    let _result = sender.send(Ok(message)).await;
                });

                handles.push(handle);
            }
            let _ = future::join_all(handles).await;
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "0.0.0.0:8080".parse()?;
    let (tx, rx) = tokio::sync::mpsc::channel(10);
    let server = ChatServer {
        messages_sender: tx,
        user_streams: Default::default(),
        user_mappings: Default::default(),
    };

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
