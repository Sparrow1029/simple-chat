use anyhow::Result;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};

use simple_chat::{sleep, Message, BUFSIZE};

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

    fn run(&mut self) -> Result<()> {
        loop {
            match self.listener.accept() {
                Ok((stream, socket_addr)) => {
                    _ = self.handle_connection(socket_addr, stream);
                }
                _ => {
                    sleep(100);
                }
            }
            self.recv_messages()?;
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
        for (_addr, stream) in self.clients.iter_mut() {
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
                    Ok(_bytes_written) => {
                        stream.flush()?;
                        dbg!(format!(
                            "Successfully wrote {_bytes_written} bytes to {addr}"
                        ));
                    }
                    Err(e) if e.kind() == ErrorKind::BrokenPipe => {
                        println!("Broken pipe. Dropping connection to peer at {addr}");
                        dropped_connections.push(*addr);
                    }
                    Err(e) => {
                        println!("Error '{}: {e}' trying to write to client stream. Adding to dropped connections.", e.kind());
                        dropped_connections.push(*addr);
                    }
                }
            }
        }
        Ok(dropped_connections)
    }
}

fn main() -> Result<()> {
    let usage = "Usage: server <addr:port>";
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        println!("{usage}");
        std::process::exit(1);
    }
    let mut server = Server::create(&args[1])?;
    dbg!("Created server {:?}", &server);
    server.run();
    Ok(())
}
