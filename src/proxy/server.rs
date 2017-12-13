use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::io::{self, Read, Write};
use std::vec::Vec;
use std::thread;
use std::sync::{Arc, Mutex};
use threadpool::ThreadPool;
use std::time::Duration;

fn encode(data: &[u8]) -> Vec<u8> {
    let mut result: Vec<u8> = vec![];
    let key = 0xee;
    for d in data {
        result.push(d ^ key);
    }
    return result;
}

fn decode(data: &[u8]) -> Vec<u8> {
    return encode(data);
}


pub struct Server {
    listener: TcpListener,
    upstream_address: String,
    encrypt: bool,
    sender: Sender<Message>,
    reciever: Arc<Mutex<Receiver<Message>>>,
    pool: ThreadPool,
}

impl Server {
    pub fn new<A: ToSocketAddrs>(
        address: A,
        upstream_address: String,
        encrypt: bool,
        thread_count: usize,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(address);
        let (sender, reciever) = channel();
        match listener {
            Ok(listener) => Ok(Server {
                listener: listener,
                encrypt: encrypt,
                upstream_address,
                sender,
                reciever: Arc::new(Mutex::new(reciever)),
                pool: ThreadPool::new(thread_count),
            }),
            Err(e) => Err(e),
        }
    }

    pub fn run(&mut self) {
        let reciever = self.reciever.clone();
        let sender = self.sender.clone();
        let pool = self.pool.clone();
        let handler = thread::spawn(move || loop {
            let rx = reciever.lock().unwrap();
            match rx.recv() {
                Ok(message) => {
                    match message {
                        Message::Read(mut pipe) => {
                            let sender = sender.clone();
                            pool.execute(move || match pipe.read() {
                                Ok(_) => {
                                    sender.send(Message::Write(pipe)).unwrap();
                                }
                                Err(e) => match e.kind() {
                                    io::ErrorKind::WouldBlock => {
                                        thread::sleep(Duration::from_millis(8));
                                        sender.send(Message::Write(pipe)).unwrap();
                                    }
                                    _ => {
                                        sender.send(Message::ShutdownPipe(pipe)).unwrap();
                                    }
                                },
                            })
                        }
                        Message::Write(mut pipe) => {
                            let sender = sender.clone();
                            pool.execute(move || match pipe.write() {
                                Ok(_) => {
                                    sender.send(Message::Read(pipe)).unwrap();
                                }
                                Err(e) => match e.kind() {
                                    io::ErrorKind::WouldBlock => {
                                        thread::sleep(Duration::from_millis(8));
                                        sender.send(Message::Read(pipe)).unwrap();
                                    }
                                    _ => {
                                        sender.send(Message::ShutdownPipe(pipe)).unwrap();
                                    }
                                },
                            })
                        }
                        Message::ShutdownPipe(pipe) => {
                            drop(pipe);
                        }
                    }
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(1));
                }
            }
        });
        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => {
                    self.handle_client(stream);
                }
                Err(e) => {
                    eprintln!("接受客户端连接失败:{}", e);
                }
            }
        }
        handler.join().unwrap();
        self.pool.join();
    }

    fn handle_client(&self, stream: TcpStream) {
        let address = self.upstream_address.clone();
        match stream.try_clone() {
            Ok(downstream) => match TcpStream::connect(address) {
                Ok(upstream) => match SocketPipe::new(stream, upstream, self.encrypt) {
                    Ok(pipe) => {
                        self.sender.send(Message::Read(pipe)).unwrap();
                    }
                    Err(e) => if let Err(ex) = downstream.shutdown(Shutdown::Both) {
                        eprintln!(
                            "连结上游服务器失败,试图关闭连接错误:{}\n{}",
                            ex,
                            e
                        );
                    } else {
                        eprintln!("连结上游服务器失败:{}", e);
                    },
                },
                Err(e) => if let Err(ex) = stream.shutdown(Shutdown::Both) {
                    eprintln!(
                        "连结上游服务器失败,试图关闭连接错误:{}\n{}",
                        ex,
                        e
                    );
                } else {
                    eprintln!("连结上游服务器失败:{}", e);
                },
            },
            Err(e) => if let Err(ex) = stream.shutdown(Shutdown::Both) {
                eprintln!(
                    "连结上游服务器失败,试图关闭连接错误:{}\n{}",
                    ex,
                    e
                );
            } else {
                eprintln!("连结上游服务器失败:{}", e);
            },
        }
    }
}

pub struct SocketPipe {
    downstream: TcpStream,
    upstream: TcpStream,
    encrypt: bool,
}

enum Message {
    Read(SocketPipe),
    Write(SocketPipe),
    ShutdownPipe(SocketPipe),
}

impl Drop for SocketPipe {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl SocketPipe {
    const BUFFER_SIZE: usize = 4 * 1024; //缓存大小4KB

    pub fn new(downstream: TcpStream, upstream: TcpStream, encrypt: bool) -> io::Result<Self> {
        match downstream.set_nonblocking(true) {
            Ok(_) => match upstream.set_nonblocking(true) {
                Ok(_) => Ok(SocketPipe {
                    downstream,
                    upstream,
                    encrypt,
                }),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        }
    }

    #[inline]
    pub fn read(&mut self) -> io::Result<()> {
        let mut buffer = [0; Self::BUFFER_SIZE];
        match self.downstream.read(&mut buffer) {
            Ok(size) => if size > 0 {
                match self.encrypt {
                    true => self.upstream.write_all(decode(&buffer[..size]).as_slice()),
                    false => self.upstream.write_all(&buffer[..size]),
                }
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "没有读取到数据",
                ))
            },
            Err(e) => Err(e),
        }
    }

    #[inline]
    pub fn write(&mut self) -> io::Result<()> {
        let mut buffer = [0; Self::BUFFER_SIZE];
        match self.upstream.read(&mut buffer) {
            Ok(size) => if size > 0 {
                match self.encrypt {
                    true => self.downstream
                        .write_all(encode(&buffer[..size]).as_slice()),
                    false => self.downstream.write_all(&buffer[..size]),
                }
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "没有读取到数据",
                ))
            },
            Err(e) => Err(e),
        }
    }

    pub fn shutdown(&self) {
        if let Err(e) = self.downstream.shutdown(Shutdown::Both) {
            eprintln!("关闭客户端连接失败:{}", e);
        }
        if let Err(e) = self.upstream.shutdown(Shutdown::Both) {
            eprintln!("关闭上游服务器连接失败:{}", e);
        }
    }
}
