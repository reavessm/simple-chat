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
    // Set up internal streams.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (exit_tx, mut exit_rx) = tokio::sync::mpsc::channel(1);
    let (io_tx, mut io_rx) = tokio::sync::mpsc::channel::<String>(10);
    let io_tx_clone = io_tx.clone();

    let mut client = ChatServiceClient::connect("http://127.0.0.1:8080").await?;

    let mut username = String::new();
    print!("Please enter a username: ");
    stdout().flush()?;
    stdin().read_line(&mut username)?;

    // remove newlines
    let username = username.trim().to_string();

    let request = tonic::Request::new(JoinRequest {
        username: username.clone(),
    });

    // Enter chat room to receive messages.
    let response = client.join_room(request).await?;
    let id = response.get_ref().id.clone();

    // Send initial message to get channel from server
    let initial_message_text = "JOINING";
    let initial_message = ClientMessage {
        id: id.clone(),
        username: username.clone(),
        message: initial_message_text.to_string(),
    };

    if tx.send(initial_message).is_err() {
        let err_msg = "Failed to send initial message";
        eprintln!("{err_msg}");
        return Err(err_msg.into());
    }

    // Spawn a thread to handle printing to the screen.  This can come from our
    // client or from the server so we use an internal channel.
    let print_handle = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = io_rx.recv().await {
            let _ = stdout.write_all(message.as_bytes()).await;
            let _ = stdout.flush().await;
        }
    });

    // Handle user input
    let id_clone = id.clone();
    let input_handle = tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut stdin_reader = tokio::io::BufReader::new(stdin).lines();

        let prompt = "> ";
        let _ = io_tx.send(prompt.into()).await;

        while let Ok(Some(line)) = stdin_reader.next_line().await {
            let message = line.trim().to_string();

            // Handle exiting by alerting another thread to do the clean up.
            if message == "leave" {
                drop(exit_tx.send(()));
                return;
            }

            let message = message.strip_prefix("send ");
            if message.is_none() {
                eprintln!("ERROR: Invalid message.  Must be of the form 'leave' or 'send <msg>'");
                let _ = io_tx.send(prompt.into()).await;
                continue;
            }
            let message = message.unwrap().to_string();

            let message_request = ClientMessage {
                message,
                id: id_clone.clone(),
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

    // Handle replies from other clients
    let reply_handle = tokio::spawn(async move {
        pin_mut!(response_stream);

        loop {
            tokio::select! {
                biased;
                Some(response) = response_stream.next() => {
                    match response {
                        Ok(resp) => {
                            // \r should clear the line before printing.  Then
                            // we put the prompt back and the user is
                            // none-the-wiser!
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

    // Leave the room only after all the threads have returned.
    let request = tonic::Request::new(LeaveRequest { id });
    let _ = client.leave_room(request).await?;

    Ok(())
}
