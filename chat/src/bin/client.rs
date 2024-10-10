pub mod chatter {
    tonic::include_proto!("chatter");
}

use std::io::{stdin, stdout, Write};

use chatter::chat_service_client::ChatServiceClient;
use chatter::{ClientMessage, JoinRequest, LeaveRequest};
use futures::pin_mut;
use futures::stream::StreamExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = ChatServiceClient::connect("http://127.0.0.1:8080").await?;

    let mut username = String::new();

    print!("Please enter a username: ");
    stdout().flush()?;
    stdin().read_line(&mut username)?;

    let username = username.trim().to_string();

    let request = tonic::Request::new(JoinRequest {
        username: username.clone(),
    });

    let response = client.join_room(request).await?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (exit_tx, mut exit_rx) = tokio::sync::mpsc::channel(1);
    let (io_tx, mut io_rx) = tokio::sync::mpsc::channel::<String>(10);
    let io_tx_clone = io_tx.clone();

    let response = std::sync::Arc::new(response);
    let response_clone = std::sync::Arc::clone(&response);

    let initial_message_text = "JOINING";
    let initial_message = ClientMessage {
        id: response.get_ref().clone().id,
        username: username.clone(),
        message: initial_message_text.to_string(),
    };

    if tx.send(initial_message).is_err() {
        let err_msg = "Failed to send initial message";
        eprintln!("{err_msg}");
        return Err(err_msg.into());
    }

    let print_handle = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = io_rx.recv().await {
            let _ = stdout.write_all(message.as_bytes()).await;
            let _ = stdout.flush().await;
        }
    });

    let input_handle = tokio::spawn(async move {
        let id = &response_clone.get_ref().id;
        let stdin = tokio::io::stdin();
        let mut stdin_reader = tokio::io::BufReader::new(stdin).lines();

        let prompt = "> ";
        let _ = io_tx.send(prompt.into()).await;

        while let Ok(Some(line)) = stdin_reader.next_line().await {
            let message = line.trim().to_string();

            if message == "exit" {
                let _ = exit_tx.send(());
                return;
            }

            let message_request = ClientMessage {
                message,
                id: id.clone(),
                username: username.clone(),
            };

            if tx.send(message_request).is_err() {
                eprintln!("Failed to send message");
                break;
            }

            let _ = io_tx.send(prompt.into()).await;
        }
    });

    let outbound_stream = Box::pin(tokio_stream::wrappers::UnboundedReceiverStream::new(rx));

    let response_stream = client
        .message(tonic::Request::new(outbound_stream))
        .await?
        .into_inner();

    let reply_handle = tokio::spawn(async move {
        pin_mut!(response_stream);

        loop {
            tokio::select! {
                biased;
                Some(response) = response_stream.next() => {
                    match response {
                        Ok(resp) => {
                            let _ = io_tx_clone.send(format!("\r{:>.10}> {}\n> ", resp.username, resp.message)).await;
                        },
                        Err(e) => {
                            eprintln!("ERROR! {:?}", e);
                            break;
                        }
                    }
                },
                    _ = exit_rx.recv() => break,
            }
        }
    });

    let _ = futures::join!(reply_handle, input_handle, print_handle);

    let request = tonic::Request::new(LeaveRequest {
        id: response.get_ref().id.to_string(),
    });

    let _ = client.leave_room(request).await?;

    Ok(())
}

// async fn handle_message(username: String, id: String) -> impl Stream<Item = ClientMessage> {
//     async_stream::stream! {
//         // loop {
//             let username = username.clone();
//             let id = id.clone();
//
//             print!("> ");
//             stdout().flush()?;
//
//             let mut message = String::new();
//             stdin().read_line(&mut message)?;
//             let message = message.trim().to_string();
//
//             if message == "exit" {
//                 yield Err(Status::new(Code::Unknown, ""));
//                 return;
//             } //else {
//
//                 yield Ok(ClientMessage{ id, username, message });
//             // }
//
//         // }
//     }
//     .filter_map(|item| async {
//         match item {
//             Ok(value) => Some(value),
//             Err(_) => None,
//         }
//     })
// }
