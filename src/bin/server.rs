use anyhow::{anyhow, Result};
use lazy_static::lazy_static;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::SocketAddr;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::Mutex;
use tokio::task;

use simple_chat::{Message, BUFSIZE};

lazy_static! {
    static ref PEERS: PeerMap = Mutex::new(HashMap::new());
}

type PeerMap = Mutex<HashMap<SocketAddr, OwnedWriteHalf>>;

async fn run_server(addr: &str) -> Result<()> {
    let addr: SocketAddr = addr.parse()?;
    let listener = TcpListener::bind(addr).await?;
    let (tx, rx) = mpsc::channel(32);
    // start broadcast task
    task::spawn(broadcast(rx));
    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                println!("New client: {addr}");
                task::spawn(handle_connection(stream, addr, tx.clone()));
            }
            Err(e) => {
                eprintln!("couldn't get client {:?}", e);
                return Err(anyhow!("Couldn't get client connection {:?}", e));
            }
        }
    }
}

async fn handle_connection(stream: TcpStream, addr: SocketAddr, tx: Sender<Message>) -> Result<()> {
    let (r, mut w) = stream.into_split();
    if let Some(msg) = get_msg_from_stream(&r).await {
        let mut peermap = PEERS.lock().await;
        peermap.insert(addr, w);
        drop(peermap);
        tx.send(msg).await?;
        task::spawn(recv_messages(tx, r));
    } else {
        // disconnect client/shutdown stream
        w.shutdown().await?;
    }
    Ok(())
}

async fn get_msg_from_stream(stream: &OwnedReadHalf) -> Option<Message> {
    let mut buf = [0u8; BUFSIZE];
    match stream.try_read(&mut buf) {
        Ok(bytes_read) => {
            if bytes_read == 0 {
                eprintln!("Client disconnected.");
                None
            } else {
                Some(Message::try_from(&buf[..]).expect("Error serializing msg from stream"))
            }
        }
        Err(ref e) if e.kind() == ErrorKind::WouldBlock => None,
        Err(e) => {
            eprintln!("Error getting message from stream {}: {e}", e.kind());
            None
        }
    }
}

async fn broadcast(rx: Receiver<Message>) -> Result<()> {
    let mut rx = rx;
    loop {
        if let Ok(msg) = rx.try_recv() {
            if msg.content.len() == 0usize {
                continue;
            }
            let encoded = msg.serialize()?;
            let mut peermap = PEERS.lock().await;
            // eprintln!(
            //     "Sending msg to all users {:?}",
            //     std::str::from_utf8(&msg.content)?
            // );
            for (addr, stream) in peermap.iter_mut() {
                stream.writable().await?;
                match stream.try_write(&encoded) {
                    Ok(_) => {}
                    Err(e) => {
                        eprintln!("Error writing to {addr}. {}: {e}", e.kind());
                    }
                }
            }
            drop(peermap);
        }
    }
}

async fn recv_messages(tx: Sender<Message>, stream: OwnedReadHalf) -> Result<()> {
    loop {
        let mut buf = [0u8; BUFSIZE];
        stream.readable().await?;
        match stream.try_read(&mut buf) {
            Ok(bytes_read) => {
                if bytes_read == 0 {
                    // client disconnected
                    eprintln!(
                        "No bytes read from ({:?}). Dropping connection.",
                        stream.peer_addr()
                    );
                    drop_peer_connection(stream, tx).await?;
                    break;
                }
                let msg = Message::try_from(&buf[..])?;
                tx.send(msg).await?;
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                continue;
            }
            Err(e) if e.kind() == ErrorKind::ConnectionReset => {
                drop_peer_connection(stream, tx).await?;
                break;
            }
            Err(e) => {
                eprintln!(
                    "Error receiving messages from client ({:?}). {}: {e}",
                    stream.peer_addr(),
                    e.kind()
                );
            }
        }
    }
    Ok(())
}

async fn drop_peer_connection(stream: OwnedReadHalf, tx: Sender<Message>) -> Result<()> {
    let addr = stream.peer_addr()?;
    let mut peermap = PEERS.lock().await;
    peermap.remove(&addr);
    drop(stream);
    tx.send(Message {
        username: "user".to_string(),
        content: b"has left the chat.".to_vec(),
    })
    .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let usage = "Usage: server <addr:port>";
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        println!("{usage}");
        std::process::exit(1);
    }
    run_server(&args[1]).await?;
    Ok(())
}
