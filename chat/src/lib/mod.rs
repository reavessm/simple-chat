pub mod chatter {
    tonic::include_proto!("chatter");
}

use chatter::ServerMessage;
use futures::future;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio_stream::Stream;
use tonic::Status;
use uuid::Uuid;

// holy type signature batman!
pub type MessageStream =
    Pin<Box<dyn Stream<Item = Result<ServerMessage, Status>> + Send + 'static>>;
pub type ClientStream = Arc<RwLock<HashMap<Uuid, Sender<Result<ServerMessage, Status>>>>>;

use chatter::chat_service_server::ChatService;
use chatter::{ClientMessage, JoinRequest, JoinResponse, LeaveRequest, LeaveResponse};
use std::collections::HashSet;
use tonic::{Code, Request, Response, Streaming};

/// UserMappings stores a set of usernames and a mapping from id to username.
///
/// The set of usernames maintains uniqueness, and the UUID maintains a (weak)
/// sense of "security" that the message came from the client.  Every client
/// can see any other client's username, but not their UUID.  A client's UUID
/// should be treated like a session token.
#[derive(Debug, Default)]
pub struct UserMappings {
    usernames: HashSet<String>,
    uuid_to_usernames: HashMap<Uuid, String>,
}

#[derive(Debug)]
pub struct ChatServer {
    /// List of connected clients.
    pub user_mappings: RwLock<UserMappings>,

    /// Pipes to send messages to clients.
    //user_streams: Arc<RwLock<HashMap<Uuid, Sender<Result<ServerMessage, Status>>>>>,
    pub user_streams: ClientStream,

    /// Relayable messages.
    pub messages_sender: Sender<(Uuid, ServerMessage)>,
}

#[tonic::async_trait]
impl ChatService for ChatServer {
    type MessageStream = MessageStream;

    /// join_room handles the initial request from the client to enter the
    /// chat.  They should be able to recieve messages once this succeeds.
    async fn join_room(
        &self,
        request: Request<JoinRequest>,
    ) -> Result<Response<JoinResponse>, Status> {
        let username = &request.get_ref().username;
        let mut username = username.to_string();

        // TODO: Make this configurable?
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

        // Store username and unique id.
        {
            let mut guard = self.user_mappings.write().await;

            guard.usernames.insert(username.clone());
            guard.uuid_to_usernames.insert(user_id, username.clone());
        }

        Ok(Response::new(JoinResponse { id: user_id.into() }))
    }

    /// A parting-shot that a client sends as it exits cleanly.  All clean-up
    /// should happen here and we shouldn't expect any messages from this
    /// client until they rejoin.
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

        self.remove_user(id).await?;

        Ok(Response::new(LeaveResponse::default()))
    }

    /// message handles receiving messages from a client.  The client should
    /// have already joined by this point.  This method opens bidirectional
    /// channel with the client.  Any incoming messages will be relayed to all
    /// other clients, not including the sender.
    async fn message(
        &self,
        request: Request<Streaming<ClientMessage>>,
    ) -> Result<Response<MessageStream>, Status> {
        // grab gRPC channel
        let mut stream = request.into_inner();

        // handle first message before loop to setup routing
        let Ok(Some(msg)) = stream.message().await else {
            return Err(Status::unknown("Unkown error"));
        };

        // This is done separetely because it only needs to be done once, not
        // on every message.
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

        // Create channel to give to client.
        let output_stream = async_stream::stream! {
            while let Some(msg) = rx.recv().await {
                yield msg;
            }
        };

        // Store one end of the client channel so we can relay messages into
        // it.
        self.user_streams.write().await.insert(id, tx);

        // For every message we recieve, send to every other client.  The
        // actual sending happens in `relay`, but this sends the message to the
        // relay.
        let sender = self.messages_sender.clone();
        let forward_result = tokio::spawn(async move {
            while let Ok(msg) = stream.message().await {
                if msg.is_none() {
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

        // drive and ignore
        drop(forward_result);

        // Send loose end of channel to client.
        Ok(Response::new(Box::pin(output_stream) as MessageStream))
    }
}

impl ChatServer {
    /// remove_user is a general function to clean up the housekeeping related
    /// to removing a user.
    async fn remove_user(&self, uuid: Uuid) -> Result<(), Status> {
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

/// relay receives messages from the server and asynchronously sends them to
/// each client.
pub async fn relay(
    mut server_receiver: Receiver<(Uuid, ServerMessage)>,
    client_streams: ClientStream,
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
