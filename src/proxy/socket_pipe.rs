use super::crypt::Crypt;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};

pub struct SocketPipe {
    downstream: TcpStream,
    upstream: TcpStream,
    encrypt: bool,
    crypto: Crypt,
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