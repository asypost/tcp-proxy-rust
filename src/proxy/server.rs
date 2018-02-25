use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::io::{self, Read, Write};
use std::vec::Vec;
use std::thread;
use std::sync::{Arc, Mutex};
use threadpool::ThreadPool;
use std::time::Duration;
use ocl;
use ocl::ProQue;
use ocl::flags::DeviceType;
use ocl::builders::DeviceSpecifier;

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

pub struct Crypt {
    use_gpu: bool,
    key: u8,
    gpu_program_builder: Option<ocl::builders::ProgramBuilder>,
    gpu_device: Option<ocl::Device>,
    gpu_platform: Option<ocl::Platform>,
}

impl Crypt {
    pub fn new(key: u8, use_gpu: bool) -> Self {
        let src = include_str!("crypt.cl");
        let mut platform: Option<ocl::Platform> = Option::None;
        let mut program_builder: Option<ocl::builders::ProgramBuilder> = Option::None;
        let mut device: Option<ocl::Device> = Option::None;

        if use_gpu {
            for plat in ocl::Platform::list().into_iter() {
                platform = Option::Some(plat);
                if let Ok(name) = plat.name() {
                    if name.starts_with("NVIDIA") {
                        break;
                    }
                }
            }

            if let Option::Some(platform) = platform {
                if let Ok(devices) = ocl::Device::list(platform, Option::Some(DeviceType::GPU)) {
                    if devices.len() > 0 {
                        device = Option::Some(devices[0]);
                        program_builder = Option::Some(
                            ocl::Program::builder()
                                .src(src)
                                .devices(DeviceSpecifier::Single(devices[0])),
                        );
                    }
                }
            }
        }
        Self {
            key,
            use_gpu,
            gpu_platform: platform,
            gpu_program_builder: program_builder,
            gpu_device: device,
        }
    }

    pub fn create_gpu_pro_que(&self) -> ocl::Result<ProQue> {
        if let Option::Some(ref program_builder) = self.gpu_program_builder {
            if let Option::Some(platform) = self.gpu_platform {
                if let Option::Some(device) = self.gpu_device {
                    if let Ok(context) = ocl::Context::builder()
                        .platform(platform)
                        .devices(device)
                        .build()
                    {
                        let queue = ocl::Queue::new(&context, device, Option::None)?;
                        let program = program_builder.clone().build(&context)?;
                        let pro_que = ProQue::new(
                            context,
                            queue,
                            program,
                            Option::Some(ocl::SpatialDims::One(256)),
                        );
                        return Ok(pro_que);
                    }
                }
            }
        }
        return Err(ocl::Error::from("No supported OpenCL platform found"));
    }

    pub fn gpu_available(&self) -> bool {
        match self.gpu_program_builder {
            Option::Some(_) => true,
            Option::None => false,
        }
    }
    fn gpu_encode(&self, data: &[u8]) -> ocl::Result<Vec<u8>> {
        let mut result = Vec::from(data);

        if let Option::Some(_) = self.gpu_program_builder {
            let mut pro_que = self.create_gpu_pro_que()?;
            pro_que.set_dims(data.len());
            let buffer = pro_que.create_buffer::<u8>()?;
            buffer.write(&result[..]).enq()?;
            // let buffer = ocl::Buffer::builder()
            // .queue(pro_que.queue().clone())
            // .flags(ocl::MemFlags::new().read_write().copy_host_ptr())
            // .len(data.len())
            // .host_data(&result[..])
            // .build()?;

            let kernel = pro_que
                .create_kernel("crypt")?
                .arg_scl(self.key)
                .arg_buf(&buffer);

            unsafe {
                kernel.enq()?;
            }
            buffer.read(&mut result).enq()?;
            return Ok(result);
        }
        return Err(ocl::Error::from("No supported OpenCL platform found"));
    }

    fn gpu_decode(&self, data: &[u8]) -> ocl::Result<Vec<u8>> {
        return self.gpu_encode(data);
    }

    fn cpu_encode(&self, data: &[u8]) -> Vec<u8> {
        let mut result: Vec<u8> = vec![];
        for d in data {
            result.push(d ^ self.key);
        }
        return result;
    }

    fn cpu_decode(&self, data: &[u8]) -> Vec<u8> {
        return self.cpu_encode(data);
    }

    pub fn encode(&self, data: &[u8]) -> Vec<u8> {
        if self.use_gpu && self.gpu_available() {
            match self.gpu_encode(data) {
                Ok(result) => return result,
                Err(e) => {
                    eprintln!("{}", e);
                    eprintln!("GPU crypt failed,fallback to CPU");
                    return self.cpu_encode(data);
                }
            }
        }
        return self.cpu_encode(data);
    }

    pub fn decode(&self, data: &[u8]) -> Vec<u8> {
        if self.use_gpu && self.gpu_available() {
            match self.gpu_decode(data) {
                Ok(result) => return result,
                Err(e) => {
                    eprintln!("{}", e);
                    eprintln!("GPU crypt failed,fallback to CPU");
                    return self.cpu_decode(data);
                }
            }
        }
        return self.cpu_decode(data);
    }
}

pub struct SocketPipe {
    downstream: TcpStream,
    upstream: TcpStream,
    encrypt: bool,
    crypto: Crypt,
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

    pub fn new(
        downstream: TcpStream,
        upstream: TcpStream,
        encrypt: bool,
        gpu_encrypt: bool,
    ) -> io::Result<Self> {
        match downstream.set_nonblocking(true) {
            Ok(_) => match upstream.set_nonblocking(true) {
                Ok(_) => Ok(SocketPipe {
                    downstream,
                    upstream,
                    encrypt,
                    crypto: Crypt::new(0xee, gpu_encrypt),
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
                    true => self.upstream
                        .write_all(self.crypto.decode(&buffer[..size]).as_slice()),
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
                        .write_all(self.crypto.encode(&buffer[..size]).as_slice()),
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
