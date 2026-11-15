use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;

use windows::Win32::Foundation::{GENERIC_READ, HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::Fxc::{D3DCOMPILE_OPTIMIZATION_LEVEL3, D3DCompile};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST,
    D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP, D3D_SRV_DIMENSION_BUFFER, ID3DBlob,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::{
    DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR, DCompositionCreateDevice, IDCompositionDevice, IDCompositionEffectGroup, IDCompositionTarget,
    IDCompositionVisual,
};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_R16G16_FLOAT,
    DXGI_FORMAT_R32_FLOAT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32G32B32_FLOAT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT, IDXGIAdapter,
    IDXGIDevice, IDXGIFactory2, IDXGISwapChain1,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::core::{Error, Interface, PCSTR, PCWSTR, Result, s};
use windows_numerics::Matrix3x2;

use snowhush_core::geometry::V2;
use snowhush_core::halloween::{PUMPKIN_ASPECT, PUMPKIN_CANDLE};
use snowhush_core::simulation::{COLUMN_WIDTH, PumpkinSprite, SnowSimulation, Sprite};

use crate::halloween::{self, SIGN_SCALE};
use crate::log;

// The digitized leaves' tables come first: the shaders draw the leaves from them.
const SHADERS: &str = concat!(include_str!("leaves.hlsl"), "\n", include_str!("shaders.hlsl"));

/// How far the cursor's candlelight reaches, in points.
pub const LANTERN_REACH: f32 = 130.0;

// The sprites' input layout reads a ghost's, a leaf's, rain's and a drop's parts at these offsets.
const _: () = assert!(
    offset_of!(Sprite, ghost) == 48
        && offset_of!(Sprite, pose) == 64
        && offset_of!(Sprite, mood) == 80
        && offset_of!(Sprite, leaf) == 88
        && offset_of!(Sprite, spin) == 100
        && offset_of!(Sprite, rain) == 108
        && offset_of!(Sprite, drop) == 116
        && offset_of!(Sprite, petal) == 132
        && size_of::<Sprite>() == 136
);

#[repr(C)]
#[derive(Clone, Copy)]
struct Uniforms {
    view_size: [f32; 2],
    scale: f32,
    time: f32,
    column_width: f32,
    column_count: i32,
    pile_top: f32,
    occluder_count: i32,
    origin: [f32; 2],
    pile_bottom: f32,
    height_offset: i32,
    light: [f32; 4],
    shadow_tint: [f32; 4],
    rim: [f32; 4],
    /// The cursor's glow: where it is, how far it reaches and how bright it is.
    lantern: [f32; 4],
    /// How far everything is jolted: x and y.
    shake: [f32; 4],
    /// The pumpkins' candles: where each burns, how far its light reaches and how bright it is.
    pumpkins: [[f32; 4]; 3],
    occluders: [[f32; 4]; 16],
    /// One while all that heaps up is leaves, for a ghost; a pile has its own leaves and petals in each column.
    autumn: f32,
    /// One while the leaves are cherry blossom petals.
    blossom: f32,
    /// One while fallen leaves wither away.
    wilt: f32,
    padding: f32,
}

/// Where a sign stands: its stick's foot, its size in points, how far it is turned and the drift's surface it comes up out of.
#[repr(C)]
#[derive(Clone, Copy)]
struct Placement {
    anchor: [f32; 2],
    size: [f32; 2],
    angle: f32,
    ground: f32,
    padding: [f32; 2],
}

/// Where a pumpkin sits: the middle of its bottom, its size in points, where its candle burns and what sets its
/// flicker apart from the others'.
#[repr(C)]
#[derive(Clone, Copy)]
struct PumpkinPlacement {
    anchor: [f32; 2],
    size: [f32; 2],
    candle: [f32; 2],
    seed: f32,
    padding: f32,
}

impl PumpkinPlacement {
    fn new(pumpkin: &PumpkinSprite) -> PumpkinPlacement {
        let size = [pumpkin.width, pumpkin.width * PUMPKIN_ASPECT];
        let candle = [pumpkin.anchor.x + (PUMPKIN_CANDLE[0] - 0.5) * size[0], pumpkin.anchor.y + PUMPKIN_CANDLE[1] * size[1]];
        PumpkinPlacement { anchor: [pumpkin.anchor.x, pumpkin.anchor.y], size, candle, seed: pumpkin.seed, padding: 0.0 }
    }

    /// Its candle's light on the snow: where it burns, how far it reaches and how bright it is, brighter at `night`.
    fn light(&self, night: f32) -> [f32; 4] {
        [self.candle[0], self.candle[1], self.size[0] * 1.6, 0.4 + 0.6 * night]
    }
}

/// The light the snow is drawn in.
#[derive(Clone, Copy)]
pub struct Ambient {
    pub light: [f32; 3],
    pub shadow_tint: [f32; 4],
    /// The colour the snow's surface glows, and how much.
    pub rim: [f32; 4],
    /// The cursor, in points, glowing like a candle in a jack-o'-lantern, and how brightly.
    pub lantern: (V2, f32),
}

/// A sign's picture on the graphics card, and its size in points.
type Picture = (ID3D11ShaderResourceView, [f32; 2]);

struct CleanerImage {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

fn decode_cleaner_storyboard(bytes: &[u8]) -> Result<CleanerImage> {
    unsafe {
        let imaging: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream = SHCreateMemStream(Some(bytes)).ok_or_else(Error::empty)?;
        let decoder = imaging.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let converter = imaging.CreateFormatConverter()?;
        converter.Initialize(&decoder.GetFrame(0)?, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
        let (mut width, mut height) = (0, 0);
        converter.GetSize(&mut width, &mut height)?;
        if width == 0 || height == 0 || width > 4096 || height > 4096 {
            return Err(Error::empty());
        }
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
        Ok(CleanerImage { width, height, pixels })
    }
}

pub struct Gpu {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    factory: IDXGIFactory2,
    composition: IDCompositionDevice,
    sprite_vertex: ID3D11VertexShader,
    sprite_fragment: ID3D11PixelShader,
    sprite_layout: ID3D11InputLayout,
    pile_vertex: ID3D11VertexShader,
    pile_fragment: ID3D11PixelShader,
    sign_vertex: ID3D11VertexShader,
    sign_fragment: ID3D11PixelShader,
    cleaner_vertex: ID3D11VertexShader,
    cleaner_fragment: ID3D11PixelShader,
    pumpkin_vertex: ID3D11VertexShader,
    pumpkin_fragment: ID3D11PixelShader,
    /// Drops on the glass: their bumps added up into a field, and the glass drawn from it.
    field_vertex: ID3D11VertexShader,
    field_fragment: ID3D11PixelShader,
    glass_vertex: ID3D11VertexShader,
    glass_fragment: ID3D11PixelShader,
    winter_scene_vertex: ID3D11VertexShader,
    winter_scene_fragment: ID3D11PixelShader,
    adding: ID3D11BlendState,
    sampler: ID3D11SamplerState,
    /// Clear past the picture's edges, for the pumpkins' glow around them.
    bordered: ID3D11SamplerState,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    /// The same, drawing only within the scissor rect: the glass, around the drops.
    scissored: ID3D11RasterizerState,
    uniforms: ID3D11Buffer,
    placement: ID3D11Buffer,
    pumpkin: ID3D11Buffer,
    /// Each caption's sign, made the first time a ghost holds it up; none if it couldn't be.
    signs: RefCell<HashMap<usize, Option<Picture>>>,
    /// A user-selected cleaner image, decoded only when the custom cleaner is visible.
    winter_picture: RefCell<HashMap<String, Option<Picture>>>,
    /// The pumpkin's shell and its face, made the first time one is drawn; none if they couldn't be.
    pumpkin_pictures: OnceCell<Option<[ID3D11ShaderResourceView; 2]>>,
    /// The two six-frame cleaner strips, decoded and uploaded once with the rest of the device resources.
    cleaner_pictures: OnceCell<Option<[Picture; 2]>>,
    pub adapter: String,
}

impl Gpu {
    pub fn new() -> Result<Gpu> {
        unsafe {
            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;
            let device: ID3D11Device = device.ok_or_else(Error::empty)?;
            let context = context.ok_or_else(Error::empty)?;
            let dxgi: IDXGIDevice = device.cast()?;
            let adapter: IDXGIAdapter = dxgi.GetAdapter()?;
            let description = adapter.GetDesc()?;
            let name_length = description.Description.iter().position(|&c| c == 0).unwrap_or(description.Description.len());
            let factory: IDXGIFactory2 = adapter.GetParent()?;
            let composition: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;

            let code = Code::get()?;
            let sprite_vertex = vertex_shader(&device, &code.sprite_vertex)?;
            let pile_vertex = vertex_shader(&device, &code.pile_vertex)?;
            let sprite_fragment = pixel_shader(&device, &code.sprite_fragment)?;
            let pile_fragment = pixel_shader(&device, &code.pile_fragment)?;
            let sign_vertex = vertex_shader(&device, &code.sign_vertex)?;
            let sign_fragment = pixel_shader(&device, &code.sign_fragment)?;
            let cleaner_vertex = vertex_shader(&device, &code.cleaner_vertex)?;
            let cleaner_fragment = pixel_shader(&device, &code.cleaner_fragment)?;
            let pumpkin_vertex = vertex_shader(&device, &code.pumpkin_vertex)?;
            let pumpkin_fragment = pixel_shader(&device, &code.pumpkin_fragment)?;
            let field_vertex = vertex_shader(&device, &code.field_vertex)?;
            let field_fragment = pixel_shader(&device, &code.field_fragment)?;
            let glass_vertex = vertex_shader(&device, &code.glass_vertex)?;
            let glass_fragment = pixel_shader(&device, &code.glass_fragment)?;
            let winter_scene_vertex = vertex_shader(&device, &code.winter_scene_vertex)?;
            let winter_scene_fragment = pixel_shader(&device, &code.winter_scene_fragment)?;

            let element = |name: PCSTR, format: DXGI_FORMAT, offset: u32| D3D11_INPUT_ELEMENT_DESC {
                SemanticName: name,
                SemanticIndex: 0,
                Format: format,
                InputSlot: 0,
                AlignedByteOffset: offset,
                InputSlotClass: D3D11_INPUT_PER_INSTANCE_DATA,
                InstanceDataStepRate: 1,
            };
            let elements = [
                element(s!("POSITION"), DXGI_FORMAT_R32G32_FLOAT, 0),
                element(s!("VELOCITY"), DXGI_FORMAT_R32G32_FLOAT, 8),
                element(s!("BODY"), DXGI_FORMAT_R32G32B32A32_FLOAT, 16),
                element(s!("TINTCOLOR"), DXGI_FORMAT_R32G32B32_FLOAT, 32),
                element(s!("LUMP"), DXGI_FORMAT_R32_FLOAT, 44),
                element(s!("GHOST"), DXGI_FORMAT_R32G32B32A32_FLOAT, 48),
                element(s!("POSE"), DXGI_FORMAT_R32G32B32A32_FLOAT, 64),
                element(s!("MOOD"), DXGI_FORMAT_R32G32_FLOAT, 80),
                element(s!("LEAF"), DXGI_FORMAT_R32G32B32_FLOAT, 88),
                element(s!("SPIN"), DXGI_FORMAT_R32G32_FLOAT, 100),
                element(s!("RAIN"), DXGI_FORMAT_R32G32_FLOAT, 108),
                element(s!("DROP"), DXGI_FORMAT_R32G32B32A32_FLOAT, 116),
                element(s!("PETAL"), DXGI_FORMAT_R32_FLOAT, 132),
            ];
            let mut sprite_layout = None;
            device.CreateInputLayout(&elements, &code.sprite_vertex, Some(&mut sprite_layout))?;

            let target = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_ONE,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_ONE,
                DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let blend_desc = D3D11_BLEND_DESC { RenderTarget: [target; 8], ..Default::default() };
            let mut blend = None;
            device.CreateBlendState(&blend_desc, Some(&mut blend))?;
            let adding_target = D3D11_RENDER_TARGET_BLEND_DESC { DestBlend: D3D11_BLEND_ONE, DestBlendAlpha: D3D11_BLEND_ONE, ..target };
            let mut adding = None;
            device.CreateBlendState(&D3D11_BLEND_DESC { RenderTarget: [adding_target; 8], ..Default::default() }, Some(&mut adding))?;

            let raster_desc =
                D3D11_RASTERIZER_DESC { FillMode: D3D11_FILL_SOLID, CullMode: D3D11_CULL_NONE, DepthClipEnable: true.into(), ..Default::default() };
            let mut raster = None;
            device.CreateRasterizerState(&raster_desc, Some(&mut raster))?;
            let mut scissored = None;
            device.CreateRasterizerState(&D3D11_RASTERIZER_DESC { ScissorEnable: true.into(), ..raster_desc }, Some(&mut scissored))?;

            let sampler_desc = D3D11_SAMPLER_DESC {
                Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
                AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
                AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
                MaxAnisotropy: 1,
                ComparisonFunc: D3D11_COMPARISON_NEVER,
                MaxLOD: f32::MAX,
                ..Default::default()
            };
            let mut sampler = None;
            device.CreateSamplerState(&sampler_desc, Some(&mut sampler))?;
            let bordered_desc = D3D11_SAMPLER_DESC {
                AddressU: D3D11_TEXTURE_ADDRESS_BORDER,
                AddressV: D3D11_TEXTURE_ADDRESS_BORDER,
                AddressW: D3D11_TEXTURE_ADDRESS_BORDER,
                BorderColor: [0.0; 4],
                ..sampler_desc
            };
            let mut bordered = None;
            device.CreateSamplerState(&bordered_desc, Some(&mut bordered))?;

            let uniforms = dynamic_buffer(&device, size_of::<Uniforms>(), D3D11_BIND_CONSTANT_BUFFER)?;
            let placement = dynamic_buffer(&device, size_of::<Placement>(), D3D11_BIND_CONSTANT_BUFFER)?;
            let pumpkin = dynamic_buffer(&device, size_of::<PumpkinPlacement>(), D3D11_BIND_CONSTANT_BUFFER)?;

            Ok(Gpu {
                device,
                context,
                factory,
                composition,
                sprite_vertex,
                sprite_fragment,
                sprite_layout: sprite_layout.ok_or_else(Error::empty)?,
                pile_vertex,
                pile_fragment,
                sign_vertex,
                sign_fragment,
                cleaner_vertex,
                cleaner_fragment,
                pumpkin_vertex,
                pumpkin_fragment,
                field_vertex,
                field_fragment,
                glass_vertex,
                glass_fragment,
                winter_scene_vertex,
                winter_scene_fragment,
                adding: adding.ok_or_else(Error::empty)?,
                sampler: sampler.ok_or_else(Error::empty)?,
                bordered: bordered.ok_or_else(Error::empty)?,
                blend: blend.ok_or_else(Error::empty)?,
                raster: raster.ok_or_else(Error::empty)?,
                scissored: scissored.ok_or_else(Error::empty)?,
                uniforms,
                placement,
                pumpkin,
                signs: RefCell::new(HashMap::new()),
                winter_picture: RefCell::new(HashMap::new()),
                pumpkin_pictures: OnceCell::new(),
                cleaner_pictures: OnceCell::new(),
                adapter: String::from_utf16_lossy(&description.Description[..name_length]),
            })
        }
    }

    /// Why the graphics card was lost, as to a driver update or a reset; fine while it is still there.
    pub fn removed(&self) -> Result<()> {
        unsafe { self.device.GetDeviceRemovedReason() }
    }

    fn write<T: Copy>(&self, buffer: &ID3D11Buffer, value: &T) -> Result<()> {
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            mapped.pData.cast::<T>().write(*value);
            self.context.Unmap(buffer, 0);
        }
        Ok(())
    }

    fn sign(&self, caption: usize) -> Option<Picture> {
        let mut signs = self.signs.borrow_mut();
        let picture =
            signs.entry(caption).or_insert_with(|| self.make_sign(caption).inspect_err(|error| log!("sign {caption} failed: {error}")).ok());
        picture.clone()
    }

    fn winter_image(&self, path: &str) -> Option<Picture> {
        let mut cache = self.winter_picture.borrow_mut();
        cache.entry(path.to_owned()).or_insert_with(|| self.load_winter_image(path).inspect_err(|error| log!("winter cleaner image {path} failed: {error}")).ok()).clone()
    }

    fn load_winter_image(&self, path: &str) -> Result<Picture> {
        let name: Vec<u16> = std::ffi::OsStr::new(path).encode_wide().chain([0]).collect();
        let pixels = unsafe {
            let imaging: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let decoder = imaging.CreateDecoderFromFilename(PCWSTR(name.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
            let converter = imaging.CreateFormatConverter()?;
            converter.Initialize(&decoder.GetFrame(0)?, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
            let (mut width, mut height) = (0, 0);
            converter.GetSize(&mut width, &mut height)?;
            if width == 0 || height == 0 || width > 4096 || height > 4096 || u64::from(width) * u64::from(height) > 16_777_216 {
                return Err(Error::empty());
            }
            let mut pixels = vec![0u8; (width * height * 4) as usize];
            converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
            for pixel in pixels.chunks_exact_mut(4) {
                let alpha = pixel[3] as u32;
                for channel in &mut pixel[..3] {
                    *channel = ((*channel as u32 * alpha + 127) / 255) as u8;
                }
            }
            (width, height, pixels)
        };
        let (width, height, pixels) = pixels;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA { pSysMem: pixels.as_ptr().cast(), SysMemPitch: width * 4, SysMemSlicePitch: 0 };
        let mut texture = None;
        let mut view = None;
        unsafe {
            self.device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))?;
            self.device.CreateShaderResourceView(&texture.ok_or_else(Error::empty)?, None, Some(&mut view))?;
        }
        let max_height = 38.0;
        let display_height = (max_height * height as f32 / width.max(1) as f32).clamp(12.0, 38.0);
        Ok((view.ok_or_else(Error::empty)?, [max_height, display_height]))
    }

    fn cleaner_pictures(&self) -> Option<[Picture; 2]> {
        self.cleaner_pictures.get_or_init(|| self.make_cleaner_pictures().inspect_err(|error| log!("cleaner storyboard failed: {error}")).ok()).clone()
    }

    fn make_cleaner_pictures(&self) -> Result<[Picture; 2]> {
        let worker=decode_cleaner_storyboard(include_bytes!("../assets/cleaners/worker-original-sheet.png"))?;
        let picture=self.upload_cleaner_picture(worker)?;
        Ok([picture.clone(),picture])
    }

    fn upload_cleaner_picture(&self, image: CleanerImage) -> Result<Picture> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: image.width,
            Height: image.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA { pSysMem: image.pixels.as_ptr().cast(), SysMemPitch: image.width * 4, SysMemSlicePitch: 0 };
        let mut texture = None;
        let mut view = None;
        unsafe {
            self.device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))?;
            self.device.CreateShaderResourceView(&texture.ok_or_else(Error::empty)?, None, Some(&mut view))?;
        }
        // The source's active sprite is roughly 90 by 140 pixels inside each frame cell.
        // Keeping the character near 80 DIPs tall makes it read clearly at ordinary desktop scale.
        Ok((view.ok_or_else(Error::empty)?, [72.0, 80.0]))
    }

    fn make_sign(&self, caption: usize) -> Result<Picture> {
        let picture = halloween::sign_picture(caption)?;
        let desc = D3D11_TEXTURE2D_DESC {
            Width: picture.width,
            Height: picture.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let data = D3D11_SUBRESOURCE_DATA { pSysMem: picture.pixels.as_ptr().cast(), SysMemPitch: picture.width * 4, SysMemSlicePitch: 0 };
        let mut texture = None;
        let mut view = None;
        unsafe {
            self.device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))?;
            self.device.CreateShaderResourceView(&texture.ok_or_else(Error::empty)?, None, Some(&mut view))?;
        }
        Ok((view.ok_or_else(Error::empty)?, [picture.width as f32 / SIGN_SCALE, picture.height as f32 / SIGN_SCALE]))
    }

    fn pumpkin_pictures(&self) -> Option<&[ID3D11ShaderResourceView; 2]> {
        self.pumpkin_pictures.get_or_init(|| self.make_pumpkin_pictures().inspect_err(|error| log!("pumpkins failed: {error}")).ok()).as_ref()
    }

    fn make_pumpkin_pictures(&self) -> Result<[ID3D11ShaderResourceView; 2]> {
        let [shell, face] = halloween::pumpkin_pictures()?;
        Ok([self.mipmapped(&shell)?, self.mipmapped(&face)?])
    }

    /// A picture with its smaller copies, made on the graphics card: the pumpkins are shown at a fraction of their
    /// pictures' size, and would shimmer without them.
    fn mipmapped(&self, picture: &halloween::Picture) -> Result<ID3D11ShaderResourceView> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: picture.width,
            Height: picture.height,
            MipLevels: 0,
            ArraySize: 1,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
        };
        let mut texture = None;
        let mut view = None;
        unsafe {
            self.device.CreateTexture2D(&desc, None, Some(&mut texture))?;
            let texture = texture.ok_or_else(Error::empty)?;
            self.device.CreateShaderResourceView(&texture, None, Some(&mut view))?;
            let view = view.ok_or_else(Error::empty)?;
            self.context.UpdateSubresource(&texture, 0, None, picture.pixels.as_ptr().cast(), picture.width * 4, 0);
            self.context.GenerateMips(&view);
            Ok(view)
        }
    }
}

/// One display's drawing surface: a swap chain shown through DirectComposition, so it composes with per-pixel alpha.
pub struct Surface {
    swap_chain: IDXGISwapChain1,
    target_view: ID3D11RenderTargetView,
    _target: IDCompositionTarget,
    _visual: IDCompositionVisual,
    /// How much of the snow shows, as it fades out under a full-screen app and back in.
    effect: IDCompositionEffectGroup,
    width: u32,
    height: u32,
    sprites: Option<ID3D11Buffer>,
    sprite_capacity: usize,
    heights: Option<(ID3D11Buffer, ID3D11ShaderResourceView)>,
    height_capacity: usize,
    panel_checked: bool,
    test_last_capture: u32,
    /// The field drops on the glass add up into, as big as the swap chain; made while there are any.
    field: Option<(ID3D11RenderTargetView, ID3D11ShaderResourceView)>,
}

impl Surface {
    /// Snow is soft enough that one pixel per DIP is indistinguishable from the panel's full resolution,
    /// so the swap chain is DIP-sized and composition scales it up.
    pub fn new(gpu: &Gpu, window: HWND, width: u32, height: u32, scale: (f32, f32)) -> Result<Surface> {
        unsafe {
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let swap_chain = gpu.factory.CreateSwapChainForComposition(&gpu.device, &desc, None)?;
            let back: ID3D11Texture2D = swap_chain.GetBuffer(0)?;
            let mut target_view = None;
            gpu.device.CreateRenderTargetView(&back, None, Some(&mut target_view))?;

            let target = gpu.composition.CreateTargetForHwnd(window, true)?;
            let visual = gpu.composition.CreateVisual()?;
            visual.SetContent(&swap_chain)?;
            visual.SetBitmapInterpolationMode(DCOMPOSITION_BITMAP_INTERPOLATION_MODE_LINEAR)?;
            visual.SetTransform2(&Matrix3x2 { M11: scale.0, M12: 0.0, M21: 0.0, M22: scale.1, M31: 0.0, M32: 0.0 })?;
            let effect = gpu.composition.CreateEffectGroup()?;
            visual.SetEffect(&effect)?;
            target.SetRoot(&visual)?;
            gpu.composition.Commit()?;

            Ok(Surface {
                swap_chain,
                target_view: target_view.ok_or_else(Error::empty)?,
                _target: target,
                _visual: visual,
                effect,
                width,
                height,
                sprites: None,
                sprite_capacity: 0,
                heights: None,
                height_capacity: 0,
                panel_checked: false,
                test_last_capture: u32::MAX,
                field: None,
            })
        }
    }

    pub fn set_opacity(&self, gpu: &Gpu, opacity: f32) -> Result<()> {
        unsafe {
            self.effect.SetOpacity2(opacity)?;
            gpu.composition.Commit()
        }
    }

    /// Draws a procedural winter cabin or porch on its own composition surface, which the app places behind the desktop windows.
    pub fn draw_winter_scene(&mut self, gpu: &Gpu, scene: u32, time: f32) -> Result<()> {
        if scene == 0 {
            unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()? };
            return Ok(());
        }
        let uniforms = Uniforms {
            view_size: [self.width as f32, self.height as f32],
            scale: 1.0,
            time,
            column_width: COLUMN_WIDTH,
            column_count: 0,
            pile_top: 0.0,
            occluder_count: 0,
            origin: [0.0; 2],
            pile_bottom: 0.0,
            height_offset: 0,
            light: [1.0; 4],
            shadow_tint: [0.0; 4],
            rim: [0.0; 4],
            lantern: [0.0; 4],
            shake: [0.0; 4],
            pumpkins: [[0.0; 4]; 3],
            occluders: [[0.0; 4]; 16],
            autumn: 0.0,
            blossom: 0.0,
            wilt: 0.0,
            padding: scene as f32,
        };
        gpu.write(&gpu.uniforms, &uniforms)?;
        unsafe {
            self.context_bind_and_draw_winter_scene(gpu);
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()
        }
    }

    unsafe fn context_bind_and_draw_winter_scene(&self, gpu: &Gpu) {
        unsafe {
            let context = &gpu.context;
            context.OMSetRenderTargets(Some(&[Some(self.target_view.clone())]), None);
            context.ClearRenderTargetView(&self.target_view, &[0.0, 0.0, 0.0, 1.0]);
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0, TopLeftY: 0.0, Width: self.width as f32, Height: self.height as f32, MinDepth: 0.0, MaxDepth: 1.0,
            }]));
            context.RSSetState(&gpu.raster);
            context.OMSetBlendState(&gpu.blend, None, u32::MAX);
            context.IASetInputLayout(None);
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            context.VSSetConstantBuffers(0, Some(&[Some(gpu.uniforms.clone())]));
            context.PSSetConstantBuffers(0, Some(&[Some(gpu.uniforms.clone())]));
            context.VSSetShader(&gpu.winter_scene_vertex, None);
            context.PSSetShader(&gpu.winter_scene_fragment, None);
            context.Draw(4, 0);
        }
    }

    /// Small click-through particle surface over the taskbar; no CPU readback or extra simulation.
    pub fn draw_panel_snow(&mut self, gpu: &Gpu, sprites: &[Sprite], size: (u32, u32), time: f32) -> Result<()> {
        self.reserve(gpu, sprites.len(), 0)?;
        let buffer = self.sprites.as_ref().ok_or_else(Error::empty)?;
        let context = &gpu.context;
        let uniforms = Uniforms {
            view_size: [size.0 as f32, size.1 as f32], scale: 1.0, time,
            column_width: COLUMN_WIDTH, column_count: 0, pile_top: 0.0, occluder_count: 0,
            origin: [0.0; 2], pile_bottom: 0.0, height_offset: 0,
            light: [1.0; 4], shadow_tint: [0.0; 4], rim: [0.0; 4], lantern: [0.0; 4],
            shake: [0.0; 4], pumpkins: [[0.0; 4]; 3], occluders: [[0.0; 4]; 16],
            autumn: 0.0, blossom: 0.0, wilt: 0.0, padding: 0.0,
        };
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping(sprites.as_ptr(), mapped.pData.cast::<Sprite>(), sprites.len());
            context.Unmap(buffer, 0);
            context.OMSetRenderTargets(Some(&[Some(self.target_view.clone())]), None);
            context.ClearRenderTargetView(&self.target_view, &[0.0; 4]);
            context.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: self.width as f32, Height: self.height as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
            context.RSSetState(&gpu.raster);
            context.OMSetBlendState(&gpu.blend, None, u32::MAX);
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            gpu.write(&gpu.uniforms, &uniforms)?;
            context.VSSetConstantBuffers(0, Some(&[Some(gpu.uniforms.clone())]));
            context.PSSetConstantBuffers(0, Some(&[Some(gpu.uniforms.clone())]));
            context.IASetInputLayout(&gpu.sprite_layout);
            let stride = size_of::<Sprite>() as u32;
            let offset = 0;
            context.IASetVertexBuffers(0, 1, Some(&Some(buffer.clone())), Some(&stride), Some(&offset));
            context.VSSetShader(&gpu.sprite_vertex, None);
            context.PSSetShader(&gpu.sprite_fragment, None);
            context.DrawInstanced(4, sprites.len() as u32, 0, 0);
            if !self.panel_checked && sprites.iter().any(|sprite| sprite.ghost[0] == -10.0) {
                self.panel_checked = true;
                let check = (|| -> Result<[u8; 4]> {
                    let back: ID3D11Texture2D = self.swap_chain.GetBuffer(0)?;
                    let desc = D3D11_TEXTURE2D_DESC { Width: self.width, Height: self.height, MipLevels: 1, ArraySize: 1,
                        Format: DXGI_FORMAT_B8G8R8A8_UNORM, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        Usage: D3D11_USAGE_STAGING, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, ..Default::default() };
                    let mut staging = None;
                    gpu.device.CreateTexture2D(&desc, None, Some(&mut staging))?;
                    let staging = staging.ok_or_else(Error::empty)?;
                    context.CopyResource(&staging, &back);
                    let mut data = D3D11_MAPPED_SUBRESOURCE::default();
                    context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut data))?;
                    let offset = (self.height as usize / 2) * data.RowPitch as usize + (self.width as usize / 2) * 4;
                    let mut pixel = [0u8; 4];
                    std::ptr::copy_nonoverlapping(data.pData.cast::<u8>().add(offset), pixel.as_mut_ptr(), 4);
                    context.Unmap(&staging, 0);
                    Ok(pixel)
                })();
                match check { Ok(pixel) => log!("taskbar water rendered BGRA {:?}", pixel), Err(error) => log!("taskbar water check failed: {error}") }
            }
            self.swap_chain.Present(0, DXGI_PRESENT(0)).ok()
        }
    }

    pub fn draw(&mut self, gpu: &Gpu, simulation: &SnowSimulation, ambient: Ambient, time: f32) -> Result<()> {
        let mut winter = simulation.winter_sprites();
        winter.extend(simulation.puddle_sprites().into_iter().filter(|sprite| sprite.ghost[0] != -10.0 && sprite.ghost[0] != -11.0));
        winter.extend(simulation.thunder_sprites());
        winter.extend(simulation.tornado_sprites());
        self.reserve(gpu, simulation.sprite_count() + winter.len(), simulation.column_count())?;
        let glass = if simulation.has_glass_drops() {
            let field = self.field(gpu)?;
            simulation.glass_bounds().map(|drops| (field, drops))
        } else {
            self.field = None;
            None
        };
        let (Some(sprite_buffer), Some((height_buffer, height_view))) = (&self.sprites, &self.heights) else {
            return Ok(());
        };
        let context = &gpu.context;
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(sprite_buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            let buffer = std::slice::from_raw_parts_mut(mapped.pData.cast::<Sprite>(), self.sprite_capacity);
            let base_sprites = simulation.write_sprites(buffer);
            buffer[base_sprites..base_sprites + winter.len()].copy_from_slice(&winter);
            let sprites = base_sprites + winter.len();
            context.Unmap(sprite_buffer, 0);

            context.Map(height_buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            let piles = simulation.write_layers(std::slice::from_raw_parts_mut(mapped.pData.cast::<f32>(), self.height_capacity));
            context.Unmap(height_buffer, 0);
            let pumpkins: Vec<PumpkinPlacement> = simulation.pumpkins().iter().map(PumpkinPlacement::new).collect();

            context.OMSetRenderTargets(Some(&[Some(self.target_view.clone())]), None);
            context.ClearRenderTargetView(&self.target_view, &[0.0; 4]);
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: self.width as f32,
                Height: self.height as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }]));
            context.RSSetState(&gpu.raster);
            context.OMSetBlendState(&gpu.blend, None, u32::MAX);
            context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
            let constants = [Some(gpu.uniforms.clone()), Some(gpu.placement.clone()), Some(gpu.pumpkin.clone())];
            context.VSSetConstantBuffers(0, Some(&constants));
            context.PSSetConstantBuffers(0, Some(&constants));

            let (cursor, glow) = ambient.lantern;
            let shake = simulation.shake();
            let mut uniforms = Uniforms {
                view_size: [simulation.width(), simulation.height()],
                scale: 1.0,
                time,
                column_width: COLUMN_WIDTH,
                column_count: 0,
                pile_top: 0.0,
                occluder_count: 0,
                origin: [0.0; 2],
                pile_bottom: 0.0,
                height_offset: 0,
                light: [ambient.light[0], ambient.light[1], ambient.light[2], 1.0],
                shadow_tint: ambient.shadow_tint,
                rim: ambient.rim,
                lantern: [cursor.x, cursor.y, LANTERN_REACH, glow],
                shake: [shake.x, shake.y, 0.0, 0.0],
                pumpkins: std::array::from_fn(|k| pumpkins.get(k).map_or([0.0; 4], |pumpkin| pumpkin.light(glow))),
                occluders: [[0.0; 4]; 16],
                autumn: if simulation.leafy() { 1.0 } else { 0.0 },
                blossom: if simulation.blossom() { 1.0 } else { 0.0 },
                wilt: if simulation.wilts() { 1.0 } else { 0.0 },
                padding: 0.0,
            };

            // The pumpkins go down before the snow, so the drift they sit in covers their bottoms.
            if !pumpkins.is_empty()
                && let Some([shell, face]) = gpu.pumpkin_pictures()
            {
                gpu.write(&gpu.uniforms, &uniforms)?;
                context.IASetInputLayout(None);
                context.VSSetShader(&gpu.pumpkin_vertex, None);
                context.PSSetShader(&gpu.pumpkin_fragment, None);
                context.PSSetSamplers(1, Some(&[Some(gpu.bordered.clone())]));
                context.PSSetShaderResources(2, Some(&[Some(shell.clone()), Some(face.clone())]));
                for pumpkin in &pumpkins {
                    gpu.write(&gpu.pumpkin, pumpkin)?;
                    context.Draw(4, 0);
                }
            }
            if !piles.is_empty() {
                context.IASetInputLayout(None);
                context.VSSetShader(&gpu.pile_vertex, None);
                context.PSSetShader(&gpu.pile_fragment, None);
                context.PSSetShaderResources(0, Some(&[Some(height_view.clone())]));
                for pile in &piles {
                    uniforms.column_count = pile.count as i32;
                    uniforms.pile_top = pile.top + 2.0;
                    uniforms.pile_bottom = pile.bottom;
                    uniforms.origin = [pile.origin.x, pile.origin.y];
                    uniforms.shake[2] = pile.drift;
                    uniforms.height_offset = pile.offset as i32;
                    uniforms.occluder_count = pile.occluders.len().min(16) as i32;
                    uniforms.occluders[..pile.occluders.len().min(16)].copy_from_slice(&pile.occluders[..pile.occluders.len().min(16)]);
                    gpu.write(&gpu.uniforms, &uniforms)?;
                    context.Draw(4, 0);
                }
            }
            uniforms.occluder_count = 0;
            let signs = simulation.signs();
            if !signs.is_empty() {
                gpu.write(&gpu.uniforms, &uniforms)?;
                context.IASetInputLayout(None);
                context.VSSetShader(&gpu.sign_vertex, None);
                context.PSSetShader(&gpu.sign_fragment, None);
                context.PSSetSamplers(0, Some(&[Some(gpu.sampler.clone())]));
                for sign in &signs {
                    let Some((picture, size)) = gpu.sign(sign.caption) else { continue };
                    let placement =
                        Placement { anchor: [sign.anchor.x, sign.anchor.y], size, angle: sign.angle, ground: sign.ground, padding: [0.0; 2] };
                    gpu.write(&gpu.placement, &placement)?;
                    context.PSSetShaderResources(1, Some(&[Some(picture)]));
                    context.Draw(4, 0);
                }
            }
            if sprites > 0 {
                gpu.write(&gpu.uniforms, &uniforms)?;
                let stride = size_of::<Sprite>() as u32;
                let offset = 0;
                context.IASetInputLayout(&gpu.sprite_layout);
                context.IASetVertexBuffers(0, 1, Some(&Some(sprite_buffer.clone())), Some(&stride), Some(&offset));
                // Drops on the glass first add up into their field.
                if let Some(((field_target, _), _)) = &glass {
                    context.OMSetRenderTargets(Some(&[Some(field_target.clone())]), None);
                    context.ClearRenderTargetView(field_target, &[0.0; 4]);
                    context.OMSetBlendState(&gpu.adding, None, u32::MAX);
                    context.VSSetShader(&gpu.field_vertex, None);
                    context.PSSetShader(&gpu.field_fragment, None);
                    context.DrawInstanced(4, sprites as u32, 0, 0);
                    context.OMSetRenderTargets(Some(&[Some(self.target_view.clone())]), None);
                    context.OMSetBlendState(&gpu.blend, None, u32::MAX);
                }
                context.VSSetShader(&gpu.sprite_vertex, None);
                context.PSSetShader(&gpu.sprite_fragment, None);
                context.DrawInstanced(4, sprites as u32, 0, 0);
                if let Some(path) = simulation.winter_image() {
                    if let Some((picture, size)) = gpu.winter_image(path) {
                        context.IASetInputLayout(None);
                        context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
                        context.VSSetShader(&gpu.sign_vertex, None);
                        context.PSSetShader(&gpu.sign_fragment, None);
                        context.PSSetSamplers(0, Some(&[Some(gpu.sampler.clone())]));
                        context.PSSetShaderResources(1, Some(&[Some(picture)]));
                        for cleaner in winter.iter().filter(|sprite| sprite.ghost[0] == -1.0 && sprite.ghost[2] >= 3.5 && sprite.ghost[2] < 4.5) {
                            let width = size[0] * if cleaner.ghost[1] < 0.0 { -1.0 } else { 1.0 };
                            let placement = Placement {
                                anchor: [cleaner.position.x, cleaner.position.y - 12.0],
                                size: [width, size[1]],
                                angle: cleaner.ghost[3],
                                ground: cleaner.position.y - 25.0,
                                padding: [0.0; 2],
                            };
                            gpu.write(&gpu.placement, &placement)?;
                            context.Draw(4, 0);
                        }
                        context.PSSetShaderResources(1, Some(&[None]));
                    }
                }
                if winter.iter().any(|sprite| sprite.ghost[0] == -1.0 && (4.5..6.5).contains(&sprite.ghost[2]))
                    && let Some([snowman_picture, worker_picture]) = gpu.cleaner_pictures()
                {
                    context.IASetInputLayout(None);
                    context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
                    context.VSSetShader(&gpu.cleaner_vertex, None);
                    context.PSSetShader(&gpu.cleaner_fragment, None);
                    context.PSSetSamplers(0, Some(&[Some(gpu.sampler.clone())]));
                    for cleaner in winter.iter().filter(|sprite| sprite.ghost[0] == -1.0 && (4.5..6.5).contains(&sprite.ghost[2])) {
                        let snowman = cleaner.ghost[2] < 5.5;
                        let (picture, frame) = if snowman {
                            // Core's 2.8-second pose cycle defines the snowman's six storyboard beats.
                            let pose = cleaner.pose[1].clamp(0.0, 1.0);
                            let frame = pose;
                            (&snowman_picture.0, frame)
                        } else {
                            // Six walking frames make one relaxed 1.5-second stride cycle.
                            (&worker_picture.0, (time.rem_euclid(4.0 / 1.5) / (4.0 / 1.5) * 30.0).floor())
                        };
                        let width = (if snowman { 86.0 } else { 94.0 }) * if cleaner.ghost[1] < 0.0 { -1.0 } else { 1.0 };
                        let ground = cleaner.position.y - 46.0 * 0.395;
                        let placement = Placement {
                            anchor: [cleaner.position.x, ground],
                            size: [width, 80.0],
                            angle: cleaner.ghost[3] + if snowman && cleaner.pose[1] < 0.42 { (time * 9.0).sin() * 0.035 } else { 0.0 },
                            ground,
                            padding: [frame, if snowman { 2.0 + cleaner.pose[0] } else { 1.0 }],
                        };
                        gpu.write(&gpu.placement, &placement)?;
                        context.PSSetShaderResources(1, Some(&[Some(picture.clone())]));
                        context.Draw(4, 0);
                    }
                    context.PSSetShaderResources(1, Some(&[None]));
                }
                if let Some(((_, field), drops)) = &glass {
                    context.IASetInputLayout(None);
                    context.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                    context.VSSetShader(&gpu.glass_vertex, None);
                    context.PSSetShader(&gpu.glass_fragment, None);
                    context.PSSetShaderResources(4, Some(&[Some(field.clone())]));
                    // The glass shows at most 2 pixels below the drops: drawn within 4 pixels of them, it is all drawn.
                    context.RSSetState(&gpu.scissored);
                    context.RSSetScissorRects(Some(&[self.around(*drops, shake, uniforms.view_size, 4.0)]));
                    context.Draw(3, 0);
                    context.RSSetState(&gpu.raster);
                    // Unbound, it can be drawn into again next frame.
                    context.PSSetShaderResources(4, Some(&[None]));
                }
            }
            let second = time as u32;
            if [3, 13, 29].contains(&second) && self.test_last_capture != second {
                if let Some(dir) = crate::test_capture_dir() {
                    self.test_last_capture = second;
                    let path = dir.join(format!("overlay-{second}-{}x{}.bmp", self.width, self.height));
                    match self.capture_bmp(gpu, &path) {
                        Ok(()) => log!("TEST GPU capture {:?}: sakura={:?}, rain={:?}, panel={}", path, simulation.sakura_character(), simulation.rain_character(), simulation.weather_floor),
                        Err(error) => log!("TEST capture failed: {error}"),
                    }
                }
            }
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()
        }
    }

    fn capture_bmp(&self, gpu: &Gpu, path: &std::path::Path) -> Result<()> {
        unsafe {
            let back: ID3D11Texture2D = self.swap_chain.GetBuffer(0)?;
            let desc = D3D11_TEXTURE2D_DESC { Width: self.width, Height: self.height, MipLevels: 1, ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM, SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, ..Default::default() };
            let mut staging = None;
            gpu.device.CreateTexture2D(&desc, None, Some(&mut staging))?;
            let staging = staging.ok_or_else(Error::empty)?;
            gpu.context.CopyResource(&staging, &back);
            let mut data = D3D11_MAPPED_SUBRESOURCE::default();
            gpu.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut data))?;
            let bytes = self.width * self.height * 4;
            let mut bmp = Vec::with_capacity(bytes as usize + 54);
            bmp.extend_from_slice(b"BM"); bmp.extend_from_slice(&(bytes + 54).to_le_bytes());
            bmp.extend_from_slice(&[0;4]); bmp.extend_from_slice(&54u32.to_le_bytes());
            bmp.extend_from_slice(&40u32.to_le_bytes()); bmp.extend_from_slice(&(self.width as i32).to_le_bytes());
            bmp.extend_from_slice(&(-(self.height as i32)).to_le_bytes());
            bmp.extend_from_slice(&1u16.to_le_bytes()); bmp.extend_from_slice(&32u16.to_le_bytes());
            bmp.extend_from_slice(&0u32.to_le_bytes()); bmp.extend_from_slice(&bytes.to_le_bytes()); bmp.extend_from_slice(&[0;16]);
            for row in 0..self.height as usize {
                bmp.extend_from_slice(std::slice::from_raw_parts(data.pData.cast::<u8>().add(row * data.RowPitch as usize), self.width as usize * 4));
            }
            gpu.context.Unmap(&staging, 0);
            std::fs::create_dir_all(path.parent().ok_or_else(Error::empty)?).map_err(|_| Error::empty())?;
            std::fs::write(path, bmp).map_err(|_| Error::empty())?;
            Ok(())
        }
    }

    /// The swap chain's pixels within `margin` of `bounds`, left, bottom, right, top in the simulation's points.
    fn around(&self, bounds: [f32; 4], shake: V2, view: [f32; 2], margin: f32) -> RECT {
        let (width, height) = (self.width as f32, self.height as f32);
        let (across, up) = (width / view[0], height / view[1]);
        let left = ((bounds[0] + shake.x) * across - margin).floor().clamp(0.0, width);
        let right = ((bounds[2] + shake.x) * across + margin).ceil().clamp(0.0, width);
        let top = (height - (bounds[3] + shake.y) * up - margin).floor().clamp(0.0, height);
        let bottom = (height - (bounds[1] + shake.y) * up + margin).ceil().clamp(0.0, height);
        RECT { left: left as i32, top: top as i32, right: right as i32, bottom: bottom as i32 }
    }

    /// The field drops on the glass add up into, made the first time it is needed.
    fn field(&mut self, gpu: &Gpu) -> Result<(ID3D11RenderTargetView, ID3D11ShaderResourceView)> {
        if let Some(field) = &self.field {
            return Ok(field.clone());
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: self.width,
            Height: self.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R16G16_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut texture = None;
        let mut target = None;
        let mut view = None;
        unsafe {
            gpu.device.CreateTexture2D(&desc, None, Some(&mut texture))?;
            let texture = texture.ok_or_else(Error::empty)?;
            gpu.device.CreateRenderTargetView(&texture, None, Some(&mut target))?;
            gpu.device.CreateShaderResourceView(&texture, None, Some(&mut view))?;
        }
        let field = (target.ok_or_else(Error::empty)?, view.ok_or_else(Error::empty)?);
        self.field = Some(field.clone());
        Ok(field)
    }

    fn reserve(&mut self, gpu: &Gpu, sprites: usize, columns: usize) -> Result<()> {
        if sprites > self.sprite_capacity || self.sprites.is_none() {
            self.sprite_capacity = 1024.max(sprites * 3 / 2);
            self.sprites = Some(dynamic_buffer(&gpu.device, self.sprite_capacity * size_of::<Sprite>(), D3D11_BIND_VERTEX_BUFFER)?);
        }
        if columns > self.height_capacity || self.heights.is_none() {
            self.height_capacity = 256.max(columns);
            let buffer = dynamic_buffer(&gpu.device, self.height_capacity * size_of::<f32>(), D3D11_BIND_SHADER_RESOURCE)?;
            let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R32_FLOAT,
                ViewDimension: D3D_SRV_DIMENSION_BUFFER,
                Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                    Buffer: D3D11_BUFFER_SRV {
                        Anonymous1: D3D11_BUFFER_SRV_0 { FirstElement: 0 },
                        Anonymous2: D3D11_BUFFER_SRV_1 { NumElements: self.height_capacity as u32 },
                    },
                },
            };
            let mut view = None;
            unsafe { gpu.device.CreateShaderResourceView(&buffer, Some(&desc), Some(&mut view))? };
            self.heights = Some((buffer, view.ok_or_else(Error::empty)?));
        }
        Ok(())
    }
}

/// The shaders' bytecode, compiled once: it doesn't depend on the device, so a new one after the old was lost reuses it.
struct Code {
    sprite_vertex: Vec<u8>,
    sprite_fragment: Vec<u8>,
    pile_vertex: Vec<u8>,
    pile_fragment: Vec<u8>,
    sign_vertex: Vec<u8>,
    sign_fragment: Vec<u8>,
    cleaner_vertex: Vec<u8>,
    cleaner_fragment: Vec<u8>,
    pumpkin_vertex: Vec<u8>,
    pumpkin_fragment: Vec<u8>,
    field_vertex: Vec<u8>,
    field_fragment: Vec<u8>,
    glass_vertex: Vec<u8>,
    glass_fragment: Vec<u8>,
    winter_scene_vertex: Vec<u8>,
    winter_scene_fragment: Vec<u8>,
}

impl Code {
    fn get() -> Result<&'static Code> {
        static CODE: OnceLock<Code> = OnceLock::new();
        if let Some(code) = CODE.get() {
            return Ok(code);
        }
        let code = Code {
            sprite_vertex: compile(s!("spriteVertex"), s!("vs_4_0"))?,
            sprite_fragment: compile(s!("spriteFragment"), s!("ps_4_0"))?,
            pile_vertex: compile(s!("pileVertex"), s!("vs_4_0"))?,
            pile_fragment: compile(s!("pileFragment"), s!("ps_4_0"))?,
            sign_vertex: compile(s!("signVertex"), s!("vs_4_0"))?,
            sign_fragment: compile(s!("signFragment"), s!("ps_4_0"))?,
            cleaner_vertex: compile(s!("cleanerVertex"), s!("vs_4_0"))?,
            cleaner_fragment: compile(s!("cleanerFragment"), s!("ps_4_0"))?,
            pumpkin_vertex: compile(s!("pumpkinVertex"), s!("vs_4_0"))?,
            pumpkin_fragment: compile(s!("pumpkinFragment"), s!("ps_4_0"))?,
            field_vertex: compile(s!("fieldVertex"), s!("vs_4_0"))?,
            field_fragment: compile(s!("fieldFragment"), s!("ps_4_0"))?,
            glass_vertex: compile(s!("glassVertex"), s!("vs_4_0"))?,
            glass_fragment: compile(s!("glassFragment"), s!("ps_4_0"))?,
            winter_scene_vertex: compile(s!("winterSceneVertex"), s!("vs_4_0"))?,
            winter_scene_fragment: compile(s!("winterSceneFragment"), s!("ps_4_0"))?,
        };
        Ok(CODE.get_or_init(|| code))
    }
}

fn compile(entry: PCSTR, target: PCSTR) -> Result<Vec<u8>> {
    let mut code = None;
    let mut errors = None;
    let result = unsafe {
        D3DCompile(
            SHADERS.as_ptr().cast(),
            SHADERS.len(),
            s!("shaders.hlsl"),
            None,
            None,
            entry,
            target,
            D3DCOMPILE_OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            Some(&mut errors),
        )
    };
    if let Err(error) = result {
        let message = errors.as_ref().map(|blob| String::from_utf8_lossy(blob_bytes(blob)).into_owned()).unwrap_or_default();
        return Err(Error::new(error.code(), message));
    }
    code.map(|blob| blob_bytes(&blob).to_vec()).ok_or_else(Error::empty)
}

fn blob_bytes(blob: &ID3DBlob) -> &[u8] {
    unsafe { std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize()) }
}

fn vertex_shader(device: &ID3D11Device, code: &[u8]) -> Result<ID3D11VertexShader> {
    let mut shader = None;
    unsafe { device.CreateVertexShader(code, None, Some(&mut shader))? };
    shader.ok_or_else(Error::empty)
}

fn pixel_shader(device: &ID3D11Device, code: &[u8]) -> Result<ID3D11PixelShader> {
    let mut shader = None;
    unsafe { device.CreatePixelShader(code, None, Some(&mut shader))? };
    shader.ok_or_else(Error::empty)
}

fn dynamic_buffer(device: &ID3D11Device, length: usize, bind: D3D11_BIND_FLAG) -> Result<ID3D11Buffer> {
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: length as u32,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buffer = None;
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buffer))? };
    buffer.ok_or_else(Error::empty)
}
