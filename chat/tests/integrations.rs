use chatter_lib::chatter::chat_service_client::ChatServiceClient;
use chatter_lib::chatter::chat_service_server::ChatServiceServer;
use chatter_lib::chatter::{ClientMessage, JoinRequest, LeaveRequest};
use chatter_lib::{relay, ChatServer};
use serial_test::serial;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tonic::transport::Server;
use tonic::{Code, Request};

// This keeps things simple but ultimately means the tests need to be run
// serially to avoid multiple servers binding to the same port.
static ADDR: &'static str = "127.0.0.1:8080";

#[tokio::test]
async fn connecion_error() {
    let result = ChatServiceClient::connect("dummy").await;

    assert!(result.is_err());
}

fn default_server() -> (JoinHandle<()>, JoinHandle<()>, oneshot::Sender<()>) {
    let (tx, rx) = tokio::sync::mpsc::channel(10);
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
    let server = ChatServer {
        messages_sender: tx,
        user_streams: Default::default(),
        user_mappings: Default::default(),
    };

    let client_streams = server.user_streams.clone();
    let relay_handle = tokio::spawn(async move {
        relay(rx, client_streams).await;
    });

    let server_handle = tokio::spawn(async move {
        Server::builder()
            .add_service(ChatServiceServer::new(server))
            .serve_with_shutdown(ADDR.parse().unwrap(), async { drop(exit_rx.await) })
            .await
            .unwrap();
    });

    return (server_handle, relay_handle, exit_tx);
}

#[tokio::test]
#[serial]
async fn connection_ok() {
    let (server_handle, _relay_handle, exit_tx) = default_server();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = ChatServiceClient::connect(format!("http://{ADDR}")).await;

    assert!(result.is_ok());

    let _ = exit_tx.send(());
    let _ = server_handle.await;
}

#[tokio::test]
#[serial]
async fn join_room_ok() {
    let (server_handle, _relay_handle, exit_tx) = default_server();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = ChatServiceClient::connect(format!("http://{ADDR}"))
        .await
        .unwrap();

    let username = "foo";
    let request = Request::new(JoinRequest {
        username: username.into(),
    });

    let response = client.join_room(request).await;

    assert!(response.is_ok());

    let _ = exit_tx.send(());
    let _ = server_handle.await;
}

#[tokio::test]
#[serial]
async fn join_room_err() {
    let (server_handle, _relay_handle, exit_tx) = default_server();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = ChatServiceClient::connect(format!("http://{ADDR}"))
        .await
        .unwrap();

    let username = "foo";
    let request = Request::new(JoinRequest {
        username: username.into(),
    });

    let response = client.join_room(request).await;
    assert!(response.is_ok());

    let mut bad_client = ChatServiceClient::connect(format!("http://{ADDR}"))
        .await
        .unwrap();

    let bad_request = Request::new(JoinRequest {
        username: username.into(),
    });
    let bad_response = bad_client.join_room(bad_request).await;
    assert!(bad_response.is_err());
    assert_eq!(bad_response.unwrap_err().code(), Code::AlreadyExists,);

    let _ = exit_tx.send(());
    let _ = server_handle.await;
}

#[tokio::test]
#[serial]
async fn leave_room_ok() {
    let (server_handle, _relay_handle, exit_tx) = default_server();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = ChatServiceClient::connect(format!("http://{ADDR}"))
        .await
        .unwrap();

    let username = "foo";
    let request = Request::new(JoinRequest {
        username: username.into(),
    });

    let response = client.join_room(request).await;

    let request = Request::new(LeaveRequest {
        id: response.unwrap().get_ref().id.clone(),
    });

    let response = client.leave_room(request).await;
    assert!(response.is_ok());

    let _ = exit_tx.send(());
    let _ = server_handle.await;
}

// #[timeout(2000)]
#[tokio::test]
#[serial]
async fn message_ok() {
    let (server_handle, relay_handle, exit_tx) = default_server();

    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut client = ChatServiceClient::connect(format!("http://{ADDR}"))
        .await
        .unwrap();

    let username = "foo".to_string();
    let request = Request::new(JoinRequest {
        username: username.clone(),
    });

    let response = client.join_room(request).await;
    assert!(response.is_ok());

    let id = response.unwrap().get_ref().id.clone();

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

    let messages: Vec<ClientMessage> = vec![
        ClientMessage {
            id: id.clone(),
            username: username.clone(),
            message: "send hello".to_string(),
        },
        ClientMessage {
            id: id.clone(),
            username: username.clone(),
            message: "leave".to_string(),
        },
    ];

    for msg in messages {
        assert!(tx.send(msg).is_ok());
    }

    let inbound_stream = Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx));
    let response_stream = client.message(tonic::Request::new(inbound_stream)).await;
    assert!(response_stream.is_ok());
    let _response_stream = response_stream.unwrap().into_inner();

    // Closes connection to server.
    drop(tx);

    let request = Request::new(LeaveRequest { id });
    let response = client.leave_room(request).await;
    assert!(response.is_ok());

    let _ = exit_tx.send(());
    let _ = relay_handle.await;
    let _ = server_handle.await;
}

#[tokio::test]
#[serial]
async fn message_two_client_ok() {
    // Create server
    let (server_handle, relay_handle, exit_tx) = default_server();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // Create first client
    let mut foo_client = ChatServiceClient::connect(format!("http://{ADDR}"))
        .await
        .unwrap();

    let foo_username = "foo".to_string();
    let request = Request::new(JoinRequest {
        username: foo_username.clone(),
    });

    let response = foo_client.join_room(request).await;
    assert!(response.is_ok());

    let foo_id = response.unwrap().get_ref().id.clone();

    // Queue messages
    let (foo_tx, foo_rx) = tokio::sync::mpsc::unbounded_channel();
    let foo_messages: Vec<ClientMessage> = vec![
        ClientMessage {
            id: foo_id.clone(),
            username: foo_username.clone(),
            message: "JOINING".to_string(),
        },
        ClientMessage {
            id: foo_id.clone(),
            username: foo_username.clone(),
            message: "send hello".to_string(),
        },
        ClientMessage {
            id: foo_id.clone(),
            username: foo_username.clone(),
            message: "send how are you?".to_string(),
        },
    ];

    // Create second client
    let mut bar_client = ChatServiceClient::connect(format!("http://{ADDR}"))
        .await
        .unwrap();

    let bar_username = "bar".to_string();
    let request = Request::new(JoinRequest {
        username: bar_username.clone(),
    });

    let response = bar_client.join_room(request).await;
    assert!(response.is_ok());

    let bar_id = response.unwrap().get_ref().id.clone();

    // Queue messages
    let (bar_tx, bar_rx) = tokio::sync::mpsc::unbounded_channel();
    let bar_messages: Vec<ClientMessage> = vec![
        ClientMessage {
            id: bar_id.clone(),
            username: bar_username.clone(),
            message: "JOINING".to_string(),
        },
        ClientMessage {
            id: bar_id.clone(),
            username: bar_username.clone(),
            message: "send hey".to_string(),
        },
        ClientMessage {
            id: bar_id.clone(),
            username: bar_username.clone(),
            message: "send good, and you?".to_string(),
        },
    ];

    // Queue handshake message
    assert!(foo_tx.send(foo_messages[0].clone()).is_ok());
    assert!(bar_tx.send(bar_messages[0].clone()).is_ok());

    // Handle return values
    let foo_inbound_stream = Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(foo_rx));
    let foo_response_stream = foo_client
        .message(tonic::Request::new(foo_inbound_stream))
        .await;
    assert!(foo_response_stream.is_ok());
    let mut foo_response_stream = foo_response_stream.unwrap().into_inner();

    let bar_inbound_stream = Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(bar_rx));
    let bar_response_stream = bar_client
        .message(tonic::Request::new(bar_inbound_stream))
        .await;
    assert!(bar_response_stream.is_ok());
    let mut bar_response_stream = bar_response_stream.unwrap().into_inner();

    // Send messages in conversation order
    assert!(foo_tx.send(foo_messages[1].clone()).is_ok());
    assert!(bar_tx.send(bar_messages[1].clone()).is_ok());
    assert!(foo_tx.send(foo_messages[2].clone()).is_ok());
    assert!(bar_tx.send(bar_messages[2].clone()).is_ok());

    // Foo should have two messages
    let resp = foo_response_stream.message().await;
    assert!(resp.is_ok_and(|r| r.is_some()));
    let resp = foo_response_stream.message().await;
    assert!(resp.is_ok_and(|r| r.is_some()));

    // Bar should have two messages
    let resp = bar_response_stream.message().await;
    assert!(resp.is_ok_and(|r| r.is_some()));
    let resp = bar_response_stream.message().await;
    assert!(resp.is_ok_and(|r| r.is_some()));

    // Closes connection to server.
    drop(foo_tx);
    drop(bar_tx);

    // Leave room
    let request = Request::new(LeaveRequest { id: foo_id });
    let response = foo_client.leave_room(request).await;
    assert!(response.is_ok());
    let request = Request::new(LeaveRequest { id: bar_id });
    let response = bar_client.leave_room(request).await;
    assert!(response.is_ok());

    let _ = exit_tx.send(());
    let _ = relay_handle.await;
    let _ = server_handle.await;
}
