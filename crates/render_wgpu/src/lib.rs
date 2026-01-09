//! wgpu-based renderer for the vector editor.
//!
//! Uses lyon for path tessellation, fontdue for text rasterization, and wgpu for GPU rendering.

use std::collections::HashMap;

use editor_render::{
    DisplayItem, DisplayList, LineCap, LineJoin, Paint, PathCmd, PathData, Stroke, TextItem,
};
use fontdue::{Font, FontSettings};
use glam::{Affine2, Vec2};
use lyon::geom::point;
use lyon::path::PathEvent;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, StrokeOptions, StrokeTessellator,
    StrokeVertex, VertexBuffers,
};
use wgpu::util::DeviceExt;

/// Vertex data for GPU rendering (colored shapes).
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

impl Vertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Vertex data for textured rendering (glyphs).
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TextVertex {
    pub position: [f32; 2],
    pub tex_coords: [f32; 2],
    pub color: [f32; 4],
}

impl TextVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 3] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TextVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Cached glyph information in the atlas.
#[derive(Clone, Copy, Debug)]
struct CachedGlyph {
    /// UV coordinates in the atlas (min_u, min_v, max_u, max_v)
    uv: [f32; 4],
    /// Glyph metrics
    width: f32,
    height: f32,
    advance_width: f32,
    bearing_x: f32,
    bearing_y: f32,
}

/// Glyph atlas for caching rasterized glyphs.
struct GlyphAtlas {
    /// Cached glyphs by (font_family, char, size_bucket)
    glyphs: HashMap<(String, char, u32), CachedGlyph>,
    /// Atlas texture data (single channel)
    data: Vec<u8>,
    /// Current packing position
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    /// Atlas dimensions
    width: u32,
    height: u32,
    /// Whether the texture needs to be re-uploaded
    dirty: bool,
}

impl GlyphAtlas {
    fn new(width: u32, height: u32) -> Self {
        Self {
            glyphs: HashMap::new(),
            data: vec![0; (width * height) as usize],
            cursor_x: 1, // Start at 1 to avoid edge artifacts
            cursor_y: 1,
            row_height: 0,
            width,
            height,
            dirty: true,
        }
    }

    /// Get or create a cached glyph.
    fn get_or_insert(
        &mut self,
        font: &Font,
        font_family: &str,
        c: char,
        font_size: f32,
    ) -> Option<CachedGlyph> {
        // Bucket font sizes to reduce atlas fragmentation
        let size_bucket = (font_size * 2.0) as u32;
        let key = (font_family.to_string(), c, size_bucket);

        if let Some(&glyph) = self.glyphs.get(&key) {
            return Some(glyph);
        }

        // Rasterize the glyph
        let (metrics, bitmap) = font.rasterize(c, font_size);

        if metrics.width == 0 || metrics.height == 0 {
            // Space or empty glyph
            let glyph = CachedGlyph {
                uv: [0.0, 0.0, 0.0, 0.0],
                width: 0.0,
                height: 0.0,
                advance_width: metrics.advance_width,
                bearing_x: 0.0,
                bearing_y: 0.0,
            };
            self.glyphs.insert(key, glyph);
            return Some(glyph);
        }

        let glyph_width = metrics.width as u32;
        let glyph_height = metrics.height as u32;

        // Check if we need to move to next row
        if self.cursor_x + glyph_width + 1 > self.width {
            self.cursor_x = 1;
            self.cursor_y += self.row_height + 1;
            self.row_height = 0;
        }

        // Check if atlas is full
        if self.cursor_y + glyph_height + 1 > self.height {
            log::warn!("Glyph atlas is full, cannot add more glyphs");
            return None;
        }

        // Copy bitmap to atlas
        for y in 0..glyph_height {
            for x in 0..glyph_width {
                let src_idx = (y * glyph_width + x) as usize;
                let dst_idx = ((self.cursor_y + y) * self.width + self.cursor_x + x) as usize;
                self.data[dst_idx] = bitmap[src_idx];
            }
        }

        // Calculate UV coordinates
        let u0 = self.cursor_x as f32 / self.width as f32;
        let v0 = self.cursor_y as f32 / self.height as f32;
        let u1 = (self.cursor_x + glyph_width) as f32 / self.width as f32;
        let v1 = (self.cursor_y + glyph_height) as f32 / self.height as f32;

        let glyph = CachedGlyph {
            uv: [u0, v0, u1, v1],
            width: glyph_width as f32,
            height: glyph_height as f32,
            advance_width: metrics.advance_width,
            bearing_x: metrics.xmin as f32,
            bearing_y: metrics.ymin as f32,
        };

        // Update cursor
        self.cursor_x += glyph_width + 1;
        self.row_height = self.row_height.max(glyph_height);
        self.dirty = true;

        self.glyphs.insert(key, glyph);
        Some(glyph)
    }

    fn is_dirty(&self) -> bool {
        self.dirty
    }

    fn mark_clean(&mut self) {
        self.dirty = false;
    }

    fn data(&self) -> &[u8] {
        &self.data
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Cache for loaded fonts.
struct FontCache {
    fonts: HashMap<String, Font>,
    default_font: Option<String>,
}

impl FontCache {
    fn new() -> Self {
        Self {
            fonts: HashMap::new(),
            default_font: None,
        }
    }

    /// Load a font from bytes.
    fn load_font(&mut self, name: &str, data: &[u8]) -> bool {
        match Font::from_bytes(data, FontSettings::default()) {
            Ok(font) => {
                if self.default_font.is_none() {
                    self.default_font = Some(name.to_string());
                }
                self.fonts.insert(name.to_string(), font);
                true
            }
            Err(e) => {
                log::error!("Failed to load font '{}': {}", name, e);
                false
            }
        }
    }

    /// Get a font by family name, falling back to default.
    fn get_font(&self, family: &str) -> Option<&Font> {
        self.fonts
            .get(family)
            .or_else(|| self.default_font.as_ref().and_then(|d| self.fonts.get(d)))
    }

    /// Get the default font.
    #[allow(dead_code)]
    fn default_font(&self) -> Option<&Font> {
        self.default_font.as_ref().and_then(|d| self.fonts.get(d))
    }

    fn default_font_name(&self) -> Option<&str> {
        self.default_font.as_deref()
    }
}

/// The wgpu renderer.
pub struct WgpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    shape_pipeline: wgpu::RenderPipeline,
    text_pipeline: wgpu::RenderPipeline,
    text_bind_group_layout: wgpu::BindGroupLayout,
    text_bind_group: Option<wgpu::BindGroup>,
    atlas_texture: Option<wgpu::Texture>,
    surface_config: wgpu::SurfaceConfiguration,
    surface: wgpu::Surface<'static>,
    fill_tessellator: FillTessellator,
    stroke_tessellator: StrokeTessellator,
    font_cache: FontCache,
    glyph_atlas: GlyphAtlas,
}

impl WgpuRenderer {
    /// Create a new wgpu renderer.
    pub async fn new(
        window: std::sync::Arc<winit::window::Window>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let size = window.inner_size();

        // Create wgpu instance
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        // Create surface
        let surface = instance.create_surface(window)?;

        // Request adapter
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or("Failed to find adapter")?;

        // Request device
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    label: None,
                    memory_hints: Default::default(),
                },
                None,
            )
            .await?;

        // Configure surface
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        // Create shape shader and pipeline
        let shape_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shape Shader"),
            source: wgpu::ShaderSource::Wgsl(SHAPE_SHADER.into()),
        });

        let shape_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Shape Pipeline Layout"),
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });

        let shape_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Shape Pipeline"),
            layout: Some(&shape_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shape_shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shape_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Create text shader and pipeline
        let text_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Text Shader"),
            source: wgpu::ShaderSource::Wgsl(TEXT_SHADER.into()),
        });

        let text_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Text Bind Group Layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let text_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Text Pipeline Layout"),
            bind_group_layouts: &[&text_bind_group_layout],
            push_constant_ranges: &[],
        });

        let text_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Text Pipeline"),
            layout: Some(&text_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &text_shader,
                entry_point: Some("vs_main"),
                buffers: &[TextVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &text_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let mut renderer = Self {
            device,
            queue,
            shape_pipeline,
            text_pipeline,
            text_bind_group_layout,
            text_bind_group: None,
            atlas_texture: None,
            surface_config,
            surface,
            fill_tessellator: FillTessellator::new(),
            stroke_tessellator: StrokeTessellator::new(),
            font_cache: FontCache::new(),
            glyph_atlas: GlyphAtlas::new(1024, 1024),
        };

        // Load default font
        renderer.load_default_font();

        Ok(renderer)
    }

    /// Load a default font for text rendering.
    fn load_default_font(&mut self) {
        // Try to load system fonts
        let font_paths = if cfg!(target_os = "macos") {
            vec![
                "/System/Library/Fonts/SFNS.ttf",
                "/System/Library/Fonts/Helvetica.ttc",
                "/Library/Fonts/Arial.ttf",
                "/System/Library/Fonts/Supplemental/Arial.ttf",
            ]
        } else if cfg!(target_os = "windows") {
            vec![
                "C:\\Windows\\Fonts\\arial.ttf",
                "C:\\Windows\\Fonts\\segoeui.ttf",
                "C:\\Windows\\Fonts\\tahoma.ttf",
            ]
        } else {
            // Linux
            vec![
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "/usr/share/fonts/TTF/DejaVuSans.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
            ]
        };

        for path in font_paths {
            if let Ok(data) = std::fs::read(path) {
                if self.load_font("sans-serif", &data) {
                    log::info!("Loaded default font from: {}", path);
                    return;
                }
            }
        }

        log::warn!("Could not load any system font, text will render as rectangles");
    }

    /// Load a font for text rendering.
    pub fn load_font(&mut self, name: &str, data: &[u8]) -> bool {
        self.font_cache.load_font(name, data)
    }

    /// Resize the renderer surface.
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.surface_config.width = new_size.width;
            self.surface_config.height = new_size.height;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    /// Get the current surface size.
    pub fn size(&self) -> (u32, u32) {
        (self.surface_config.width, self.surface_config.height)
    }

    /// Update the glyph atlas texture if needed.
    fn update_atlas_texture(&mut self) {
        if !self.glyph_atlas.is_dirty() && self.atlas_texture.is_some() {
            return;
        }

        let (width, height) = self.glyph_atlas.dimensions();

        // Create or recreate texture
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Upload texture data
        self.queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            self.glyph_atlas.data(),
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = self.device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Text Bind Group"),
            layout: &self.text_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        self.atlas_texture = Some(texture);
        self.text_bind_group = Some(bind_group);
        self.glyph_atlas.mark_clean();
    }

    /// Render a display list.
    pub fn render(&mut self, display_list: &DisplayList) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Tessellate shapes and text
        let (shape_vertices, shape_indices) = self.tessellate_shapes(display_list);
        let (text_vertices, text_indices) = self.tessellate_text(display_list);

        // Update atlas texture if needed
        self.update_atlas_texture();

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.95,
                            g: 0.95,
                            b: 0.95,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            // Render shapes
            if !shape_indices.is_empty() {
                let vertex_buffer =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Shape Vertex Buffer"),
                            contents: bytemuck::cast_slice(&shape_vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        });

                let index_buffer =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Shape Index Buffer"),
                            contents: bytemuck::cast_slice(&shape_indices),
                            usage: wgpu::BufferUsages::INDEX,
                        });

                render_pass.set_pipeline(&self.shape_pipeline);
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..shape_indices.len() as u32, 0, 0..1);
            }

            // Render text
            if !text_indices.is_empty() {
                if let Some(ref bind_group) = self.text_bind_group {
                    let vertex_buffer =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("Text Vertex Buffer"),
                                contents: bytemuck::cast_slice(&text_vertices),
                                usage: wgpu::BufferUsages::VERTEX,
                            });

                    let index_buffer =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("Text Index Buffer"),
                                contents: bytemuck::cast_slice(&text_indices),
                                usage: wgpu::BufferUsages::INDEX,
                            });

                    render_pass.set_pipeline(&self.text_pipeline);
                    render_pass.set_bind_group(0, bind_group, &[]);
                    render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                    render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    render_pass.draw_indexed(0..text_indices.len() as u32, 0, 0..1);
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    /// Render the canvas display list with a UI overlay.
    pub fn render_with_ui(
        &mut self,
        display_list: &DisplayList,
        ui_draw_list: &ui::DrawList,
    ) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // Tessellate canvas shapes and text
        let (shape_vertices, shape_indices) = self.tessellate_shapes(display_list);
        let (text_vertices, text_indices) = self.tessellate_text(display_list);

        // Tessellate UI
        let (ui_shape_vertices, ui_shape_indices) = self.tessellate_ui_shapes(ui_draw_list);
        let (ui_text_vertices, ui_text_indices) = self.tessellate_ui_text(ui_draw_list);

        // Update atlas texture if needed
        self.update_atlas_texture();

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.95,
                            g: 0.95,
                            b: 0.95,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            // Render canvas shapes
            if !shape_indices.is_empty() {
                let vertex_buffer =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Shape Vertex Buffer"),
                            contents: bytemuck::cast_slice(&shape_vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        });

                let index_buffer =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Shape Index Buffer"),
                            contents: bytemuck::cast_slice(&shape_indices),
                            usage: wgpu::BufferUsages::INDEX,
                        });

                render_pass.set_pipeline(&self.shape_pipeline);
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..shape_indices.len() as u32, 0, 0..1);
            }

            // Render canvas text
            if !text_indices.is_empty() {
                if let Some(ref bind_group) = self.text_bind_group {
                    let vertex_buffer =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("Text Vertex Buffer"),
                                contents: bytemuck::cast_slice(&text_vertices),
                                usage: wgpu::BufferUsages::VERTEX,
                            });

                    let index_buffer =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("Text Index Buffer"),
                                contents: bytemuck::cast_slice(&text_indices),
                                usage: wgpu::BufferUsages::INDEX,
                            });

                    render_pass.set_pipeline(&self.text_pipeline);
                    render_pass.set_bind_group(0, bind_group, &[]);
                    render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                    render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    render_pass.draw_indexed(0..text_indices.len() as u32, 0, 0..1);
                }
            }

            // Render UI shapes (on top of canvas)
            if !ui_shape_indices.is_empty() {
                let vertex_buffer =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("UI Shape Vertex Buffer"),
                            contents: bytemuck::cast_slice(&ui_shape_vertices),
                            usage: wgpu::BufferUsages::VERTEX,
                        });

                let index_buffer =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("UI Shape Index Buffer"),
                            contents: bytemuck::cast_slice(&ui_shape_indices),
                            usage: wgpu::BufferUsages::INDEX,
                        });

                render_pass.set_pipeline(&self.shape_pipeline);
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..ui_shape_indices.len() as u32, 0, 0..1);
            }

            // Render UI text
            if !ui_text_indices.is_empty() {
                if let Some(ref bind_group) = self.text_bind_group {
                    let vertex_buffer =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("UI Text Vertex Buffer"),
                                contents: bytemuck::cast_slice(&ui_text_vertices),
                                usage: wgpu::BufferUsages::VERTEX,
                            });

                    let index_buffer =
                        self.device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("UI Text Index Buffer"),
                                contents: bytemuck::cast_slice(&ui_text_indices),
                                usage: wgpu::BufferUsages::INDEX,
                            });

                    render_pass.set_pipeline(&self.text_pipeline);
                    render_pass.set_bind_group(0, bind_group, &[]);
                    render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                    render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                    render_pass.draw_indexed(0..ui_text_indices.len() as u32, 0, 0..1);
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    /// Tessellate UI shapes from DrawList.
    fn tessellate_ui_shapes(&mut self, draw_list: &ui::DrawList) -> (Vec<Vertex>, Vec<u32>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let (width, height) = self.size();

        for command in &draw_list.commands {
            match command {
                ui::DrawCommand::FillRect {
                    rect,
                    color,
                    corner_radius,
                } => {
                    self.add_ui_rect(
                        &mut vertices,
                        &mut indices,
                        rect,
                        color,
                        *corner_radius,
                        width,
                        height,
                    );
                }
                ui::DrawCommand::StrokeRect {
                    rect,
                    color,
                    width: stroke_width,
                    corner_radius: _,
                } => {
                    // Simple stroke as 4 rectangles
                    let sw = *stroke_width;
                    // Top
                    self.add_ui_rect(
                        &mut vertices,
                        &mut indices,
                        &ui::Rect::new(rect.x, rect.y, rect.width, sw),
                        color,
                        0.0,
                        width,
                        height,
                    );
                    // Bottom
                    self.add_ui_rect(
                        &mut vertices,
                        &mut indices,
                        &ui::Rect::new(rect.x, rect.y + rect.height - sw, rect.width, sw),
                        color,
                        0.0,
                        width,
                        height,
                    );
                    // Left
                    self.add_ui_rect(
                        &mut vertices,
                        &mut indices,
                        &ui::Rect::new(rect.x, rect.y + sw, sw, rect.height - 2.0 * sw),
                        color,
                        0.0,
                        width,
                        height,
                    );
                    // Right
                    self.add_ui_rect(
                        &mut vertices,
                        &mut indices,
                        &ui::Rect::new(
                            rect.x + rect.width - sw,
                            rect.y + sw,
                            sw,
                            rect.height - 2.0 * sw,
                        ),
                        color,
                        0.0,
                        width,
                        height,
                    );
                }
                ui::DrawCommand::Icon { kind, rect, color } => {
                    self.add_ui_icon(&mut vertices, &mut indices, *kind, rect, color, width, height);
                }
                _ => {}
            }
        }

        (vertices, indices)
    }

    /// Tessellate UI text from DrawList.
    fn tessellate_ui_text(&mut self, draw_list: &ui::DrawList) -> (Vec<TextVertex>, Vec<u32>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let (width, height) = self.size();

        for command in &draw_list.commands {
            if let ui::DrawCommand::Text { text, rect, style, .. } = command {
                self.add_ui_text(
                    &mut vertices,
                    &mut indices,
                    text,
                    rect,
                    style,
                    width,
                    height,
                );
            }
        }

        (vertices, indices)
    }

    /// Add a UI rectangle to the vertex buffer.
    fn add_ui_rect(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        rect: &ui::Rect,
        color: &ui::Color,
        _corner_radius: f32,
        width: u32,
        height: u32,
    ) {
        let base = vertices.len() as u32;

        // Convert to NDC
        let x1 = (rect.x / width as f32) * 2.0 - 1.0;
        let y1 = 1.0 - (rect.y / height as f32) * 2.0;
        let x2 = ((rect.x + rect.width) / width as f32) * 2.0 - 1.0;
        let y2 = 1.0 - ((rect.y + rect.height) / height as f32) * 2.0;

        let c = color.to_array();

        vertices.extend_from_slice(&[
            Vertex { position: [x1, y1], color: c },
            Vertex { position: [x2, y1], color: c },
            Vertex { position: [x2, y2], color: c },
            Vertex { position: [x1, y2], color: c },
        ]);

        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Add UI text to the vertex buffer.
    fn add_ui_text(
        &mut self,
        vertices: &mut Vec<TextVertex>,
        indices: &mut Vec<u32>,
        text: &str,
        rect: &ui::Rect,
        style: &ui::TextStyle,
        width: u32,
        height: u32,
    ) {
        let color = style.color.to_array();
        let font_size = style.font_size;
        let char_width = font_size * 0.6;
        let line_height = font_size * 1.4;

        // Get font from cache
        let font_family = self
            .font_cache
            .default_font_name()
            .unwrap_or("sans-serif")
            .to_string();

        // We need to get a raw pointer to avoid borrowing issues
        // This is safe because font_cache is not modified during glyph_atlas operations
        let font_ptr = self.font_cache.get_font(&font_family).map(|f| f as *const Font);

        let Some(font_ptr) = font_ptr else {
            // No font available - use fallback rectangles
            return;
        };

        // Center text vertically in rect
        let y_offset = (rect.height - line_height) / 2.0;
        let mut x = rect.x;
        let y = rect.y + y_offset;

        for c in text.chars() {
            if c == ' ' {
                x += char_width;
                continue;
            }

            // SAFETY: font_ptr points to font_cache data which is not modified during this operation
            let font = unsafe { &*font_ptr };

            // Get or create glyph
            let glyph = self.glyph_atlas.get_or_insert(font, &font_family, c, font_size);

            if let Some(glyph) = glyph {
                let base = vertices.len() as u32;

                let gx = x + glyph.bearing_x;
                let gy = y + (line_height - glyph.bearing_y);
                let gw = glyph.width;
                let gh = glyph.height;

                // Convert to NDC
                let x1 = (gx / width as f32) * 2.0 - 1.0;
                let y1 = 1.0 - (gy / height as f32) * 2.0;
                let x2 = ((gx + gw) / width as f32) * 2.0 - 1.0;
                let y2 = 1.0 - ((gy + gh) / height as f32) * 2.0;

                let [u1, v1, u2, v2] = glyph.uv;

                vertices.extend_from_slice(&[
                    TextVertex { position: [x1, y1], tex_coords: [u1, v1], color },
                    TextVertex { position: [x2, y1], tex_coords: [u2, v1], color },
                    TextVertex { position: [x2, y2], tex_coords: [u2, v2], color },
                    TextVertex { position: [x1, y2], tex_coords: [u1, v2], color },
                ]);

                indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);

                x += glyph.advance_width;
            } else {
                x += char_width;
            }
        }
    }

    /// Add a UI icon to the vertex buffer.
    fn add_ui_icon(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        kind: ui::IconKind,
        rect: &ui::Rect,
        color: &ui::Color,
        width: u32,
        height: u32,
    ) {
        let c = color.to_array();

        // Simple icon rendering as basic shapes
        match kind {
            ui::IconKind::ChevronRight => {
                // Right-pointing triangle
                let cx = rect.x + rect.width / 2.0;
                let cy = rect.y + rect.height / 2.0;
                let size = rect.width.min(rect.height) * 0.4;

                let base = vertices.len() as u32;

                let p1 = self.to_ndc(cx - size * 0.3, cy - size, width, height);
                let p2 = self.to_ndc(cx + size * 0.5, cy, width, height);
                let p3 = self.to_ndc(cx - size * 0.3, cy + size, width, height);

                vertices.extend_from_slice(&[
                    Vertex { position: p1, color: c },
                    Vertex { position: p2, color: c },
                    Vertex { position: p3, color: c },
                ]);

                indices.extend_from_slice(&[base, base + 1, base + 2]);
            }
            ui::IconKind::ChevronDown => {
                // Down-pointing triangle
                let cx = rect.x + rect.width / 2.0;
                let cy = rect.y + rect.height / 2.0;
                let size = rect.width.min(rect.height) * 0.4;

                let base = vertices.len() as u32;

                let p1 = self.to_ndc(cx - size, cy - size * 0.3, width, height);
                let p2 = self.to_ndc(cx + size, cy - size * 0.3, width, height);
                let p3 = self.to_ndc(cx, cy + size * 0.5, width, height);

                vertices.extend_from_slice(&[
                    Vertex { position: p1, color: c },
                    Vertex { position: p2, color: c },
                    Vertex { position: p3, color: c },
                ]);

                indices.extend_from_slice(&[base, base + 1, base + 2]);
            }
            ui::IconKind::Eye | ui::IconKind::EyeOff => {
                // Simple eye as a circle
                let cx = rect.x + rect.width / 2.0;
                let cy = rect.y + rect.height / 2.0;
                let r = rect.width.min(rect.height) * 0.3;

                self.add_circle(vertices, indices, cx, cy, r, &c, width, height);
            }
            ui::IconKind::Rectangle | ui::IconKind::Frame => {
                // Simple square outline
                let inset = rect.width.min(rect.height) * 0.2;
                let inner = ui::Rect::new(
                    rect.x + inset,
                    rect.y + inset,
                    rect.width - 2.0 * inset,
                    rect.height - 2.0 * inset,
                );
                self.add_ui_rect(vertices, indices, &inner, color, 0.0, width, height);
            }
            ui::IconKind::Folder | ui::IconKind::Group => {
                // Folder shape
                let inset = rect.width.min(rect.height) * 0.15;
                let inner = ui::Rect::new(
                    rect.x + inset,
                    rect.y + inset * 1.5,
                    rect.width - 2.0 * inset,
                    rect.height - 2.0 * inset,
                );
                self.add_ui_rect(vertices, indices, &inner, color, 0.0, width, height);
            }
            _ => {
                // Default: small filled square
                let inset = rect.width.min(rect.height) * 0.25;
                let inner = ui::Rect::new(
                    rect.x + inset,
                    rect.y + inset,
                    rect.width - 2.0 * inset,
                    rect.height - 2.0 * inset,
                );
                self.add_ui_rect(vertices, indices, &inner, color, 0.0, width, height);
            }
        }
    }

    /// Add a circle to the vertex buffer.
    fn add_circle(
        &self,
        vertices: &mut Vec<Vertex>,
        indices: &mut Vec<u32>,
        cx: f32,
        cy: f32,
        r: f32,
        color: &[f32; 4],
        width: u32,
        height: u32,
    ) {
        let segments = 16;
        let base = vertices.len() as u32;

        // Center vertex
        let center = self.to_ndc(cx, cy, width, height);
        vertices.push(Vertex { position: center, color: *color });

        // Perimeter vertices
        for i in 0..segments {
            let angle = (i as f32 / segments as f32) * std::f32::consts::TAU;
            let px = cx + r * angle.cos();
            let py = cy + r * angle.sin();
            let p = self.to_ndc(px, py, width, height);
            vertices.push(Vertex { position: p, color: *color });
        }

        // Triangle fan indices
        for i in 0..segments {
            let next = (i + 1) % segments;
            indices.extend_from_slice(&[base, base + 1 + i as u32, base + 1 + next as u32]);
        }
    }

    /// Convert screen coordinates to NDC.
    fn to_ndc(&self, x: f32, y: f32, width: u32, height: u32) -> [f32; 2] {
        [
            (x / width as f32) * 2.0 - 1.0,
            1.0 - (y / height as f32) * 2.0,
        ]
    }

    /// Tessellate shapes (fills and strokes) into vertices and indices.
    fn tessellate_shapes(&mut self, display_list: &DisplayList) -> (Vec<Vertex>, Vec<u32>) {
        let mut geometry: VertexBuffers<Vertex, u32> = VertexBuffers::new();
        let (width, height) = self.size();

        for item in &display_list.items {
            match item {
                DisplayItem::FillPath {
                    path,
                    paint,
                    transform,
                    opacity,
                } => {
                    self.tessellate_fill(
                        path,
                        paint,
                        transform,
                        *opacity,
                        &mut geometry,
                        width,
                        height,
                    );
                }
                DisplayItem::StrokePath {
                    path,
                    stroke,
                    transform,
                    opacity,
                } => {
                    self.tessellate_stroke(
                        path,
                        stroke,
                        transform,
                        *opacity,
                        &mut geometry,
                        width,
                        height,
                    );
                }
                DisplayItem::Text { .. } => {
                    // Text is handled separately
                }
                DisplayItem::BeginClip { .. } | DisplayItem::EndClip => {
                    // Clipping not yet implemented in wgpu renderer
                    // Would require stencil buffer support
                }
                DisplayItem::SnapGuide { start, end, .. } => {
                    // Render snap guide as a line
                    let color = [1.0, 0.0, 1.0, 0.8]; // Magenta
                    let width_f = width as f32;
                    let height_f = height as f32;

                    // Convert to NDC
                    let to_ndc = |p: &Vec2| {
                        [
                            (p.x / width_f) * 2.0 - 1.0,
                            1.0 - (p.y / height_f) * 2.0,
                        ]
                    };

                    let base_index = geometry.vertices.len() as u32;
                    let thickness = 1.0;

                    // Create a thin quad for the line
                    let dx = end.x - start.x;
                    let dy = end.y - start.y;
                    let len = (dx * dx + dy * dy).sqrt();
                    if len > 0.0 {
                        let nx = -dy / len * thickness;
                        let ny = dx / len * thickness;

                        let p0 = Vec2::new(start.x + nx, start.y + ny);
                        let p1 = Vec2::new(start.x - nx, start.y - ny);
                        let p2 = Vec2::new(end.x + nx, end.y + ny);
                        let p3 = Vec2::new(end.x - nx, end.y - ny);

                        geometry.vertices.push(Vertex {
                            position: to_ndc(&p0),
                            color,
                        });
                        geometry.vertices.push(Vertex {
                            position: to_ndc(&p1),
                            color,
                        });
                        geometry.vertices.push(Vertex {
                            position: to_ndc(&p2),
                            color,
                        });
                        geometry.vertices.push(Vertex {
                            position: to_ndc(&p3),
                            color,
                        });

                        geometry.indices.push(base_index);
                        geometry.indices.push(base_index + 1);
                        geometry.indices.push(base_index + 2);
                        geometry.indices.push(base_index + 1);
                        geometry.indices.push(base_index + 3);
                        geometry.indices.push(base_index + 2);
                    }
                }
                DisplayItem::SelectionRect { min, max } => {
                    // Render selection rectangle as 4 line segments
                    let color = [0.0, 0.4, 1.0, 1.0]; // Blue
                    let width_f = width as f32;
                    let height_f = height as f32;
                    let thickness = 1.0;

                    let to_ndc = |p: &Vec2| {
                        [
                            (p.x / width_f) * 2.0 - 1.0,
                            1.0 - (p.y / height_f) * 2.0,
                        ]
                    };

                    // Helper to draw a line segment as a quad
                    let mut draw_line = |start: Vec2, end: Vec2| {
                        let base_index = geometry.vertices.len() as u32;
                        let dx = end.x - start.x;
                        let dy = end.y - start.y;
                        let len = (dx * dx + dy * dy).sqrt();
                        if len > 0.0 {
                            let nx = -dy / len * thickness;
                            let ny = dx / len * thickness;

                            let p0 = Vec2::new(start.x + nx, start.y + ny);
                            let p1 = Vec2::new(start.x - nx, start.y - ny);
                            let p2 = Vec2::new(end.x + nx, end.y + ny);
                            let p3 = Vec2::new(end.x - nx, end.y - ny);

                            geometry.vertices.push(Vertex {
                                position: to_ndc(&p0),
                                color,
                            });
                            geometry.vertices.push(Vertex {
                                position: to_ndc(&p1),
                                color,
                            });
                            geometry.vertices.push(Vertex {
                                position: to_ndc(&p2),
                                color,
                            });
                            geometry.vertices.push(Vertex {
                                position: to_ndc(&p3),
                                color,
                            });

                            geometry.indices.push(base_index);
                            geometry.indices.push(base_index + 1);
                            geometry.indices.push(base_index + 2);
                            geometry.indices.push(base_index + 1);
                            geometry.indices.push(base_index + 3);
                            geometry.indices.push(base_index + 2);
                        }
                    };

                    // Draw 4 edges of the rectangle
                    let top_left = *min;
                    let top_right = Vec2::new(max.x, min.y);
                    let bottom_right = *max;
                    let bottom_left = Vec2::new(min.x, max.y);

                    draw_line(top_left, top_right);       // Top
                    draw_line(top_right, bottom_right);   // Right
                    draw_line(bottom_right, bottom_left); // Bottom
                    draw_line(bottom_left, top_left);     // Left
                }
            }
        }

        (geometry.vertices, geometry.indices)
    }

    /// Tessellate text into vertices and indices.
    fn tessellate_text(&mut self, display_list: &DisplayList) -> (Vec<TextVertex>, Vec<u32>) {
        let mut vertices: Vec<TextVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let (width, height) = self.size();
        let width_f = width as f32;
        let height_f = height as f32;

        for item in &display_list.items {
            if let DisplayItem::Text {
                text,
                transform,
                opacity,
            } = item
            {
                let color = paint_to_color(&text.fill, *opacity);

                // Get font
                let font = match self.font_cache.get_font(&text.font_family) {
                    Some(f) => f,
                    None => {
                        // Fallback: render rectangles
                        self.tessellate_text_fallback(
                            text,
                            transform,
                            color,
                            &mut vertices,
                            &mut indices,
                            width_f,
                            height_f,
                        );
                        continue;
                    }
                };

                let font_family = self
                    .font_cache
                    .default_font_name()
                    .unwrap_or("sans-serif")
                    .to_string();

                // Render each character
                let mut x = 0.0f32;
                let baseline_y = text.font_size * 0.8; // Approximate baseline

                for c in text.content.chars() {
                    // Get or create cached glyph
                    let glyph =
                        match self
                            .glyph_atlas
                            .get_or_insert(font, &font_family, c, text.font_size)
                        {
                            Some(g) => g,
                            None => continue,
                        };

                    if glyph.width > 0.0 && glyph.height > 0.0 {
                        // Calculate glyph position
                        let glyph_x = x + glyph.bearing_x;
                        let glyph_y = baseline_y - glyph.bearing_y - glyph.height;

                        // Transform corners
                        let p0 = transform.transform_point2(Vec2::new(glyph_x, glyph_y));
                        let p1 =
                            transform.transform_point2(Vec2::new(glyph_x + glyph.width, glyph_y));
                        let p2 =
                            transform.transform_point2(Vec2::new(glyph_x, glyph_y + glyph.height));
                        let p3 = transform.transform_point2(Vec2::new(
                            glyph_x + glyph.width,
                            glyph_y + glyph.height,
                        ));

                        // Convert to NDC
                        let to_ndc =
                            |p: Vec2| [(p.x / width_f) * 2.0 - 1.0, 1.0 - (p.y / height_f) * 2.0];

                        let base_index = vertices.len() as u32;

                        // Add vertices with UV coordinates
                        vertices.push(TextVertex {
                            position: to_ndc(p0),
                            tex_coords: [glyph.uv[0], glyph.uv[1]],
                            color,
                        });
                        vertices.push(TextVertex {
                            position: to_ndc(p1),
                            tex_coords: [glyph.uv[2], glyph.uv[1]],
                            color,
                        });
                        vertices.push(TextVertex {
                            position: to_ndc(p2),
                            tex_coords: [glyph.uv[0], glyph.uv[3]],
                            color,
                        });
                        vertices.push(TextVertex {
                            position: to_ndc(p3),
                            tex_coords: [glyph.uv[2], glyph.uv[3]],
                            color,
                        });

                        // Add indices for two triangles
                        indices.push(base_index);
                        indices.push(base_index + 1);
                        indices.push(base_index + 2);
                        indices.push(base_index + 1);
                        indices.push(base_index + 3);
                        indices.push(base_index + 2);
                    }

                    x += glyph.advance_width;
                }
            }
        }

        (vertices, indices)
    }

    /// Fallback text rendering when no font is available.
    #[allow(clippy::too_many_arguments)]
    fn tessellate_text_fallback(
        &self,
        text: &TextItem,
        transform: &Affine2,
        color: [f32; 4],
        vertices: &mut Vec<TextVertex>,
        indices: &mut Vec<u32>,
        width_f: f32,
        height_f: f32,
    ) {
        let char_width = text.font_size * 0.6;
        let char_height = text.font_size;
        let mut x = 0.0;

        for _ in text.content.chars() {
            let p0 = transform.transform_point2(Vec2::new(x, 0.0));
            let p1 = transform.transform_point2(Vec2::new(x + char_width * 0.8, 0.0));
            let p2 = transform.transform_point2(Vec2::new(x, char_height));
            let p3 = transform.transform_point2(Vec2::new(x + char_width * 0.8, char_height));

            let to_ndc = |p: Vec2| [(p.x / width_f) * 2.0 - 1.0, 1.0 - (p.y / height_f) * 2.0];

            let base_index = vertices.len() as u32;

            // Use UV coordinates that map to a blank area (0,0)
            vertices.push(TextVertex {
                position: to_ndc(p0),
                tex_coords: [0.0, 0.0],
                color,
            });
            vertices.push(TextVertex {
                position: to_ndc(p1),
                tex_coords: [0.0, 0.0],
                color,
            });
            vertices.push(TextVertex {
                position: to_ndc(p2),
                tex_coords: [0.0, 0.0],
                color,
            });
            vertices.push(TextVertex {
                position: to_ndc(p3),
                tex_coords: [0.0, 0.0],
                color,
            });

            indices.push(base_index);
            indices.push(base_index + 1);
            indices.push(base_index + 2);
            indices.push(base_index + 1);
            indices.push(base_index + 3);
            indices.push(base_index + 2);

            x += char_width;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn tessellate_fill(
        &mut self,
        path: &PathData,
        paint: &Paint,
        transform: &Affine2,
        opacity: f32,
        geometry: &mut VertexBuffers<Vertex, u32>,
        width: u32,
        height: u32,
    ) {
        let color = paint_to_color(paint, opacity);
        let events = path_to_events(path, transform, width, height);

        let _ = self.fill_tessellator.tessellate(
            events,
            &FillOptions::default(),
            &mut BuffersBuilder::new(geometry, |vertex: FillVertex| Vertex {
                position: vertex.position().to_array(),
                color,
            }),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn tessellate_stroke(
        &mut self,
        path: &PathData,
        stroke: &Stroke,
        transform: &Affine2,
        opacity: f32,
        geometry: &mut VertexBuffers<Vertex, u32>,
        width: u32,
        height: u32,
    ) {
        let color = paint_to_color(&stroke.paint, opacity);
        let events = path_to_events(path, transform, width, height);

        let line_cap = match stroke.line_cap {
            LineCap::Butt => lyon::tessellation::LineCap::Butt,
            LineCap::Round => lyon::tessellation::LineCap::Round,
            LineCap::Square => lyon::tessellation::LineCap::Square,
        };

        let line_join = match stroke.line_join {
            LineJoin::Miter => lyon::tessellation::LineJoin::Miter,
            LineJoin::Round => lyon::tessellation::LineJoin::Round,
            LineJoin::Bevel => lyon::tessellation::LineJoin::Bevel,
        };

        let options = StrokeOptions::default()
            .with_line_width(stroke.width)
            .with_start_cap(line_cap)
            .with_end_cap(line_cap)
            .with_line_join(line_join);

        let _ = self.stroke_tessellator.tessellate(
            events,
            &options,
            &mut BuffersBuilder::new(geometry, |vertex: StrokeVertex| Vertex {
                position: vertex.position().to_array(),
                color,
            }),
        );
    }
}

/// Convert a Paint to a color array.
fn paint_to_color(paint: &Paint, opacity: f32) -> [f32; 4] {
    match paint {
        Paint::Solid([r, g, b, a]) => [*r, *g, *b, *a * opacity],
    }
}

/// Convert a PathData to lyon path events, applying transform and converting to NDC.
fn path_to_events<'a>(
    path: &'a PathData,
    transform: &'a Affine2,
    width: u32,
    height: u32,
) -> impl Iterator<Item = PathEvent> + 'a {
    let width = width as f32;
    let height = height as f32;

    // Convert from screen coordinates to normalized device coordinates (-1 to 1)
    let to_ndc = move |p: Vec2| -> lyon::geom::Point<f32> {
        let transformed = transform.transform_point2(p);
        let x = (transformed.x / width) * 2.0 - 1.0;
        let y = 1.0 - (transformed.y / height) * 2.0; // Flip Y
        point(x, y)
    };

    let mut start_point = None;
    let mut current_point = point(0.0, 0.0);

    path.commands.iter().map(move |cmd| match cmd {
        PathCmd::MoveTo(p) => {
            let pt = to_ndc(*p);
            start_point = Some(pt);
            current_point = pt;
            PathEvent::Begin { at: pt }
        }
        PathCmd::LineTo(p) => {
            let from = current_point;
            let to = to_ndc(*p);
            current_point = to;
            PathEvent::Line { from, to }
        }
        PathCmd::CubicTo { c1, c2, p } => {
            let from = current_point;
            let ctrl1 = to_ndc(*c1);
            let ctrl2 = to_ndc(*c2);
            let to = to_ndc(*p);
            current_point = to;
            PathEvent::Cubic {
                from,
                ctrl1,
                ctrl2,
                to,
            }
        }
        PathCmd::Close => {
            let last = current_point;
            let first = start_point.unwrap_or(point(0.0, 0.0));
            current_point = first;
            PathEvent::End {
                last,
                first,
                close: true,
            }
        }
    })
}

/// WGSL shader for rendering colored shapes.
const SHAPE_SHADER: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

/// WGSL shader for rendering textured glyphs.
const TEXT_SHADER: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@group(0) @binding(0)
var t_glyph: texture_2d<f32>;
@group(0) @binding(1)
var s_glyph: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(in.position, 0.0, 1.0);
    out.tex_coords = in.tex_coords;
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let alpha = textureSample(t_glyph, s_glyph, in.tex_coords).r;
    return vec4<f32>(in.color.rgb, in.color.a * alpha);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vertex_layout() {
        let vertex = Vertex {
            position: [0.0, 0.0],
            color: [1.0, 0.0, 0.0, 1.0],
        };
        assert_eq!(std::mem::size_of::<Vertex>(), 24); // 2 floats + 4 floats = 6 * 4 = 24 bytes
        assert_eq!(vertex.position, [0.0, 0.0]);
    }

    #[test]
    fn test_text_vertex_layout() {
        let vertex = TextVertex {
            position: [0.0, 0.0],
            tex_coords: [0.0, 0.0],
            color: [1.0, 0.0, 0.0, 1.0],
        };
        // 2 + 2 + 4 = 8 floats = 32 bytes
        assert_eq!(std::mem::size_of::<TextVertex>(), 32);
        assert_eq!(vertex.position, [0.0, 0.0]);
    }

    #[test]
    fn test_paint_to_color() {
        let paint = Paint::Solid([1.0, 0.5, 0.25, 1.0]);
        let color = paint_to_color(&paint, 0.5);
        assert_eq!(color, [1.0, 0.5, 0.25, 0.5]);
    }

    #[test]
    fn test_glyph_atlas_new() {
        let atlas = GlyphAtlas::new(512, 512);
        assert_eq!(atlas.dimensions(), (512, 512));
        assert!(atlas.is_dirty());
    }
}
