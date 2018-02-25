extern crate num_cpus;
extern crate threadpool;
extern crate ocl;

mod proxy;
use proxy::Server;
use std::env;
use std::vec::Vec;

fn main() {
    let arguments: Vec<String> = env::args().collect();
    let count = arguments.len();
    if count < 3 {
        println!("使用方法如下:");
        println!(
            "proxy-server.exe [监听地址(格式为IP:Port)] [源服务器地址(格式为IP:Port)] [是否加密(可选,true表示加密)] [是否使用GPU(true表示使用)]"
        );
    } else if count <= 5 {
        let address = &arguments[1];
        let source = &arguments[2];
        let mut encrypt = false;
        let mut gpu_encrypt =false;
        if count >= 4 {
            encrypt = arguments[3] == "true";
        }
        if count == 5{
            gpu_encrypt =arguments[4] == "true"
        }
        let thread_count = num_cpus::get() * 2 + 1;
        let mut server = Server::new(
            address.trim().to_string(),
            source.trim().to_string(),
            encrypt,
            gpu_encrypt,
            thread_count,
        ).unwrap();
        let encrypt_mode = match encrypt {
            true => "ON",
            false => "OFF",
        };
        let use_gpu_mode = match gpu_encrypt{
            true => "ON",
            false => "OFF",
        };
        println!(
            "启动服务:线程数{},加密模式——[{}],GPU模式——[{}]",
            thread_count,
            encrypt_mode,
            use_gpu_mode
        );
        server.run();
    } else {
        println!("参数过多，使用方法如下:");
        println!(
            "proxy-server.exe [监听地址(格式为IP:Port)] [源服务器地址(格式为IP:Port)] [是否加密(可选,true表示加密)] [是否使用GPU(true表示使用)]"
        );
    }
}
