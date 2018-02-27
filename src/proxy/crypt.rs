use ocl;
use ocl::ProQue;
use ocl::flags::DeviceType;
use ocl::builders::DeviceSpecifier;
use std::vec::Vec;

pub struct Crypt {
    use_gpu: bool,
    key: u8,
    gpu_pro_que_builder: Option<ocl::builders::ProQueBuilder>,
}

impl Crypt {
    pub fn new(key: u8, use_gpu: bool) -> Self {
        let mut gpu_pro_que_builder: Option<ocl::builders::ProQueBuilder> = Option::None;
        if use_gpu {
            gpu_pro_que_builder = match Self::create_gpu_pro_que_builder() {
                Ok(que) => Option::Some(que),
                Err(_) => Option::None,
            };
        }
        Self {
            key,
            use_gpu,
            gpu_pro_que_builder,
        }
    }

    fn create_gpu_pro_que_builder() -> ocl::Result<ocl::builders::ProQueBuilder> {
        let mut platform: Option<ocl::Platform> = Option::None;
        let src = include_str!("crypt.cl");

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
                    let mut pro_que_builder = ProQue::builder();
                    pro_que_builder
                        .device(DeviceSpecifier::Single(devices[0]))
                        .platform(platform)
                        .src(src);
                    return Ok(pro_que_builder);
                }
            }
        }
        return Err(ocl::Error::from("No supported OpenCL platform found"));
    }

    pub fn gpu_available(&self) -> bool {
        match self.gpu_pro_que_builder {
            Option::Some(_) => true,
            Option::None => false,
        }
    }

    #[inline]
    fn gpu_encode(&mut self, data: &[u8]) -> ocl::Result<Vec<u8>> {
        let mut result = Vec::from(data);

        if let Some(ref mut pro_que_builder) = self.gpu_pro_que_builder {
            let mut pro_que = pro_que_builder.dims(data.len()).build()?;
            // let buffer = pro_que.create_buffer::<u8>()?;
            // buffer.write(&result[..]).enq()?;
            let buffer = ocl::Buffer::builder()
                .queue(pro_que.queue().clone())
                .flags(ocl::MemFlags::new().read_write().use_host_ptr())
                .len(data.len())
                .host_data(&result[..])
                .build()?;

            let mut kernel = pro_que
                .create_kernel("crypt")?
                .arg_scl(self.key)
                .arg_buf_named("data", Option::Some(&buffer));

            unsafe {
                kernel.enq()?;
            }
            buffer.read(&mut result).enq()?;
            return Ok(result);
        }

        return Err(ocl::Error::from("No supported OpenCL platform found"));
    }

    #[inline]
    fn gpu_decode(&mut self, data: &[u8]) -> ocl::Result<Vec<u8>> {
        return self.gpu_encode(data);
    }

    #[inline]
    fn cpu_encode(&self, data: &[u8]) -> Vec<u8> {
        let mut result: Vec<u8> = vec![];
        for d in data {
            result.push(d ^ self.key);
        }
        return result;
    }

    #[inline]
    fn cpu_decode(&self, data: &[u8]) -> Vec<u8> {
        return self.cpu_encode(data);
    }

    pub fn encode(&mut self, data: &[u8]) -> Vec<u8> {
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

    pub fn decode(&mut self, data: &[u8]) -> Vec<u8> {
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
