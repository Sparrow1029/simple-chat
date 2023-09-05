use anyhow::{anyhow, Result};
use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::thread;

use simple_chat::{lock, sleep, Message, BUFSIZE};

type PeerMap = HashMap<String, (SocketAddr, TcpStream)>;
/// Arc map of username -> (addr, stream)
type SharedPeerMap = Arc<Mutex<PeerMap>>;
type MsgQueue = VecDeque<Message>;
type SharedMsgQueue = Arc<Mutex<MsgQueue>>;

/// Main server loop
/// spins up a new thread for each peers 'Read' stream.
fn run_server(addr: impl ToSocketAddrs) -> Result<()> {
    let peer_map: SharedPeerMap = Arc::new(Mutex::new(HashMap::new()));
    let msg_q: SharedMsgQueue = Arc::new(Mutex::new(VecDeque::with_capacity(128)));

    let bcast_map = Arc::clone(&peer_map);
    let bcast_q = Arc::clone(&msg_q);
    thread::spawn(move || broadcast_messages(bcast_map, bcast_q));

    let listener = TcpListener::bind(addr)?;

    eprintln!("Listening for connections...");
    for s in listener.incoming() {
        if let Ok(stream) = s {
            eprintln!("Got a new connection from {}", stream.peer_addr()?);
            stream.set_nonblocking(true)?;
            let map_clone = Arc::clone(&peer_map);
            let q_clone = Arc::clone(&msg_q);
            thread::spawn(move || handle_connection(stream, map_clone, q_clone));
        }
    }
    Ok(())
}

/// Handle new peer connection
fn handle_connection(
    stream: TcpStream,
    peermap: SharedPeerMap,
    msg_q: SharedMsgQueue,
) -> Result<()> {
    // check if username is already present in the map
    //   if so, send an error message to the client

    eprintln!("Handling connection...");
    let peer_addr = stream.peer_addr()?;
    let (mut r, mut w) = (stream.try_clone()?, stream);
    let mut msg_buf = [0u8; BUFSIZE];
    if let Ok(bytes_read) = r.read(&mut msg_buf) {
        eprintln!("Got initial message! Read {bytes_read} bytes");

        let mut peermap_handle = lock!(peermap);
        let mut msg_q_handle = lock!(msg_q);

        eprintln!("Acquired locks");
        if bytes_read > 0 {
            let join_msg = Message::try_from(&msg_buf[..])?;
            if peermap_handle.contains_key(&join_msg.username) {
                let err_msg = Message {
                    username: "server".to_string(),
                    content: b"Username is already taken.".to_vec(),
                };
                w.write_all(&err_msg.serialize()?)?;
                w.flush()?;
                return Err(anyhow!("Username was already taken"));
            }

            peermap_handle.insert(join_msg.username.clone(), (peer_addr, w));
            msg_q_handle.push_back(join_msg);
            drop(peermap_handle);
            drop(msg_q_handle);

            let recv_msg_q = Arc::clone(&msg_q);
            thread::spawn(move || recv_messages(r, recv_msg_q));
        } else if bytes_read == 0 {
            // client closed connection without doing anything
            return Err(anyhow!("Client closed connection without sending anything"));
        }
    }
    Ok(())
}

/// Peer thread reading messages from stream and placing on message queue
fn recv_messages(stream: TcpStream, msg_q: SharedMsgQueue) -> Result<()> {
    let mut stream = stream;
    loop {
        let mut buf = [0u8; BUFSIZE];
        match stream.read(&mut buf) {
            Ok(bytes_read) => {
                if bytes_read > 0 {
                    let mut msg_q_handle = lock!(msg_q);
                    let message = Message::try_from(&buf[..])?;
                    msg_q_handle.push_back(message);
                } else if bytes_read == 0 {
                    // client is disconnected
                    eprintln!("Client is disconnected. Shutting down stream.");
                    stream.shutdown(std::net::Shutdown::Both)?;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
            Err(e) => eprintln!("Error reading from stream: {e} - {}", e.kind()),
        }
        sleep(500);
    }
}

/// Thread responsible for handling popping messages off the queue and
/// sending them to all connected peers
fn broadcast_messages(peermap: SharedPeerMap, msg_q: SharedMsgQueue) -> Result<()> {
    let peermap = Arc::clone(&peermap);
    let msg_q = Arc::clone(&msg_q);
    let mut to_remove: Vec<(SocketAddr, String)> = vec![];
    loop {
        let mut msg_q_handle = lock!(msg_q);
        if let Some(msg) = msg_q_handle.pop_front() {
            let mut peer_map_handle = lock!(peermap);

            let msg = msg.serialize()?;
            // send msg to all connected clients
            for (username, (addr, stream)) in peer_map_handle.iter_mut() {
                match stream.write_all(&msg) {
                    Ok(_) => stream.flush()?,
                    Err(e) if e.kind() == ErrorKind::BrokenPipe => {
                        // client disconnected
                        eprintln!("Client disconnected. Shutting down stream.");
                        match stream.shutdown(std::net::Shutdown::Both) {
                            Ok(_) => continue,
                            Err(e) => eprintln!("Err shutting broadcast stream {e} - {}", e.kind()),
                        }
                        to_remove.push((addr.clone(), username.clone()));
                    }
                    Err(e) => eprintln!("Error while writing to stream: {e} - {}", e.kind()),
                }
            }
            for (addr, username) in to_remove.iter() {
                eprintln!("Removing {username} ({addr}) from peers.");
                peer_map_handle.remove(username);
            }
            to_remove.clear();
        }
        drop(msg_q_handle);
        sleep(500);
    }
}

fn main() -> Result<()> {
    let usage = "Usage: server <address:port>";
    let args = std::env::args().collect::<Vec<String>>();
    if args.len() != 2 {
        eprintln!("{usage}");
    }
    run_server(&args[1])?;
    Ok(())
}
