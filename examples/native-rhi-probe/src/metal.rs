use std::time::Instant;

use objc2_metal::{
    MTLClearColor, MTLCommandBuffer as _, MTLCommandEncoder as _, MTLCommandQueue as _,
    MTLCreateSystemDefaultDevice, MTLDevice as _, MTLLoadAction, MTLPixelFormat,
    MTLRenderPassDescriptor, MTLStorageMode, MTLStoreAction, MTLTextureDescriptor, MTLTextureUsage,
};

use crate::{ProbeBackend, Sample, elapsed_ms};

pub struct MetalProbe {
    adapter_name: String,
    queue: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLCommandQueue>>,
    target: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLTexture>>,
}

impl MetalProbe {
    pub fn new(width: u32, height: u32) -> Self {
        let device = MTLCreateSystemDefaultDevice().expect("Metal probe must find a device");
        let queue = device
            .newCommandQueue()
            .expect("Metal probe must create a command queue");
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                width as usize,
                height as usize,
                false,
            )
        };
        descriptor.setStorageMode(MTLStorageMode::Private);
        descriptor.setUsage(MTLTextureUsage::RenderTarget);
        let target = device
            .newTextureWithDescriptor(&descriptor)
            .expect("Metal probe must create a render target");
        Self {
            adapter_name: device.name().to_string(),
            queue,
            target,
        }
    }
}

impl ProbeBackend for MetalProbe {
    fn name(&self) -> &'static str {
        "native-metal"
    }

    fn adapter_name(&self) -> String {
        self.adapter_name.clone()
    }

    fn sample(&mut self, pass_count: usize) -> Sample {
        let encode_started = Instant::now();
        let command_buffer = self
            .queue
            .commandBuffer()
            .expect("Metal probe must create a command buffer");
        for pass_index in 0..pass_count {
            let descriptor = MTLRenderPassDescriptor::new();
            let attachment = unsafe { descriptor.colorAttachments().objectAtIndexedSubscript(0) };
            attachment.setTexture(Some(&self.target));
            attachment.setLoadAction(MTLLoadAction::Clear);
            attachment.setStoreAction(MTLStoreAction::Store);
            attachment.setClearColor(MTLClearColor {
                red: 0.05 + pass_index as f64 * 0.001,
                green: 0.10,
                blue: 0.15,
                alpha: 1.0,
            });
            let encoder = command_buffer
                .renderCommandEncoderWithDescriptor(&descriptor)
                .expect("Metal probe must create a render encoder");
            encoder.endEncoding();
        }
        let encode_ms = elapsed_ms(encode_started);

        let submit_started = Instant::now();
        command_buffer.commit();
        command_buffer.waitUntilCompleted();
        if let Some(error) = command_buffer.error() {
            panic!("Metal probe command failed: {error}");
        }
        Sample {
            encode_ms,
            submit_wait_ms: elapsed_ms(submit_started),
        }
    }
}
