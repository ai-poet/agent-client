//! Within-window backdrop snapshots. All GPU state is lazy and device-local.
use super::*;

const IDLE_FRAMES: u32 = 120;
const KERNEL_VECTORS: usize = 33;
// Bound shader work even when callers supply extreme finite radii. Keep in sync
// with backdrop_pass_fragment; this is sigma in device pixels (192-tap radius).
const MAX_SIGMA: f32 = 64.;

// b1 in shaders.hlsl. Every field starts on a 16-byte HLSL register boundary.
#[repr(C)]
#[derive(Clone, Copy)]
struct Params {
    bounds: [f32; 4],
    corners: [f32; 4],
    clip: [f32; 4],
    source: [f32; 4],
    // xy = native-source UV step, z = sigma, w = 1 only for texel-aligned paired taps.
    kernel: [f32; 4],
    weights: [[f32; 4]; KERNEL_VECTORS],
}

struct Texture {
    texture: ID3D11Texture2D,
    srv: Option<ID3D11ShaderResourceView>,
    rtv: Option<ID3D11RenderTargetView>,
}

impl Texture {
    fn new(device: &ID3D11Device, width: u32, height: u32, renderable: bool) -> Result<Self> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: RENDER_TARGET_FORMAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0
                | if renderable {
                    D3D11_BIND_RENDER_TARGET.0
                } else {
                    0
                }) as u32,
            ..Default::default()
        };
        unsafe {
            let mut texture = None;
            device.CreateTexture2D(&desc, None, Some(&mut texture))?;
            let texture = texture.unwrap();
            let mut srv = None;
            device.CreateShaderResourceView(&texture, None, Some(&mut srv))?;
            let mut rtv = None;
            if renderable {
                device.CreateRenderTargetView(&texture, None, Some(&mut rtv))?;
            }
            Ok(Self { texture, srv, rtv })
        }
    }
}

struct Scratch {
    width: u32,
    height: u32,
    downsample: u32,
    snapshot: Texture,
    horizontal: Texture,
    vertical: Texture,
}

struct Shaders {
    vertex: ID3D11VertexShader,
    fragment: ID3D11PixelShader,
}

impl Shaders {
    fn new(device: &ID3D11Device, module: ShaderModule) -> Result<Self> {
        Ok(Self {
            vertex: create_vertex_shader(
                device,
                RawShaderBytes::new(module, ShaderTarget::Vertex)?.as_bytes(),
            )?,
            fragment: create_fragment_shader(
                device,
                RawShaderBytes::new(module, ShaderTarget::Fragment)?.as_bytes(),
            )?,
        })
    }
}

pub(super) struct BackdropResources {
    pass: Shaders,
    composite: Shaders,
    params: Option<ID3D11Buffer>,
    sampler: Option<ID3D11SamplerState>,
    scratch: Option<Scratch>,
    idle_frames: u32,
}

impl BackdropResources {
    #[cfg(test)]
    pub(super) fn snapshot(&self) -> &ID3D11Texture2D {
        &self.scratch.as_ref().unwrap().snapshot.texture
    }

    fn new(device: &ID3D11Device) -> Result<Self> {
        let pass = Shaders::new(device, ShaderModule::BackdropPass)?;
        let composite = Shaders::new(device, ShaderModule::BackdropComposite)?;
        unsafe {
            let mut params = None;
            device.CreateBuffer(
                &D3D11_BUFFER_DESC {
                    ByteWidth: std::mem::size_of::<Params>() as u32,
                    Usage: D3D11_USAGE_DYNAMIC,
                    BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
                    ..Default::default()
                },
                None,
                Some(&mut params),
            )?;
            let mut sampler = None;
            device.CreateSamplerState(
                &D3D11_SAMPLER_DESC {
                    Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                    AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                    MaxLOD: D3D11_FLOAT32_MAX,
                    MaxAnisotropy: 1,
                    ComparisonFunc: D3D11_COMPARISON_ALWAYS,
                    ..Default::default()
                },
                Some(&mut sampler),
            )?;
            Ok(Self {
                pass,
                composite,
                params,
                sampler,
                scratch: None,
                idle_frames: 0,
            })
        }
    }

    pub(super) fn expire(&mut self) -> bool {
        self.idle_frames += 1;
        self.idle_frames >= IDLE_FRAMES
    }

    fn ensure_scratch(
        &mut self,
        device: &ID3D11Device,
        width: u32,
        height: u32,
        downsample: u32,
        viewport: [u32; 2],
    ) -> Result<()> {
        self.idle_frames = 0;
        if self
            .scratch
            .as_ref()
            .is_some_and(|s| s.width >= width && s.height >= height && s.downsample == downsample)
        {
            return Ok(());
        }
        // Grow in tiles to avoid allocations while resizing/animating a popover.
        // Clamp to the drawable so every cached texel is filled with real pixels.
        let previous = self.scratch.as_ref();
        let width = width
            .max(previous.map_or(0, |s| s.width))
            .div_ceil(64)
            .saturating_mul(64)
            .min(viewport[0]);
        let height = height
            .max(previous.map_or(0, |s| s.height))
            .div_ceil(64)
            .saturating_mul(64)
            .min(viewport[1]);
        self.scratch = Some(Scratch {
            width,
            height,
            downsample,
            snapshot: Texture::new(device, width, height, false)?,
            horizontal: Texture::new(device, width.div_ceil(downsample), height, true)?,
            vertical: Texture::new(
                device,
                width.div_ceil(downsample),
                height.div_ceil(downsample),
                true,
            )?,
        });
        Ok(())
    }

    fn draw_pass(
        &self,
        context: &ID3D11DeviceContext,
        shaders: &Shaders,
        source: &Texture,
        target: &Option<ID3D11RenderTargetView>,
        viewport: D3D11_VIEWPORT,
        params: Params,
    ) -> Result<()> {
        // WRITE_DISCARD lets the immediate context rename storage between draws;
        // previously submitted passes keep their own constants, including nested blurs.
        update_buffer(context, self.params.as_ref().unwrap(), &[params])?;
        unsafe {
            context.PSSetShaderResources(0, Some(&[None]));
            context.OMSetRenderTargets(Some(slice::from_ref(target)), None);
            context.PSSetShaderResources(0, Some(slice::from_ref(&source.srv)));
            context.VSSetConstantBuffers(1, Some(slice::from_ref(&self.params)));
            context.PSSetConstantBuffers(1, Some(slice::from_ref(&self.params)));
            context.PSSetSamplers(0, Some(slice::from_ref(&self.sampler)));
            context.RSSetViewports(Some(&[viewport]));
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            context.VSSetShader(&shaders.vertex, None);
            context.PSSetShader(&shaders.fragment, None);
            // Default D3D11 blend state disables blending: snapshot RGBA is
            // already premultiplied and must replace, not source-over, itself.
            context.OMSetBlendState(None, None, u32::MAX);
            context.Draw(4, 0);
        }
        Ok(())
    }
}

impl DirectXRenderer {
    pub(super) fn draw_backdrop_blur(&mut self, blur: &BackdropBlur) -> Result<()> {
        if !blur.blur_radius.0.is_finite()
            || !valid_bounds(blur.bounds)
            || !valid_bounds(blur.content_mask.bounds)
            || ![
                blur.corner_radii.top_left.0,
                blur.corner_radii.top_right.0,
                blur.corner_radii.bottom_right.0,
                blur.corner_radii.bottom_left.0,
            ]
            .iter()
            .all(|value| value.is_finite() && *value >= 0.)
        {
            return Ok(());
        }
        let visible = blur
            .bounds
            .intersect(&blur.content_mask.bounds)
            .intersect(&Bounds::new(
                point(ScaledPixels(0.), ScaledPixels(0.)),
                size(
                    ScaledPixels(self.width as f32),
                    ScaledPixels(self.height as f32),
                ),
            ));
        if visible.size.width.0 <= 0. || visible.size.height.0 <= 0. {
            return Ok(());
        }
        let sigma = blur.blur_radius.0.clamp(1., MAX_SIGMA);
        let padding = (sigma * 3.).ceil() + 2.;
        let x0 = (visible.origin.x.0 - padding).floor().max(0.) as u32;
        let y0 = (visible.origin.y.0 - padding).floor().max(0.) as u32;
        let x1 = (visible.origin.x.0 + visible.size.width.0 + padding)
            .ceil()
            .min(self.width as f32) as u32;
        let y1 = (visible.origin.y.0 + visible.size.height.0 + padding)
            .ceil()
            .min(self.height as f32) as u32;
        if x1 <= x0 || y1 <= y0 {
            return Ok(());
        }
        let downsample = ((sigma / 8.) as u32).clamp(1, 4);
        let devices = self.devices.as_ref().context("devices missing")?;
        let source = self.backdrop_source.clone();
        let resources = self.resources.as_mut().context("resources missing")?;
        if resources.backdrop.is_none() {
            resources.backdrop = Some(BackdropResources::new(&devices.device)?);
        }
        let backdrop = resources.backdrop.as_mut().unwrap();
        backdrop.ensure_scratch(
            &devices.device,
            x1 - x0,
            y1 - y0,
            downsample,
            [self.width, self.height],
        )?;
        let scratch = backdrop.scratch.as_ref().unwrap();
        let copy_x = x0.min(self.width - scratch.width);
        let copy_y = y0.min(self.height - scratch.height);
        let context = &devices.device_context;
        unsafe {
            // A blur can be the first (or only) operation of the frame.
            context
                .VSSetConstantBuffers(0, Some(slice::from_ref(&self.globals.global_params_buffer)));
            context
                .PSSetConstantBuffers(0, Some(slice::from_ref(&self.globals.global_params_buffer)));
            // Unbind before copying or changing a texture from RTV to SRV.
            // D3D11 otherwise silently nulls conflicting SRV bindings.
            context.VSSetShaderResources(0, Some(&[None]));
            context.PSSetShaderResources(0, Some(&[None]));
            context.OMSetRenderTargets(None, None);
            context.CopySubresourceRegion(
                &scratch.snapshot.texture,
                0,
                0,
                0,
                0,
                source
                    .as_ref()
                    .or(resources.render_target.as_ref())
                    .context("missing render target")?,
                0,
                Some(&D3D11_BOX {
                    left: copy_x,
                    top: copy_y,
                    front: 0,
                    right: copy_x + scratch.width,
                    bottom: copy_y + scratch.height,
                    back: 1,
                }),
            );
        }
        // Filter native pixels before reducing either axis to avoid aliasing fine detail.
        let sigma_texels = sigma;
        let mut params = Params {
            bounds: rect(blur.bounds),
            corners: [
                blur.corner_radii.top_left.0,
                blur.corner_radii.top_right.0,
                blur.corner_radii.bottom_right.0,
                blur.corner_radii.bottom_left.0,
            ],
            clip: rect(blur.content_mask.bounds),
            source: [
                copy_x as f32,
                copy_y as f32,
                scratch.width as f32,
                scratch.height as f32,
            ],
            kernel: [
                1. / scratch.width as f32,
                0.,
                sigma_texels,
                downsample as f32,
            ],
            weights: gaussian_weights(sigma_texels),
        };
        let viewport = D3D11_VIEWPORT {
            Width: scratch.width.div_ceil(downsample) as f32,
            Height: scratch.height as f32,
            MaxDepth: 1.,
            ..Default::default()
        };
        // Always restore the main target and unbind owned resources, also on
        // Map failure. The context must not retain cache resources after expiry.
        let result = (|| {
            backdrop.draw_pass(
                context,
                &backdrop.pass,
                &scratch.snapshot,
                &scratch.horizontal.rtv,
                viewport,
                params,
            )?;
            params.kernel = [0., 1. / viewport.Height, sigma_texels, downsample as f32];
            let viewport = D3D11_VIEWPORT {
                Height: scratch.height.div_ceil(downsample) as f32,
                ..viewport
            };
            backdrop.draw_pass(
                context,
                &backdrop.pass,
                &scratch.horizontal,
                &scratch.vertical.rtv,
                viewport,
                params,
            )?;
            backdrop.draw_pass(
                context,
                &backdrop.composite,
                &scratch.vertical,
                &resources.render_target_view,
                resources.viewport,
                params,
            )
        })();
        unsafe {
            context.PSSetShaderResources(0, Some(&[None]));
            context.VSSetConstantBuffers(1, Some(&[None]));
            context.PSSetConstantBuffers(1, Some(&[None]));
            context.PSSetSamplers(0, Some(&[None]));
            context.VSSetShader(None, None);
            context.PSSetShader(None, None);
            context.OMSetRenderTargets(Some(slice::from_ref(&resources.render_target_view)), None);
            context.RSSetViewports(Some(&[resources.viewport]));
        }
        result.context("Drawing backdrop blur")
    }
}

fn rect(bounds: Bounds<ScaledPixels>) -> [f32; 4] {
    [
        bounds.origin.x.0,
        bounds.origin.y.0,
        bounds.size.width.0,
        bounds.size.height.0,
    ]
}

fn valid_bounds(bounds: Bounds<ScaledPixels>) -> bool {
    rect(bounds).iter().all(|value| value.is_finite())
        && bounds.size.width.0 >= 0.
        && bounds.size.height.0 >= 0.
        && bounds.right().0.is_finite()
        && bounds.bottom().0.is_finite()
}

fn gaussian_weights(sigma: f32) -> [[f32; 4]; KERNEL_VECTORS] {
    let mut weights = [[0.; 4]; KERNEL_VECTORS];
    let radius = (sigma * 3.).ceil() as usize;
    if radius > 128 {
        return weights;
    }
    let mut total = 0.;
    for k in -(radius as i32)..=radius as i32 {
        let weight = (-(k as f32) * k as f32 / (2. * sigma * sigma)).exp();
        total += weight;
        let k = k.unsigned_abs() as usize;
        weights[k / 4][k % 4] = weight;
    }
    for weight in weights.iter_mut().flatten() {
        *weight /= total;
    }
    weights
}
