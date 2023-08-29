use anyhow::Result;
use serde::{Deserialize, Serialize};
// use std::borrow::BorrowMut;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::{
    stdin,
    BufRead,
    // ErrorKind,
    Read,
    Write,
};
use std::net::{SocketAddr, TcpListener, TcpStream};

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
                    println!("No new connections found...");
                    let wait_1s = std::time::Duration::from_secs(1);
                    std::thread::sleep(wait_1s);
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
        println!("Checking for messages...");
        for (addr, stream) in self.clients.iter_mut() {
            let mut buf: [u8; 512] = [0; 512];
            if let Ok(bytes_read) = stream.read(&mut buf) {
                if bytes_read > 0 {
                    println!("Read {bytes_read} bytes from {addr}");
                    let msg = bincode::deserialize_from(&buf[..])?;
                    println!("Recieved {msg:?}");
                    self.msg_q.borrow_mut().push_back(msg);
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
        println!("in send_messages: {}", self.msg_q.borrow().len());
        while let Some(msg) = self.msg_q.borrow_mut().pop_front() {
            for (addr, stream) in self.clients.iter_mut() {
                let encoded = bincode::serialize(&msg.content)?;
                match stream.write(&encoded) {
                    Ok(bytes_written) => {
                        println!("Successfully wrote {bytes_written} bytes to {addr}");
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
            match Client::get_content_from_stdin() {
                Some(content) => {
                    if content.len() > 1 {
                        break String::from_utf8(content)
                            .expect("error converting &[u8] to String");
                    } else {
                        continue;
                    }
                }
                None => continue,
            }
        };
        Ok(Client {
            stream,
            username: username.trim().into(),
        })
    }

    fn run(&mut self) {
        loop {
            if let Some(content) = Client::get_content_from_stdin() {
                let msg = Message {
                    username: self.username.clone(),
                    content: content.into(),
                };
                let encoded = bincode::serialize(&msg).expect("Error serializing message");
                self.send_message(&encoded).expect("error sending message");
                println!("sent message...");
            }
            self.recv_messages().expect("error receiving messages");
        }
    }

    fn send_message(&mut self, content: &[u8]) -> Result<()> {
        let msg = Message {
            username: self.username.clone(),
            content: content.into(),
        };
        let encoded = bincode::serialize(&msg)?;
        self.stream.write(&encoded)?;

        Ok(())
    }

    fn recv_messages(&mut self) -> Result<()> {
        let mut buf: [u8; 512] = [0; 512];
        match self.stream.read(&mut buf) {
            Ok(bytes_read) => {
                println!("Read bytes from stream...");
                if bytes_read > 0 {
                    let msg: Message = bincode::deserialize(&buf)?;
                    let username = msg.username.trim();
                    let content = String::from_utf8(msg.content)?;
                    println!("{username}: {content}");
                }
            }
            Err(e) => {
                println!("error encountered: {} - {e}", e.kind());
                return Ok(());
            }
        }
        Ok(())
    }

    fn get_content_from_stdin() -> Option<Vec<u8>> {
        let mut buf = String::new();
        let stdin = stdin();
        let mut handle = stdin.lock();
        let bytes_read = handle
            .read_line(&mut buf)
            .expect("error reading from stdin");
        if bytes_read > 0 {
            return Some(buf.into());
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Message {
    username: String,
    content: Vec<u8>,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = String::from("Usage: simple_chat [server <addr>] [client]");
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
