use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::io;
use std::sync::{Arc, Mutex};
use threadpool::ThreadPool;
use std::time::Duration;
use super::socket_pipe::SocketPipe;

enum Message {
    Read(SocketPipe),
    Write(SocketPipe),
    ShutdownPipe(SocketPipe),
}

pub struct Server {
    listener: TcpListener,
    upstream_address: String,
    encrypt: bool,
    gpu_encrypt: bool,
    sender: Sender<Message>,
    reciever: Arc<Mutex<Receiver<Message>>>,
    pool: ThreadPool,
}

impl Server {
    pub fn new<A: ToSocketAddrs>(
        address: A,
        upstream_address: String,
        encrypt: bool,
        gpu_encrypt: bool,
        thread_count: usize,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind(address);
        let (sender, reciever) = channel();
        match listener {
            Ok(listener) => Ok(Server {
                listener: listener,
                encrypt: encrypt,
                gpu_encrypt,
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
                Ok(message) => match message {
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
                },
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
                Ok(upstream) => {
                    match SocketPipe::new(stream, upstream, self.encrypt, self.gpu_encrypt) {
                        Ok(pipe) => {
                            self.sender.send(Message::Read(pipe)).unwrap();
                        }
                        Err(e) => if let Err(ex) = downstream.shutdown(Shutdown::Both) {
                            eprintln!(
                                "连结上游服务器失败,试图关闭连接错误:{}\n{}",
                                ex, e
                            );
                        } else {
                            eprintln!("连结上游服务器失败:{}", e);
                        },
                    }
                }
                Err(e) => if let Err(ex) = stream.shutdown(Shutdown::Both) {
                    eprintln!(
                        "连结上游服务器失败,试图关闭连接错误:{}\n{}",
                        ex, e
                    );
                } else {
                    eprintln!("连结上游服务器失败:{}", e);
                },
            },
            Err(e) => if let Err(ex) = stream.shutdown(Shutdown::Both) {
                eprintln!(
                    "连结上游服务器失败,试图关闭连接错误:{}\n{}",
                    ex, e
                );
            } else {
                eprintln!("连结上游服务器失败:{}", e);
            },
        }
    }
}