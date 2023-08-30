use anyhow::Result;
use serde::{Deserialize, Serialize};
// use std::borrow::BorrowMut;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::{stdin, BufRead, ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::{thread, time};

const BUFSIZE: usize = 8 * 1024; // 8 KiB

#[derive(Debug)]
struct Server {
    clients: HashMap<SocketAddr, TcpStream>,
    listener: TcpListener,
    msg_q: RefCell<VecDeque<Message>>,
}

impl Server {
    fn create(addr: &str) -> Result<Self> {
        let socket_addr: SocketAddr = addr.parse()?;
        let listener = TcpListener::bind(socket_addr)?;
        listener.set_nonblocking(true)?;
        let clients = HashMap::new();
        let msg_q = RefCell::new(VecDeque::new());

        Ok(Server {
            listener,
            clients,
            msg_q,
        })
    }

    fn run(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((stream, socket_addr)) => {
                    _ = self.handle_connection(socket_addr, stream);
                }
                _ => {
                    sleep(100);
                }
            }
            _ = self.recv_messages();
            let keys_to_drop = self.send_messages().expect("error sending messages");
            self.drop_connections(&keys_to_drop);
        }
    }

    fn handle_connection(&mut self, addr: SocketAddr, stream: TcpStream) -> Result<()> {
        stream.set_nonblocking(true)?;
        let client_addr = stream.peer_addr()?;
        println!("New connection from {client_addr}");
        self.clients.insert(addr, stream);
        Ok(())
    }

    fn drop_connections(&mut self, keys_to_drop: &[SocketAddr]) {
        for key in keys_to_drop {
            println!("Dropping connection to {key}");
            self.clients.remove(key);
        }
    }

    fn recv_messages(&mut self) -> Result<()> {
        // println!("Checking for messages...");
        for (addr, stream) in self.clients.iter_mut() {
            let mut buf: [u8; BUFSIZE] = [0; BUFSIZE];
            if let Ok(bytes_read) = stream.read(&mut buf) {
                if bytes_read > 0 {
                    // println!("Read {bytes_read} bytes from {addr}");
                    let msg = Message::try_from(&buf[..])?;
                    // prinln!("Recieved {msg:?}");
                    self.msg_q.borrow_mut().push_back(msg);
                    stream.flush()?;
                }
            }
        }
        Ok(())
    }

    fn send_messages(&mut self) -> Result<Vec<SocketAddr>> {
        let mut dropped_connections = vec![];

        // Have to access self.msg_q through RefCell (interior mutability)
        // to allow borrowing fields on self mutably more than once.
        // It should be safe because we are mutating two _separate_ fields of `self`
        // println!("in send_messages: {}", self.msg_q.borrow().len());
        while let Some(msg) = self.msg_q.borrow_mut().pop_front() {
            for (addr, stream) in self.clients.iter_mut() {
                let encoded = msg.serialize()?;
                match stream.write(&encoded) {
                    Ok(bytes_written) => {
                        stream.flush()?;
                        // println!("Successfully wrote {bytes_written} bytes to {addr}");
                    }
                    Err(e) => {
                        println!("Error {e} trying to write to client stream. Adding to dropped connections.");
                        dropped_connections.push(*addr);
                    }
                }
            }
        }
        Ok(dropped_connections)
    }
}

#[derive(Debug)]
struct Client {
    username: String,
    stream: TcpStream,
}

impl Client {
    fn connect(addr: &str) -> Result<Client> {
        let socket: SocketAddr = addr.parse()?;
        let stream = TcpStream::connect(socket)?;
        let timeout = std::time::Duration::from_millis(100);
        stream.set_nonblocking(true)?;
        _ = stream.set_write_timeout(Some(timeout));
        let username = loop {
            println!("Enter a username: ");
            let mut buf = String::new();
            let mut handle = stdin().lock();
            match handle.read_line(&mut buf) {
                Ok(bytes_read) => {
                    if bytes_read > 1 {
                        break buf;
                    } else {
                        continue;
                    }
                }
                _ => continue,
            }
        };
        Ok(Client {
            stream,
            username: username.trim().into(),
        })
    }

    fn run(&mut self) {
        let stdin_channel = self.spawn_stdin_channel();
        loop {
            // eprintln!("Checking for new stdin...");
            if let Some(content) = self.get_content_from_stdin_channel(&stdin_channel) {
                let msg = Message {
                    username: self.username.clone(),
                    content: content.into(),
                };
                self.send_message(msg).expect("error sending message");
                // println!("sent message...");
            }
            // eprintln!("Waiting to receive messages...");
            self.recv_messages().expect("error receiving messages");
        }
    }

    fn send_message(&mut self, msg: Message) -> Result<()> {
        let encoded = msg.serialize()?;
        self.stream.write(&encoded)?;
        self.stream.flush()?;

        Ok(())
    }

    fn recv_messages(&mut self) -> Result<()> {
        let mut buf: [u8; BUFSIZE] = [0; BUFSIZE];
        match self.stream.read(&mut buf) {
            Ok(bytes_read) => {
                // println!("Read bytes from stream...");
                if bytes_read > 0 {
                    let msg = Message::try_from(&buf[..])?;
                    // eprintln!("MESSAGE: {:?}", &msg);

                    let username = msg.username.trim();
                    let content = String::from_utf8(msg.content)?;
                    println!("{username}: {content}");

                    // self.stream.flush()?;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                return Ok(());
            }
            Err(e) => {
                println!("error encountered: {} - {e}", e.kind());
            }
        }
        Ok(())
    }

    fn spawn_stdin_channel(&self) -> Receiver<String> {
        let (tx, rx) = mpsc::channel::<String>();
        thread::spawn(move || loop {
            let mut buffer = String::new();
            stdin().read_line(&mut buffer).unwrap();
            tx.send(buffer).unwrap();
        });
        rx
    }

    fn get_content_from_stdin_channel(&self, rx: &Receiver<String>) -> Option<Vec<u8>> {
        match rx.try_recv() {
            Ok(content) => {
                // println!("Received: {}", key);
                return Some(content.trim().into());
            }
            Err(TryRecvError::Empty) => {} //println!("Channel empty"),
            Err(TryRecvError::Disconnected) => panic!("Channel disconnected"),
        }
        // sleep(1000);
        None
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        eprintln!("Shutting down tcpstream");
        _ = self.stream.shutdown(Shutdown::Both);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Message {
    username: String,
    content: Vec<u8>,
}

impl Message {
    fn serialize(&self) -> bincode::Result<Vec<u8>> {
        bincode::serialize(self)
    }
}

impl<'a> TryFrom<&'a [u8]> for Message {
    type Error = Box<bincode::ErrorKind>;

    fn try_from(value: &'a [u8]) -> bincode::Result<Self> {
        bincode::deserialize(value)
    }
}

fn sleep(millis: u64) {
    let duration = time::Duration::from_millis(millis);
    thread::sleep(duration);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = String::from("Usage: simple_chat [server <addr:port>] [client <addr:port>]");
    match args.len() {
        1 | 2 => println!("{usage}"),
        3 => {
            let arg1 = args[1].as_str();
            match arg1 {
                "client" => {
                    let mut client = Client::connect(&args[2]).expect("error creating client");
                    println!("Created client {:?}", &client);
                    client.run();
                }
                "server" => {
                    let server = Server::create(&args[2]);
                    println!("Created server {:?}", &server);
                    _ = server.unwrap().run();
                }
                _ => {
                    println!("{usage}")
                }
            }
        }
        _ => println!("{usage}"),
    }
}
