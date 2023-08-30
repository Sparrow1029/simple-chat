use anyhow::Result;
use simple_chat::{Message, BUFSIZE};
use std::{
    io::{stdin, BufRead, ErrorKind, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

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

    fn run(&mut self) -> Result<()> {
        let stdin_channel = self.spawn_stdin_channel();
        loop {
            if let Some(content) = self.get_content_from_stdin_channel(&stdin_channel) {
                let msg = Message {
                    username: self.username.clone(),
                    content: content.into(),
                };
                self.send_message(msg)?;
            }
            self.recv_messages()?;
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
                if bytes_read > 0 {
                    let msg = Message::try_from(&buf[..])?;
                    let username = msg.username.trim();
                    let content = String::from_utf8(msg.content)?;
                    println!("{username}: {content}");
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

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let usage = "Usage: client <addr:port>";
    if args.len() != 2 {
        println!("{usage}");
        std::process::exit(1);
    }
    let mut client = Client::connect(&args[1])?;
    dbg!("Created client {:?}", &client);
    client.run()?;
    Ok(())
}
