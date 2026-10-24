//! Draws egui with Direct3D 11 into a flip-model swap chain.
//!
//! Direct3D 11 rather than wgpu on Direct3D 12, measured on a Radeon 780M (GPU memory is system
//! RAM): Avdeck at rest took 146 MB of private bytes with wgpu, 31 MB with this; streaming a phone
//! screen, 173 MB and ~40% of a core against 58 MB and ~10%. The 12 driver also kept a thread
//! spinning while nothing was drawn. A UI draws a few dozen batches a frame, so Direct3D 12's
//! lower cost per draw buys nothing.
//!
//! The pipeline is egui's own (the shader and blending of egui-wgpu): textures and vertex colors
//! in gamma space, premultiplied alpha, a non-sRGB target, dithering on.

use egui::epaint::{ClippedPrimitive, ImageData, ImageDelta, Primitive, Vertex};
use egui::{TextureFilter, TextureId, TextureOptions, TextureWrapMode, TexturesDelta};
use std::collections::HashMap;
use windows::Win32::Foundation::{HANDLE, HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCOMPILE_ENABLE_STRICTNESS, D3DCompile};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
    D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
    DXGI_FORMAT_R32_UINT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT,
    DXGI_SWAP_EFFECT_FLIP_DISCARD, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIDevice, IDXGIFactory2,
    IDXGISwapChain2,
};
use windows::Win32::System::Threading::WaitForSingleObjectEx;
use windows::core::{Interface, PCSTR, s};

/// Largest texture egui may ask for (what egui-wgpu allowed, to keep the same font atlas).
pub const MAX_TEXTURE_SIDE: usize = 8192;

/// Frames queued ahead of the screen: one, the app paces itself (`app::FRAME_TIME`).
const FRAME_LATENCY: u32 = 1;
const SWAP_CHAIN_FLAGS: DXGI_SWAP_CHAIN_FLAG = DXGI_SWAP_CHAIN_FLAG_FRAME_LATENCY_WAITABLE_OBJECT;

/// egui-wgpu's `egui.wgsl` (`vs_main`, `fs_main_gamma_framebuffer`) as naga translates it to HLSL
/// for wgpu's Direct3D 12 backend, minus the unused paths: the same arithmetic, so the same
/// pixels (decoding the color in the shader rather than as UNORM matters to the last bit).
const SHADER: &str = r#"
struct Locals {
    float2 screen_size; // in points
    uint dithering;
    uint padding;
};
cbuffer r_locals : register(b0) { Locals r_locals; }
Texture2D<float4> r_tex_color : register(t0);
SamplerState r_tex_sampler : register(s0);

struct VertexOutput {
    float2 tex_coord : LOC0;
    float4 color : LOC1; // gamma 0-1
    float4 position : SV_Position;
};

// Jimenez 2014, "Next Generation Post-Processing in Call of Duty".
float interleaved_gradient_noise(float2 n)
{
    float f = ((0.06711056 * n.x) + (0.00583715 * n.y));
    return frac((52.982918 * frac(f)));
}

float3 dither_interleaved(float3 rgb, float levels, float4 frag_coord)
{
    float noise_ = interleaved_gradient_noise(frag_coord.xy);
    // Scaled down so that flat colors do not get dithered.
    noise_ = ((noise_ - 0.5) * 0.95);
    return (rgb + ((noise_ / (levels - 1.0))).xxx);
}

// sRGBA bytes packed in a u32 -> 0-1.
float4 unpack_color(uint color)
{
    return (float4(float((color & 255u)), float(((color >> 8u) & 255u)), float(((color >> 16u) & 255u)), float(((color >> 24u) & 255u))) / (255.0).xxxx);
}

float4 position_from_screen(float2 screen_pos)
{
    float width = r_locals.screen_size.x;
    float height = r_locals.screen_size.y;
    return float4((((2.0 * screen_pos.x) / width) - 1.0), (1.0 - ((2.0 * screen_pos.y) / height)), 0.0, 1.0);
}

VertexOutput vs_main(float2 a_pos : LOC0, float2 a_tex_coord : LOC1, uint a_color : LOC2)
{
    VertexOutput out_ = (VertexOutput)0;
    out_.tex_coord = a_tex_coord;
    out_.color = unpack_color(a_color);
    out_.position = position_from_screen(a_pos);
    return out_;
}

float4 ps_main(VertexOutput in_) : SV_Target0
{
    float4 out_color_gamma = (in_.color * r_tex_color.Sample(r_tex_sampler, in_.tex_coord));
    if ((r_locals.dithering == 1u)) {
        out_color_gamma = float4(dither_interleaved(out_color_gamma.xyz, 256.0, in_.position), out_color_gamma.w);
    }
    return out_color_gamma;
}
"#;

struct Texture {
    texture: ID3D11Texture2D,
    view: Option<ID3D11ShaderResourceView>,
    size: [usize; 2],
    options: TextureOptions,
}

/// A growable dynamic buffer, rewritten every frame.
struct DynamicBuffer {
    buffer: Option<ID3D11Buffer>,
    capacity: usize,
    bind: D3D11_BIND_FLAG,
}

pub struct Renderer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    swap_chain: IDXGISwapChain2,
    /// Signaled when the swap chain can take another frame.
    frame_ready: HANDLE,
    target: Option<ID3D11RenderTargetView>,
    size: [u32; 2],
    vertex_shader: ID3D11VertexShader,
    pixel_shader: ID3D11PixelShader,
    input_layout: ID3D11InputLayout,
    blend: ID3D11BlendState,
    rasterizer: ID3D11RasterizerState,
    locals: ID3D11Buffer,
    vertices: DynamicBuffer,
    indices: DynamicBuffer,
    samplers: HashMap<TextureOptions, ID3D11SamplerState>,
    textures: HashMap<TextureId, Texture>,
}

impl Renderer {
    /// A renderer for `window`, whose client area is `size` pixels: on the GPU, or on WARP (the
    /// software rasterizer) when there is none.
    pub fn new(window: HWND, size: [u32; 2]) -> windows::core::Result<Self> {
        let (device, context) = create_device(D3D_DRIVER_TYPE_HARDWARE)
            .or_else(|_| create_device(D3D_DRIVER_TYPE_WARP))?;
        unsafe {
            let factory: IDXGIFactory2 = device.cast::<IDXGIDevice>()?.GetAdapter()?.GetParent()?;
            let swap_chain = factory
                .CreateSwapChainForHwnd(
                    &device,
                    window,
                    &DXGI_SWAP_CHAIN_DESC1 {
                        Width: size[0].max(1),
                        Height: size[1].max(1),
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        SampleDesc: DXGI_SAMPLE_DESC {
                            Count: 1,
                            Quality: 0,
                        },
                        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                        BufferCount: 2,
                        Scaling: DXGI_SCALING_STRETCH,
                        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                        Flags: SWAP_CHAIN_FLAGS.0 as u32,
                        ..Default::default()
                    },
                    None,
                    None,
                )?
                .cast::<IDXGISwapChain2>()?;
            // Alt+Enter would make it exclusive fullscreen.
            factory.MakeWindowAssociation(window, DXGI_MWA_NO_ALT_ENTER)?;
            swap_chain.SetMaximumFrameLatency(FRAME_LATENCY)?;
            let frame_ready = swap_chain.GetFrameLatencyWaitableObject();

            let vertex_code = compile(s!("vs_main"), s!("vs_5_0"))?;
            let pixel_code = compile(s!("ps_main"), s!("ps_5_0"))?;
            let mut vertex_shader = None;
            device.CreateVertexShader(blob(&vertex_code), None, Some(&mut vertex_shader))?;
            let mut pixel_shader = None;
            device.CreatePixelShader(blob(&pixel_code), None, Some(&mut pixel_shader))?;
            // Semantics LOC0..2, as naga names them.
            let element = |index: u32, format: DXGI_FORMAT, offset: u32| D3D11_INPUT_ELEMENT_DESC {
                SemanticName: s!("LOC"),
                SemanticIndex: index,
                Format: format,
                InputSlot: 0,
                AlignedByteOffset: offset,
                InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA,
                InstanceDataStepRate: 0,
            };
            let mut input_layout = None;
            // egui's Vertex: position, texture coordinates, sRGBA color as a u32.
            device.CreateInputLayout(
                &[
                    element(0, DXGI_FORMAT_R32G32_FLOAT, 0),
                    element(1, DXGI_FORMAT_R32G32_FLOAT, 8),
                    element(2, DXGI_FORMAT_R32_UINT, 16),
                ],
                blob(&vertex_code),
                Some(&mut input_layout),
            )?;

            // Premultiplied alpha, as egui-wgpu blends.
            let mut blend_desc = D3D11_BLEND_DESC::default();
            blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_ONE,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_INV_DEST_ALPHA,
                DestBlendAlpha: D3D11_BLEND_ONE,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let mut blend = None;
            device.CreateBlendState(&blend_desc, Some(&mut blend))?;
            let mut rasterizer = None;
            device.CreateRasterizerState(
                &D3D11_RASTERIZER_DESC {
                    FillMode: D3D11_FILL_SOLID,
                    CullMode: D3D11_CULL_NONE,
                    DepthClipEnable: true.into(),
                    ScissorEnable: true.into(),
                    ..Default::default()
                },
                Some(&mut rasterizer),
            )?;
            let mut locals = None;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: 16,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut locals),
            )?;

            Ok(Self {
                device,
                context,
                swap_chain,
                frame_ready,
                target: None,
                size,
                vertex_shader: vertex_shader.expect("vertex shader"),
                pixel_shader: pixel_shader.expect("pixel shader"),
                input_layout: input_layout.expect("input layout"),
                blend: blend.expect("blend state"),
                rasterizer: rasterizer.expect("rasterizer state"),
                locals: locals.expect("constant buffer"),
                vertices: DynamicBuffer::new(D3D11_BIND_VERTEX_BUFFER),
                indices: DynamicBuffer::new(D3D11_BIND_INDEX_BUFFER),
                samplers: HashMap::new(),
                textures: HashMap::new(),
            })
        }
    }

    /// The client area is now `size` pixels; zero while minimized.
    pub fn resize(&mut self, size: [u32; 2]) -> windows::core::Result<()> {
        if size == self.size {
            return Ok(());
        }
        self.size = size;
        if size[0] == 0 || size[1] == 0 {
            return Ok(());
        }
        // The buffers can only be resized once nothing refers to them.
        self.target = None;
        unsafe {
            self.context.OMSetRenderTargets(None, None);
            self.swap_chain.ResizeBuffers(
                0,
                size[0],
                size[1],
                DXGI_FORMAT_UNKNOWN,
                SWAP_CHAIN_FLAGS,
            )
        }
    }

    /// Applies `textures` and, unless the window has no area, draws `primitives` and presents.
    pub fn paint(
        &mut self,
        clear_color: [f32; 4],
        primitives: &[ClippedPrimitive],
        pixels_per_point: f32,
        textures: &TexturesDelta,
    ) -> windows::core::Result<()> {
        for (id, deltas) in &textures.set {
            for delta in deltas {
                self.update_texture(*id, delta)?;
            }
        }
        if self.size[0] > 0 && self.size[1] > 0 {
            self.draw(clear_color, primitives, pixels_per_point)?;
        }
        for id in &textures.free {
            self.textures.remove(id);
        }
        Ok(())
    }

    fn draw(
        &mut self,
        clear_color: [f32; 4],
        primitives: &[ClippedPrimitive],
        pixels_per_point: f32,
    ) -> windows::core::Result<()> {
        // Wait for the swap chain like a blocking present would, but before drawing.
        unsafe { WaitForSingleObjectEx(self.frame_ready, 1000, true) };
        if self.target.is_none() {
            unsafe {
                let buffer: ID3D11Texture2D = self.swap_chain.GetBuffer(0)?;
                self.device
                    .CreateRenderTargetView(&buffer, None, Some(&mut self.target))?;
            }
        }
        let meshes: Vec<_> = primitives
            .iter()
            .filter_map(|primitive| match &primitive.primitive {
                Primitive::Mesh(mesh) => Some(mesh),
                Primitive::Callback(_) => None,
            })
            .collect();
        let vertex_bytes = meshes
            .iter()
            .map(|mesh| mesh.vertices.len() * size_of::<Vertex>())
            .sum::<usize>();
        let index_bytes = meshes
            .iter()
            .map(|mesh| mesh.indices.len() * size_of::<u32>())
            .sum::<usize>();
        let context = self.context.clone();
        let vertex_buffer = self
            .vertices
            .write(&self.device, &context, vertex_bytes, |out| {
                for mesh in &meshes {
                    let bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
                    out.extend_from_slice(bytes);
                }
            })?;
        let index_buffer = self
            .indices
            .write(&self.device, &context, index_bytes, |out| {
                for mesh in &meshes {
                    out.extend_from_slice(bytemuck::cast_slice(&mesh.indices));
                }
            })?;
        let [width, height] = self.size;
        let locals = Locals {
            screen_size: [
                width as f32 / pixels_per_point,
                height as f32 / pixels_per_point,
            ],
            dithering: 1,
            padding: 0,
        };
        let target = self.target.clone().expect("render target");
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(
                &self.locals,
                0,
                D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped),
            )?;
            std::ptr::write_unaligned(mapped.pData.cast::<Locals>(), locals);
            context.Unmap(&self.locals, 0);

            context.OMSetRenderTargets(Some(&[Some(target.clone())]), None);
            context.ClearRenderTargetView(&target, &clear_color);
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: width as f32,
                Height: height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            context.RSSetState(&self.rasterizer);
            context.OMSetBlendState(&self.blend, None, u32::MAX);
            context.IASetInputLayout(&self.input_layout);
            context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            let stride = size_of::<Vertex>() as u32;
            let offset = 0;
            context.IASetVertexBuffers(0, 1, Some(&vertex_buffer), Some(&stride), Some(&offset));
            context.IASetIndexBuffer(index_buffer.as_ref(), DXGI_FORMAT_R32_UINT, 0);
            context.VSSetShader(&self.vertex_shader, None);
            context.VSSetConstantBuffers(0, Some(&[Some(self.locals.clone())]));
            context.PSSetShader(&self.pixel_shader, None);
            context.PSSetConstantBuffers(0, Some(&[Some(self.locals.clone())]));
        }

        let (mut first_vertex, mut first_index) = (0usize, 0usize);
        for primitive in primitives {
            let Primitive::Mesh(mesh) = &primitive.primitive else {
                continue;
            };
            let (vertex, index) = (first_vertex, first_index);
            first_vertex += mesh.vertices.len();
            first_index += mesh.indices.len();
            let scissor = scissor(&primitive.clip_rect, pixels_per_point, self.size);
            if scissor.right <= scissor.left || scissor.bottom <= scissor.top {
                continue;
            }
            let Some(texture) = self.textures.get(&mesh.texture_id) else {
                continue;
            };
            let sampler = sampler(&self.device, &mut self.samplers, texture.options)?;
            unsafe {
                context.RSSetScissorRects(Some(&[scissor]));
                context.PSSetShaderResources(0, Some(std::slice::from_ref(&texture.view)));
                context.PSSetSamplers(0, Some(&[Some(sampler)]));
                context.DrawIndexed(mesh.indices.len() as u32, index as u32, vertex as i32);
            }
        }
        // Vsync, as egui-wgpu presents (`PresentMode::AutoVsync`).
        unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)) }.ok()
    }

    fn update_texture(&mut self, id: TextureId, delta: &ImageDelta) -> windows::core::Result<()> {
        let ImageData::Color(image) = &delta.image;
        let size = image.size;
        let bytes: &[u8] = bytemuck::cast_slice(&image.pixels);
        let row_pitch = (size[0] * 4) as u32;
        if let Some(pos) = delta.pos {
            // Part of a texture, such as glyphs added to the font atlas.
            let Some(texture) = self.textures.get_mut(&id) else {
                return Ok(());
            };
            texture.options = delta.options;
            let area = D3D11_BOX {
                left: pos[0] as u32,
                top: pos[1] as u32,
                front: 0,
                right: (pos[0] + size[0]) as u32,
                bottom: (pos[1] + size[1]) as u32,
                back: 1,
            };
            unsafe {
                self.context.UpdateSubresource(
                    &texture.texture,
                    0,
                    Some(&area),
                    bytes.as_ptr().cast(),
                    row_pitch,
                    0,
                )
            };
            return Ok(());
        }
        // A whole image: overwrite a texture of the same size (an emulator screen, every frame),
        // or make a new one.
        if let Some(texture) = self.textures.get_mut(&id)
            && texture.size == size
        {
            texture.options = delta.options;
            unsafe {
                self.context.UpdateSubresource(
                    &texture.texture,
                    0,
                    None,
                    bytes.as_ptr().cast(),
                    row_pitch,
                    0,
                )
            };
            return Ok(());
        }
        let mut texture = None;
        let mut view = None;
        unsafe {
            self.device.CreateTexture2D(
                &D3D11_TEXTURE2D_DESC {
                    Width: size[0] as u32,
                    Height: size[1] as u32,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                    ..Default::default()
                },
                Some(&D3D11_SUBRESOURCE_DATA {
                    pSysMem: bytes.as_ptr().cast(),
                    SysMemPitch: row_pitch,
                    SysMemSlicePitch: 0,
                }),
                Some(&mut texture),
            )?;
            let texture = texture.expect("texture");
            self.device
                .CreateShaderResourceView(&texture, None, Some(&mut view))?;
            self.textures.insert(
                id,
                Texture {
                    texture,
                    view,
                    size,
                    options: delta.options,
                },
            );
        }
        Ok(())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            self.context.ClearState();
            let _ = windows::Win32::Foundation::CloseHandle(self.frame_ready);
        }
    }
}

impl DynamicBuffer {
    fn new(bind: D3D11_BIND_FLAG) -> Self {
        Self {
            buffer: None,
            capacity: 0,
            bind,
        }
    }

    /// Replaces the contents with `fill`'s `len` bytes, growing the buffer when needed.
    fn write(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        len: usize,
        fill: impl FnOnce(&mut Vec<u8>),
    ) -> windows::core::Result<Option<ID3D11Buffer>> {
        if len == 0 {
            return Ok(self.buffer.clone());
        }
        if self.buffer.is_none() || len > self.capacity {
            let capacity = len.next_power_of_two().max(64 * 1024);
            let mut buffer = None;
            unsafe {
                device.CreateBuffer(
                    &D3D11_BUFFER_DESC {
                        ByteWidth: capacity as u32,
                        Usage: D3D11_USAGE_DYNAMIC,
                        BindFlags: self.bind.0 as u32,
                        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                        ..Default::default()
                    },
                    None,
                    Some(&mut buffer),
                )?;
            }
            self.buffer = buffer;
            self.capacity = capacity;
        }
        let buffer = self.buffer.clone().expect("buffer");
        let mut bytes = Vec::with_capacity(len);
        fill(&mut bytes);
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pData.cast::<u8>(), bytes.len());
            context.Unmap(&buffer, 0);
        }
        Ok(Some(buffer))
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Locals {
    screen_size: [f32; 2],
    dithering: u32,
    padding: u32,
}

fn create_device(
    driver: D3D_DRIVER_TYPE,
) -> windows::core::Result<(ID3D11Device, ID3D11DeviceContext)> {
    let (mut device, mut context) = (None, None);
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )?;
    }
    Ok((device.expect("device"), context.expect("device context")))
}

fn compile(entry: PCSTR, target: PCSTR) -> windows::core::Result<ID3DBlob> {
    let mut code = None;
    unsafe {
        D3DCompile(
            SHADER.as_ptr().cast(),
            SHADER.len(),
            s!("egui.hlsl"),
            None,
            None,
            entry,
            target,
            // wgpu's flags.
            D3DCOMPILE_ENABLE_STRICTNESS,
            0,
            &mut code,
            None,
        )?;
    }
    Ok(code.expect("shader code"))
}

fn blob(blob: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast(), blob.GetBufferSize()) }
}

/// The sampler for `options`, made on first use. Mipmaps are not made, as in egui-wgpu.
fn sampler(
    device: &ID3D11Device,
    samplers: &mut HashMap<TextureOptions, ID3D11SamplerState>,
    options: TextureOptions,
) -> windows::core::Result<ID3D11SamplerState> {
    if let Some(sampler) = samplers.get(&options) {
        return Ok(sampler.clone());
    }
    let linear = |filter| filter == TextureFilter::Linear;
    let filter = match (linear(options.minification), linear(options.magnification)) {
        (false, false) => D3D11_FILTER_MIN_MAG_MIP_POINT,
        (false, true) => D3D11_FILTER_MIN_POINT_MAG_LINEAR_MIP_POINT,
        (true, false) => D3D11_FILTER_MIN_LINEAR_MAG_MIP_POINT,
        (true, true) => D3D11_FILTER_MIN_MAG_LINEAR_MIP_POINT,
    };
    let address = match options.wrap_mode {
        TextureWrapMode::ClampToEdge => D3D11_TEXTURE_ADDRESS_CLAMP,
        TextureWrapMode::Repeat => D3D11_TEXTURE_ADDRESS_WRAP,
        TextureWrapMode::MirroredRepeat => D3D11_TEXTURE_ADDRESS_MIRROR,
    };
    let mut sampler = None;
    unsafe {
        device.CreateSamplerState(
            &D3D11_SAMPLER_DESC {
                Filter: filter,
                AddressU: address,
                AddressV: address,
                AddressW: address,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                MaxLOD: f32::MAX,
                ..Default::default()
            },
            Some(&mut sampler),
        )?;
    }
    let sampler = sampler.expect("sampler");
    samplers.insert(options, sampler.clone());
    Ok(sampler)
}

/// `clip_rect` in pixels, rounded and kept inside the target, as egui-wgpu clips.
fn scissor(clip_rect: &egui::Rect, pixels_per_point: f32, size: [u32; 2]) -> RECT {
    let pixels =
        |value: f32, max: u32| ((value * pixels_per_point).round().max(0.0) as u32).min(max);
    let left = pixels(clip_rect.min.x, size[0]);
    let top = pixels(clip_rect.min.y, size[1]);
    let right = pixels(clip_rect.max.x, size[0]).max(left);
    let bottom = pixels(clip_rect.max.y, size[1]).max(top);
    RECT {
        left: left as i32,
        top: top as i32,
        right: right as i32,
        bottom: bottom as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scissor_rounds_and_clamps() {
        let rect = egui::Rect::from_min_max(egui::pos2(-3.0, 10.2), egui::pos2(700.0, 20.0));
        let clip = scissor(&rect, 1.5, [960, 25]);
        assert_eq!(
            (clip.left, clip.top, clip.right, clip.bottom),
            (0, 15, 960, 25)
        );
        // Entirely outside: empty.
        let rect = egui::Rect::from_min_max(egui::pos2(800.0, 0.0), egui::pos2(900.0, 10.0));
        let clip = scissor(&rect, 1.5, [960, 640]);
        assert_eq!(clip.left, clip.right);
    }
}
