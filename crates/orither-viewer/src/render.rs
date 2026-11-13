//! Draws video with the GPU: the three planes as textures, joined in a shader.

use std::sync::Arc;

use anyhow::{Context, Result};
use winit::{dpi::PhysicalSize, window::Window};

use crate::{control::Position, decode::Frame};

const SHADER: &str = r#"
struct Corner {
    @builtin(position) position: vec4f,
    @location(0) place: vec2f,
}

@vertex
fn corner(@builtin(vertex_index) index: u32) -> Corner {
    let place = vec2f(f32(index & 1u) * 2.0, f32(index & 2u));
    return Corner(vec4f(place.x * 2.0 - 1.0, 1.0 - place.y * 2.0, 0.0, 1.0), place);
}

@group(0) @binding(0) var luma: texture_2d<f32>;
@group(0) @binding(1) var blue: texture_2d<f32>;
@group(0) @binding(2) var red: texture_2d<f32>;
@group(0) @binding(3) var smooth_sampler: sampler;

// Limited range BT.709, which is what Android's encoders send.
@fragment
fn pixel(in: Corner) -> @location(0) vec4f {
    let y = (textureSample(luma, smooth_sampler, in.place).r - 16.0 / 255.0) * 1.164;
    let u = textureSample(blue, smooth_sampler, in.place).r - 0.5;
    let v = textureSample(red, smooth_sampler, in.place).r - 0.5;
    return vec4f(y + 1.793 * v, y - 0.213 * u - 0.533 * v, y + 2.112 * u, 1.0);
}
"#;

/// The window's own background, `#171717` as everywhere in Orither.
const BACKGROUND: wgpu::Color = wgpu::Color { r: 0.0902, g: 0.0902, b: 0.0902, a: 1.0 };

/// Where the video sits inside the window, in window pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewport {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    video: (u32, u32),
}

impl Viewport {
    /// Fits a video into a window, keeping its shape.
    pub fn fit(window: PhysicalSize<u32>, video: (u32, u32)) -> Self {
        let (outer_width, outer_height) = (f64::from(window.width), f64::from(window.height));
        if video.0 == 0 || video.1 == 0 {
            return Self { width: outer_width, height: outer_height, ..Self::default() };
        }
        let scale = (outer_width / f64::from(video.0)).min(outer_height / f64::from(video.1));
        let (width, height) = (f64::from(video.0) * scale, f64::from(video.1) * scale);
        Self { x: (outer_width - width) / 2.0, y: (outer_height - height) / 2.0, width, height, video }
    }

    /// Where on the video a point in the window falls, if it falls on it.
    pub fn locate(&self, x: f64, y: f64) -> Option<Position> {
        let (across, down) = ((x - self.x) / self.width, (y - self.y) / self.height);
        let inside = self.video.0 > 0 && (0.0..1.0).contains(&across) && (0.0..1.0).contains(&down);
        inside.then(|| Position {
            x: (across * f64::from(self.video.0)) as i32,
            y: (down * f64::from(self.video.1)) as i32,
            width: self.video.0 as u16,
            height: self.video.1 as u16,
        })
    }
}

struct Pictures {
    size: (u32, u32),
    planes: [wgpu::Texture; 3],
    binding: wgpu::BindGroup,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pictures: Option<Pictures>,
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Result<Self> {
        pollster::block_on(Self::create(window))
    }

    async fn create(window: Arc<Window>) -> Result<Self> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).context("creating the surface")?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await
            .context("finding a GPU")?;
        let (device, queue) = adapter.request_device(&Default::default()).await.context("opening the GPU")?;

        // The video is already gamma encoded, so an sRGB surface would do it twice.
        let abilities = surface.get_capabilities(&adapter);
        let format = abilities.formats.iter().copied().find(|format| !format.is_srgb()).unwrap_or(abilities.formats[0]);
        let present_mode = if abilities.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 1,
            alpha_mode: abilities.alpha_modes[0],
            color_space: wgpu::SurfaceColorSpace::default(),
            view_formats: Vec::new(),
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("video"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let plane = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("video"),
            entries: &[
                plane(0),
                plane(1),
                plane(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("video"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("video"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("corner"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("pixel"),
                compilation_options: Default::default(),
                targets: &[Some(format.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Ok(Self { surface, device, queue, config, pipeline, layout, sampler, pictures: None })
    }

    pub fn resize(&mut self, size: PhysicalSize<u32>) {
        self.config.width = size.width.max(1);
        self.config.height = size.height.max(1);
        self.surface.configure(&self.device, &self.config);
    }

    fn pictures_for(&mut self, size: (u32, u32)) -> &Pictures {
        if self.pictures.as_ref().is_none_or(|pictures| pictures.size != size) {
            let plane = |width: u32, height: u32| {
                self.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("video plane"),
                    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::R8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            };
            let (half_width, half_height) = (size.0.div_ceil(2), size.1.div_ceil(2));
            let planes = [plane(size.0, size.1), plane(half_width, half_height), plane(half_width, half_height)];
            let views = planes.each_ref().map(|plane| plane.create_view(&Default::default()));
            let binding = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("video"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&views[0]) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[1]) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&views[2]) },
                    wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                ],
            });
            self.pictures = Some(Pictures { size, planes, binding });
        }
        self.pictures.as_ref().expect("just made")
    }

    pub fn upload(&mut self, frame: &Frame) {
        self.pictures_for((frame.width, frame.height));
        let pictures = self.pictures.as_ref().expect("just made");
        for (texture, plane) in pictures.planes.iter().zip(&frame.planes) {
            self.queue.write_texture(
                texture.as_image_copy(),
                &plane.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(plane.stride as u32),
                    rows_per_image: None,
                },
                texture.size(),
            );
        }
    }

    pub fn draw(&mut self, viewport: Viewport) -> Result<()> {
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(output) | wgpu::CurrentSurfaceTexture::Suboptimal(output) => output,
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            _ => return Ok(()),
        };
        let view = output.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("video"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(BACKGROUND),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            if let Some(pictures) = &self.pictures {
                pass.set_viewport(
                    viewport.x as f32,
                    viewport.y as f32,
                    viewport.width as f32,
                    viewport.height as f32,
                    0.0,
                    1.0,
                );
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &pictures.binding, &[]);
                pass.draw(0..3, 0..1);
            }
        }
        self.queue.submit([encoder.finish()]);
        self.queue.present(output);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tall_video_sits_centered_in_a_wide_window() {
        let viewport = Viewport::fit(PhysicalSize::new(1000, 500), (500, 1000));
        let center = viewport.locate(500.0, 250.0).unwrap();
        assert_eq!((center.x, center.y), (250, 500));
        assert_eq!(viewport.locate(100.0, 250.0), None);
    }

    #[test]
    fn nothing_can_be_touched_before_the_video_starts() {
        let viewport = Viewport::fit(PhysicalSize::new(1000, 500), (0, 0));
        assert_eq!(viewport.locate(100.0, 100.0), None);
    }
}
