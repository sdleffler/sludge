use crate::{
    assets::{Asset, Cache, Cached, Key, Loaded},
    ecs::{ScContext, SmartComponent},
    filesystem::Filesystem,
    math::*,
    resources::Resources,
};
use {
    anyhow::*,
    derivative::*,
    lyon::{
        math::*,
        tessellation::{self as t, FillOptions, StrokeOptions},
    },
    miniquad as mq,
    rlua::prelude::*,
    serde::{Deserialize, Serialize},
    std::{
        any::{self, Any, TypeId},
        cmp::Ordering,
        fmt,
        hash::{Hash, Hasher},
        io::Read,
        marker::PhantomData,
        mem, ops,
        sync::{
            atomic::{self, AtomicBool},
            Arc, RwLock,
        },
    },
    thunderdome::{self, Arena, Index},
};

pub mod shader {
    use super::*;

    pub const BASIC_VERTEX: &'static str = include_str!("graphics/basic_es300.glslv");
    pub const BASIC_FRAGMENT: &'static str = include_str!("graphics/basic_es300.glslf");

    pub fn meta() -> mq::ShaderMeta {
        mq::ShaderMeta {
            images: vec!["t_Texture".to_string()],
            uniforms: mq::UniformBlockLayout {
                uniforms: vec![mq::UniformDesc::new("u_MVP", mq::UniformType::Mat4)],
            },
        }
    }

    #[repr(C)]
    pub struct Uniforms {
        pub mvp: Matrix4<f32>,
    }

    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct Vertex {
        pub pos: Vector3<f32>,
        pub uv: Vector2<f32>,
        pub color: LinearColor,
    }

    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct InstanceProperties {
        pub src: Vector4<f32>,
        pub tx: Matrix4<f32>,
        pub color: LinearColor,
    }
}

pub use shader::{InstanceProperties, Uniforms, Vertex};

// FIXME(sleffy): we aren't actually using `OwnedBuffer` and `Buffer` anywhere
/// An `OwnedBuffer` represents either a VertexBuffer or an IndexBuffer, and
/// can be used in a binding to a render pipeline
#[derive(Debug)]
pub struct OwnedBuffer {
    pub buffer: mq::Buffer,
}

impl From<mq::Buffer> for OwnedBuffer {
    fn from(buffer: mq::Buffer) -> Self {
        Self { buffer }
    }
}

impl Drop for OwnedBuffer {
    fn drop(&mut self) {
        self.buffer.delete();
    }
}

/// `Buffer` is a convenience type that represents a safe way to create
/// multiple asynchronous references to an `OwnedBuffer` object
#[derive(Debug, Clone)]
pub struct Buffer {
    pub shared: Arc<OwnedBuffer>,
}

impl ops::Deref for Buffer {
    type Target = OwnedBuffer;

    fn deref(&self) -> &Self::Target {
        &*self.shared
    }
}

impl From<mq::Buffer> for Buffer {
    fn from(buffer: mq::Buffer) -> Self {
        Self {
            shared: Arc::new(OwnedBuffer::from(buffer)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FilterMode {
    Nearest,
    Linear,
}

/// A `Texture` is a safe type used to obtain asynchronous references to an
/// `OwnedTexture`
#[derive(Debug)]
pub struct Texture {
    pub handle: mq::Texture,
}

impl Texture {
    /// Create a texture from a given buffer of RGBA image data.
    pub fn from_rgba8(ctx: &mut Graphics, width: u16, height: u16, bytes: &[u8]) -> Self {
        let tex = mq::Texture::from_rgba8(&mut ctx.mq, width, height, bytes);
        tex.set_filter(&mut ctx.mq, mq::FilterMode::Nearest);
        Self::from_inner(tex)
    }

    /// Parse a buffer containing the raw contents of an image file such as a PNG, GIF, etc.
    pub fn from_memory(ctx: &mut Graphics, buffer: &[u8]) -> Result<Self> {
        let rgba_image = image::load_from_memory(buffer)?.to_rgba();
        Ok(Self::from_rgba8(
            ctx,
            rgba_image.width() as u16,
            rgba_image.height() as u16,
            &rgba_image.to_vec(),
        ))
    }

    /// Parse a reader such as a `File` into a texture.
    pub fn from_reader<R: Read>(ctx: &mut Graphics, reader: &mut R) -> Result<Self> {
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Self::from_memory(ctx, &buf)
    }

    pub fn from_inner(handle: mq::Texture) -> Self {
        Self { handle }
    }

    pub fn set_filter_mode(&self, ctx: &mut Graphics, filter_mode: FilterMode) {
        self.handle.set_filter(
            &mut ctx.mq,
            match filter_mode {
                FilterMode::Nearest => mq::FilterMode::Nearest,
                FilterMode::Linear => mq::FilterMode::Linear,
            },
        );
    }

    pub fn width(&self) -> u32 {
        self.handle.width
    }

    pub fn height(&self) -> u32 {
        self.handle.height
    }
}

impl Drop for Texture {
    fn drop(&mut self) {
        self.handle.delete();
    }
}

impl Drawable for Texture {
    fn draw(&self, ctx: &mut Graphics, param: InstanceParam) {
        ctx.quad_bindings.vertex_buffers[1].update(
            &mut ctx.mq,
            &[param
                .scale2(Vector2::new(self.width() as f32, self.height() as f32))
                .scale2(param.src.extents())
                .to_instance_properties()],
        );
        ctx.quad_bindings.images[0] = self.handle;
        ctx.mq.apply_bindings(&ctx.quad_bindings);
        ctx.mq.draw(0, 6, 1);
    }
}

/// `Pipeline` represents an identifier for the current graphics pipeline
#[derive(Debug, Clone)]
pub struct Pipeline {
    pub mq: mq::Pipeline,
}

#[derive(Debug, Clone)]
pub struct RenderPass {
    pub shared: Arc<mq::RenderPass>,
}

impl AsRef<RenderPass> for RenderPass {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl ops::Deref for RenderPass {
    type Target = mq::RenderPass;

    fn deref(&self) -> &Self::Target {
        &*self.shared
    }
}

impl RenderPass {
    pub fn new(
        ctx: &mut Graphics,
        color_img: Texture,
        depth_img: impl Into<Option<Texture>>,
    ) -> Self {
        Self::from_parts(ctx, color_img.handle, depth_img.into().map(|di| di.handle))
    }

    pub fn from_parts(
        ctx: &mut Graphics,
        color_img: mq::Texture,
        depth_img: Option<mq::Texture>,
    ) -> Self {
        let render_pass = mq::RenderPass::new(&mut ctx.mq, color_img, depth_img);
        let shared = Arc::new(render_pass);
        let this = Self { shared };
        ctx.register_render_pass(this.clone());
        this
    }
}

/// A type that represents the different types of actions one can apply
/// to a frame buffer at the start of a render pass. Mainly used to initialize
/// a framebuffer to some specific state
#[derive(Debug, Copy, Clone)]
pub enum PassAction {
    Nothing,
    Clear {
        color: Option<LinearColor>,
        depth: Option<f32>,
        stencil: Option<i32>,
    },
}

impl PassAction {
    pub fn clear_color(color: Color) -> PassAction {
        PassAction::Clear {
            color: Some(color.into()),
            depth: Some(1.),
            stencil: None,
        }
    }
}

impl Default for PassAction {
    fn default() -> PassAction {
        PassAction::Clear {
            color: Some(Color::ZEROS.into()),
            depth: Some(1.),
            stencil: None,
        }
    }
}

impl From<PassAction> for mq::PassAction {
    fn from(action: PassAction) -> Self {
        match action {
            PassAction::Nothing => mq::PassAction::Nothing,
            PassAction::Clear {
                color,
                depth,
                stencil,
            } => mq::PassAction::Clear {
                color: color.map(|c| (c.r, c.g, c.b, c.a)),
                depth,
                stencil,
            },
        }
    }
}

/// A RGBA color in the `sRGB` color space represented as `f32`'s in the range `[0.0-1.0]`
///
/// For convenience, [`WHITE`](constant.WHITE.html) and [`BLACK`](constant.BLACK.html) are provided.
#[derive(Copy, Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Color {
    /// Red component
    pub r: f32,
    /// Green component
    pub g: f32,
    /// Blue component
    pub b: f32,
    /// Alpha component
    pub a: f32,
}

impl Color {
    pub const ZEROS: Color = Color::new(0.0, 0.0, 0.0, 0.0);
    pub const WHITE: Color = Color::new(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Color = Color::new(0.0, 0.0, 0.0, 1.0);
    pub const RED: Color = Color::new(1.0, 0.0, 0.0, 1.0);
    pub const GREEN: Color = Color::new(0.0, 1.0, 0.0, 1.0);
    pub const BLUE: Color = Color::new(0.0, 0.0, 1.0, 1.0);
    pub const YELLOW: Color = Color::new(1.0, 1.0, 0.0, 1.0);
    pub const MAGENTA: Color = Color::new(1.0, 0.0, 1.0, 1.0);
    pub const CYAN: Color = Color::new(0.0, 1.0, 1.0, 1.0);

    /// Create a new `Color` from four `f32`'s in the range `[0.0-1.0]`
    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }

    /// Create a new `Color` from four `u8`'s in the range `[0-255]`
    pub fn from_rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color::from((r, g, b, a))
    }

    /// Create a new `Color` from three u8's in the range `[0-255]`,
    /// with the alpha component fixed to 255 (opaque)
    pub fn from_rgb(r: u8, g: u8, b: u8) -> Color {
        Color::from((r, g, b))
    }

    /// Return a tuple of four `u8`'s in the range `[0-255]` with the `Color`'s
    /// components.
    pub fn to_rgba(self) -> (u8, u8, u8, u8) {
        self.into()
    }

    /// Return a tuple of three `u8`'s in the range `[0-255]` with the `Color`'s
    /// components.
    pub fn to_rgb(self) -> (u8, u8, u8) {
        self.into()
    }

    /// Convert a packed `u32` containing `0xRRGGBBAA` into a `Color`
    pub fn from_rgba_u32(c: u32) -> Color {
        let c = c.to_be_bytes();

        Color::from((c[0], c[1], c[2], c[3]))
    }

    /// Convert a packed `u32` containing `0x00RRGGBB` into a `Color`.
    /// This lets you do things like `Color::from_rgb_u32(0xCD09AA)` easily if you want.
    pub fn from_rgb_u32(c: u32) -> Color {
        let c = c.to_be_bytes();

        Color::from((c[1], c[2], c[3]))
    }

    /// Convert a `Color` into a packed `u32`, containing `0xRRGGBBAA` as bytes.
    pub fn to_rgba_u32(self) -> u32 {
        let (r, g, b, a): (u8, u8, u8, u8) = self.into();

        u32::from_be_bytes([r, g, b, a])
    }

    /// Convert a `Color` into a packed `u32`, containing `0x00RRGGBB` as bytes.
    pub fn to_rgb_u32(self) -> u32 {
        let (r, g, b, _a): (u8, u8, u8, u8) = self.into();

        u32::from_be_bytes([0, r, g, b])
    }
}

impl From<(u8, u8, u8, u8)> for Color {
    /// Convert a `(R, G, B, A)` tuple of `u8`'s in the range `[0-255]` into a `Color`
    fn from(val: (u8, u8, u8, u8)) -> Self {
        let (r, g, b, a) = val;
        let rf = (f32::from(r)) / 255.0;
        let gf = (f32::from(g)) / 255.0;
        let bf = (f32::from(b)) / 255.0;
        let af = (f32::from(a)) / 255.0;
        Color::new(rf, gf, bf, af)
    }
}

impl From<(u8, u8, u8)> for Color {
    /// Convert a `(R, G, B)` tuple of `u8`'s in the range `[0-255]` into a `Color`,
    /// with a value of 255 for the alpha element (i.e., no transparency.)
    fn from(val: (u8, u8, u8)) -> Self {
        let (r, g, b) = val;
        Color::from((r, g, b, 255))
    }
}

impl From<[f32; 4]> for Color {
    /// Turns an `[R, G, B, A] array of `f32`'s into a `Color` with no format changes.
    /// All inputs should be in the range `[0.0-1.0]`.
    fn from(val: [f32; 4]) -> Self {
        Color::new(val[0], val[1], val[2], val[3])
    }
}

impl From<(f32, f32, f32)> for Color {
    /// Convert a `(R, G, B)` tuple of `f32`'s in the range `[0.0-1.0]` into a `Color`,
    /// with a value of 1.0 to for the alpha element (ie, no transparency.)
    fn from(val: (f32, f32, f32)) -> Self {
        let (r, g, b) = val;
        Color::new(r, g, b, 1.0)
    }
}

impl From<(f32, f32, f32, f32)> for Color {
    /// Convert a `(R, G, B, A)` tuple of `f32`'s in the range `[0.0-1.0]` into a `Color`
    fn from(val: (f32, f32, f32, f32)) -> Self {
        let (r, g, b, a) = val;
        Color::new(r, g, b, a)
    }
}

impl From<Color> for (u8, u8, u8, u8) {
    /// Convert a `Color` into a `(R, G, B, A)` tuple of `u8`'s in the range of `[0-255]`.
    fn from(color: Color) -> Self {
        let r = (color.r * 255.0) as u8;
        let g = (color.g * 255.0) as u8;
        let b = (color.b * 255.0) as u8;
        let a = (color.a * 255.0) as u8;
        (r, g, b, a)
    }
}

impl From<Color> for (u8, u8, u8) {
    /// Convert a `Color` into a `(R, G, B)` tuple of `u8`'s in the range of `[0-255]`,
    /// ignoring the alpha term.
    fn from(color: Color) -> Self {
        let (r, g, b, _) = color.into();
        (r, g, b)
    }
}

impl From<Color> for [f32; 4] {
    /// Convert a `Color` into an `[R, G, B, A]` array of `f32`'s in the range of `[0.0-1.0]`.
    fn from(color: Color) -> Self {
        [color.r, color.g, color.b, color.a]
    }
}

/// A RGBA color in the *linear* color space,
/// suitable for shoving into a shader.
#[derive(Copy, Clone, PartialEq, Debug, Serialize, Deserialize)]
#[repr(C)]
pub struct LinearColor {
    /// Red component
    pub r: f32,
    /// Green component
    pub g: f32,
    /// Blue component
    pub b: f32,
    /// Alpha component
    pub a: f32,
}

impl LinearColor {
    pub const BLACK: LinearColor = LinearColor {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 1.,
    };

    pub const WHITE: LinearColor = LinearColor {
        r: 1.,
        g: 1.,
        b: 1.,
        a: 1.,
    };
}

impl From<Color> for LinearColor {
    /// Convert an (sRGB) Color into a linear color,
    /// per https://en.wikipedia.org/wiki/Srgb#The_reverse_transformation
    fn from(c: Color) -> Self {
        fn f(component: f32) -> f32 {
            let a = 0.055;
            if component <= 0.04045 {
                component / 12.92
            } else {
                ((component + a) / (1.0 + a)).powf(2.4)
            }
        }
        LinearColor {
            r: f(c.r),
            g: f(c.g),
            b: f(c.b),
            a: c.a,
        }
    }
}

impl From<LinearColor> for Color {
    fn from(c: LinearColor) -> Self {
        fn f(component: f32) -> f32 {
            let a = 0.055;
            if component <= 0.003_130_8 {
                component * 12.92
            } else {
                (1.0 + a) * component.powf(1.0 / 2.4)
            }
        }
        Color {
            r: f(c.r),
            g: f(c.g),
            b: f(c.b),
            a: c.a,
        }
    }
}

impl From<LinearColor> for [f32; 4] {
    fn from(color: LinearColor) -> Self {
        [color.r, color.g, color.b, color.a]
    }
}

/// `BlendEquation` represents the different types of equations that can be
/// used to blend colors
#[derive(Debug, Clone, Copy)]
pub enum BlendEquation {
    Add,
    Sub,
    ReverseSub,
}

impl From<BlendEquation> for mq::Equation {
    fn from(beq: BlendEquation) -> Self {
        match beq {
            BlendEquation::Add => mq::Equation::Add,
            BlendEquation::Sub => mq::Equation::Subtract,
            BlendEquation::ReverseSub => mq::Equation::ReverseSubtract,
        }
    }
}

/// `BlendFactor` represents the different factors that can be used when
/// blending two colors
#[derive(Debug, Clone, Copy)]
pub enum BlendFactor {
    Zero,
    One,
    SourceColor,
    SourceAlpha,
    DestinationColor,
    DestinationAlpha,
    OneMinusSourceColor,
    OneMinusSourceAlpha,
    OneMinusDestinationColor,
    OneMinusDestinationAlpha,
    SourceAlphaSaturate,
}

impl From<BlendFactor> for mq::BlendFactor {
    fn from(bf: BlendFactor) -> Self {
        use {
            mq::{BlendFactor as MqBf, BlendValue as MqBv},
            BlendFactor::*,
        };

        match bf {
            Zero => MqBf::Zero,
            One => MqBf::One,
            SourceColor => MqBf::Value(MqBv::SourceColor),
            SourceAlpha => MqBf::Value(MqBv::SourceAlpha),
            DestinationColor => MqBf::Value(MqBv::DestinationColor),
            DestinationAlpha => MqBf::Value(MqBv::DestinationAlpha),
            OneMinusSourceColor => MqBf::OneMinusValue(MqBv::SourceColor),
            OneMinusSourceAlpha => MqBf::OneMinusValue(MqBv::SourceAlpha),
            OneMinusDestinationColor => MqBf::OneMinusValue(MqBv::DestinationColor),
            OneMinusDestinationAlpha => MqBf::OneMinusValue(MqBv::DestinationAlpha),
            SourceAlphaSaturate => MqBf::SourceAlphaSaturate,
        }
    }
}

/// `BlendMode` represents a struct that encapsulates all of the different
/// fields required to blend two colors
#[derive(Debug, Copy, Clone)]
pub struct BlendMode {
    eq: BlendEquation,
    src: BlendFactor,
    dst: BlendFactor,
}

impl Default for BlendMode {
    fn default() -> Self {
        Self::new(
            BlendEquation::Add,
            BlendFactor::SourceAlpha,
            BlendFactor::OneMinusSourceAlpha,
        )
    }
}

impl BlendMode {
    pub fn new(eq: BlendEquation, src: BlendFactor, dst: BlendFactor) -> Self {
        Self { eq, src, dst }
    }
}

impl From<BlendMode> for mq::BlendState {
    fn from(bm: BlendMode) -> Self {
        mq::BlendState::new(bm.eq.into(), bm.src.into(), bm.dst.into())
    }
}

/// Specifies whether a mesh should be drawn
/// filled or as an outline.
#[derive(Debug, Copy, Clone)]
pub enum DrawMode {
    /// A stroked line with given parameters, see `StrokeOptions` documentation.
    Stroke(StrokeOptions),
    /// A filled shape with given parameters, see `FillOptions` documentation.
    Fill(FillOptions),
}

impl DrawMode {
    /// Constructs a DrawMode that draws a stroke with the given width
    pub fn stroke(width: f32) -> DrawMode {
        DrawMode::Stroke(StrokeOptions::default().with_line_width(width))
    }

    /// Constructs a DrawMode that fills shapes with default fill options.
    pub fn fill() -> DrawMode {
        DrawMode::Fill(FillOptions::default())
    }
}

#[derive(Debug, Copy, Clone)]
struct VertexBuilder {
    color: LinearColor,
}

impl t::BasicVertexConstructor<Vertex> for VertexBuilder {
    #[inline]
    fn new_vertex(&mut self, point: Point) -> Vertex {
        Vertex {
            pos: Vector3::new(point.x, point.y, 0.),
            uv: Vector2::new(point.x, point.y),
            color: self.color,
        }
    }
}

impl t::FillVertexConstructor<Vertex> for VertexBuilder {
    #[inline]
    fn new_vertex(&mut self, point: Point, _attributes: t::FillAttributes) -> Vertex {
        Vertex {
            pos: Vector3::new(point.x, point.y, 0.),
            uv: Vector2::new(point.x, point.y),
            color: self.color,
        }
    }
}

impl t::StrokeVertexConstructor<Vertex> for VertexBuilder {
    #[inline]
    fn new_vertex(&mut self, point: Point, _attributes: t::StrokeAttributes) -> Vertex {
        Vertex {
            pos: Vector3::new(point.x, point.y, 0.),
            uv: Vector2::zeros(),
            color: self.color,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TransformStack {
    ts: Vec<Matrix4<f32>>,
}

impl TransformStack {
    pub fn new() -> Self {
        Self {
            ts: vec![Matrix4::identity()],
        }
    }

    #[inline]
    pub fn top(&self) -> &Matrix4<f32> {
        self.ts.last().unwrap()
    }

    #[inline]
    pub fn top_mut(&mut self) -> &mut Matrix4<f32> {
        self.ts.last_mut().unwrap()
    }

    #[inline]
    pub fn translate2(&mut self, v: &Vector2<f32>) -> &mut Self {
        *self.top_mut() *= Translation3::from(v.push(0.)).to_homogeneous();
        self
    }

    #[inline]
    pub fn scale2(&mut self, v: &Vector2<f32>) -> &mut Self {
        *self.top_mut() *= Matrix3::from_diagonal(&v.push(1.)).to_homogeneous();
        self
    }

    #[inline]
    pub fn rotate2(&mut self, angle: f32) -> &mut Self {
        *self.top_mut() *= UnitComplex::new(angle).to_homogeneous().to_homogeneous();
        self
    }

    #[inline]
    pub fn push(&mut self, tx: impl Into<Option<Matrix4<f32>>>) {
        self.ts.push(tx.into().unwrap_or(*self.top()));
    }

    #[inline]
    pub fn pop(&mut self) {
        self.ts.pop().expect("popped empty transform stack");
    }

    #[inline]
    pub fn scope<T, F>(&mut self, thunk: F) -> T
    where
        F: FnOnce(&mut TransformStack) -> T,
    {
        self.push(None);
        let result = thunk(self);
        self.pop();
        result
    }
}

/// The main graphics struct combines a bunch of mq types and the
/// model view matrix to represent a basic context that can be drawn into
#[derive(Derivative)]
#[derivative(Debug)]
pub struct Graphics {
    #[derivative(Debug = "ignore")]
    pub mq: mq::Context,
    pub pipeline: mq::Pipeline,
    pub null_texture: Cached<Texture>,
    pub projection: Matrix4<f32>,
    pub modelview: TransformStack,
    pub quad_bindings: mq::Bindings,
    pub render_passes: Vec<RenderPass>,
}

impl Graphics {
    pub fn new(mut mq: mq::Context) -> Result<Self> {
        let shader = mq::Shader::new(
            &mut mq,
            shader::BASIC_VERTEX,
            shader::BASIC_FRAGMENT,
            shader::meta(),
        )?;

        let pipeline = mq::Pipeline::with_params(
            &mut mq,
            &[
                mq::BufferLayout::default(),
                mq::BufferLayout {
                    step_func: mq::VertexStep::PerInstance,
                    ..mq::BufferLayout::default()
                },
            ],
            &[
                mq::VertexAttribute::with_buffer("a_Pos", mq::VertexFormat::Float3, 0),
                mq::VertexAttribute::with_buffer("a_Uv", mq::VertexFormat::Float2, 0),
                mq::VertexAttribute::with_buffer("a_VertColor", mq::VertexFormat::Float4, 0),
                mq::VertexAttribute::with_buffer("a_Src", mq::VertexFormat::Float4, 1),
                mq::VertexAttribute::with_buffer("a_Tx", mq::VertexFormat::Mat4, 1),
                mq::VertexAttribute::with_buffer("a_Color", mq::VertexFormat::Float4, 1),
            ],
            shader,
            mq::PipelineParams {
                color_blend: Some(BlendMode::default().into()),
                depth_test: mq::Comparison::LessOrEqual,
                depth_write: true,
                ..mq::PipelineParams::default()
            },
        );

        let null_texture = Texture::from_inner(mq::Texture::from_rgba8(
            &mut mq,
            1,
            1,
            &[255, 255, 255, 255],
        ));

        let quad_vertices =
            mq::Buffer::immutable(&mut mq, mq::BufferType::VertexBuffer, &quad_vertices());
        let quad_indices =
            mq::Buffer::immutable(&mut mq, mq::BufferType::IndexBuffer, &quad_indices());

        let instances = mq::Buffer::stream(
            &mut mq,
            mq::BufferType::VertexBuffer,
            mem::size_of::<InstanceProperties>(),
        );

        let quad_bindings = mq::Bindings {
            vertex_buffers: vec![quad_vertices, instances],
            index_buffer: quad_indices,
            images: vec![null_texture.handle],
        };

        Ok(Self {
            mq,
            pipeline,
            null_texture: null_texture.into(),
            projection: Matrix4::identity(),
            modelview: TransformStack::new(),
            quad_bindings,
            render_passes: Vec::new(),
        })
    }

    #[inline]
    pub(crate) fn register_render_pass(&mut self, pass: RenderPass) {
        self.render_passes.push(pass);
    }

    #[inline]
    pub(crate) fn expire_render_passes(&mut self) {
        for pass in self
            .render_passes
            .drain_filter(|rp| Arc::strong_count(&rp.shared) == 1)
        {
            pass.shared.delete(&mut self.mq);
        }
    }

    #[inline]
    pub fn transforms(&mut self) -> &mut TransformStack {
        &mut self.modelview
    }

    #[inline]
    pub fn mul_transform(&mut self, tx: Matrix4<f32>) {
        *self.modelview.top_mut() *= tx;
    }

    #[inline]
    pub fn push_multiplied_transform(&mut self, tx: Matrix4<f32>) {
        let mult = self.modelview.top() * tx;
        self.modelview.push(mult);
    }

    #[inline]
    pub fn apply_transforms(&mut self) {
        let mvp = self.projection * self.modelview.top();
        self.mq.apply_uniforms(&shader::Uniforms { mvp });
    }

    #[inline]
    pub fn push_transform(&mut self, tx: impl Into<Option<Matrix4<f32>>>) {
        self.modelview.push(tx);
    }

    #[inline]
    pub fn pop_transform(&mut self) {
        self.modelview.pop();
    }

    #[inline]
    pub fn set_projection<M>(&mut self, projection: M)
    where
        M: Into<Matrix4<f32>>,
    {
        self.projection = projection.into();
    }

    #[inline]
    pub fn apply_default_pipeline(&mut self) {
        self.mq.apply_pipeline(&self.pipeline);
    }

    #[inline]
    pub fn apply_pipeline(&mut self, pipeline: &Pipeline) {
        self.mq.apply_pipeline(&pipeline.mq);
    }

    #[inline]
    pub fn commit_frame(&mut self) {
        self.mq.commit_frame();
        self.expire_render_passes();
    }

    #[inline]
    pub fn begin_default_pass(&mut self, action: PassAction) {
        self.mq.begin_default_pass(action.into());
    }

    #[inline]
    pub fn begin_pass(&mut self, pass: &impl AsRef<RenderPass>, action: PassAction) {
        self.mq
            .begin_pass(**pass.as_ref(), mq::PassAction::from(action));
    }

    #[inline]
    pub fn end_pass(&mut self) {
        self.mq.end_render_pass();
    }

    #[inline]
    pub fn draw<D, P>(&mut self, drawable: &D, param: P)
    where
        D: Drawable + ?Sized,
        P: Into<Option<InstanceParam>>,
    {
        drawable.draw(self, param.into().unwrap_or_default());
    }

    #[inline]
    pub fn set_blend(&mut self, blend: Option<BlendMode>) {
        self.mq.set_blend(blend.map(mq::BlendState::from), None);
    }

    #[inline]
    pub fn get_screen_size(&self) -> (f32, f32) {
        self.mq.screen_size()
    }
}

#[derive(Debug)]
pub struct Mesh {
    /// The shared reference to the texture, so that it doesn't get dropped and deleted.
    /// The inner data is already in `bindings` so this is really just to keep it from
    /// being dropped.
    pub texture: Cached<Texture>,
    pub bindings: mq::Bindings,
    pub len: i32,
    pub aabb: Box2<f32>,
}

impl Drawable for Mesh {
    fn draw(&self, ctx: &mut Graphics, param: InstanceParam) {
        self.bindings.vertex_buffers[1].update(&mut ctx.mq, &[param.to_instance_properties()]);
        ctx.mq.apply_bindings(&self.bindings);
        ctx.mq.draw(0, self.len, 1);
    }
}

#[derive(Debug)]
pub struct MeshBuilder {
    pub buffer: t::geometry_builder::VertexBuffers<Vertex, u16>,
    pub texture: Cached<Texture>,
}

impl MeshBuilder {
    pub fn new<T>(texture: T) -> Self
    where
        T: Into<Cached<Texture>>,
    {
        Self {
            buffer: t::VertexBuffers::new(),
            texture: texture.into(),
        }
    }

    /// Create a new mesh for a line of one or more connected segments.
    pub fn line<P>(&mut self, points: &[P], width: f32, color: Color) -> Result<&mut Self>
    where
        P: Into<mint::Point2<f32>> + Clone,
    {
        self.polyline(DrawMode::stroke(width), points, color)
    }

    /// Create a new mesh for a series of connected lines.
    pub fn polyline<P>(&mut self, mode: DrawMode, points: &[P], color: Color) -> Result<&mut Self>
    where
        P: Into<mint::Point2<f32>> + Clone,
    {
        ensure!(
            points.len() >= 2,
            "MeshBuilder::polyline() got a list of < 2 points"
        );
        self.polyline_inner(mode, points, false, color)
    }

    /// Create a new mesh for a circle.
    ///
    /// For the meaning of the `tolerance` parameter, [see here](https://docs.rs/lyon_geom/0.11.0/lyon_geom/#flattening).
    pub fn circle<P>(
        &mut self,
        mode: DrawMode,
        point: P,
        radius: f32,
        tolerance: f32,
        color: Color,
    ) -> &mut Self
    where
        P: Into<mint::Point2<f32>>,
    {
        {
            let point = point.into();
            let buffers = &mut self.buffer;
            let vb = VertexBuilder {
                color: LinearColor::from(color),
            };
            match mode {
                DrawMode::Fill(fill_options) => {
                    let builder = &mut t::BuffersBuilder::new(buffers, vb);
                    let _ = t::basic_shapes::fill_circle(
                        t::math::point(point.x, point.y),
                        radius,
                        &fill_options.with_tolerance(tolerance),
                        builder,
                    );
                }
                DrawMode::Stroke(options) => {
                    let builder = &mut t::BuffersBuilder::new(buffers, vb);
                    let _ = t::basic_shapes::stroke_circle(
                        t::math::point(point.x, point.y),
                        radius,
                        &options.with_tolerance(tolerance),
                        builder,
                    );
                }
            };
        }
        self
    }

    /// Create a new mesh for a closed polygon.
    /// The points given must be in clockwise order,
    /// otherwise at best the polygon will not draw.
    pub fn polygon<P>(&mut self, mode: DrawMode, points: &[P], color: Color) -> Result<&mut Self>
    where
        P: Into<mint::Point2<f32>> + Clone,
    {
        ensure!(
            points.len() >= 3,
            "MeshBuilder::polygon() got a list of < 3 points"
        );

        self.polyline_inner(mode, points, true, color)
    }

    fn polyline_inner<P>(
        &mut self,
        mode: DrawMode,
        points: &[P],
        is_closed: bool,
        color: Color,
    ) -> Result<&mut Self>
    where
        P: Into<mint::Point2<f32>> + Clone,
    {
        {
            assert!(points.len() > 1);
            let buffers = &mut self.buffer;
            let points = points.iter().cloned().map(|p| {
                let mint_point: mint::Point2<f32> = p.into();
                t::math::point(mint_point.x, mint_point.y)
            });
            let vb = VertexBuilder {
                color: LinearColor::from(color),
            };
            match mode {
                DrawMode::Fill(options) => {
                    let builder = &mut t::BuffersBuilder::new(buffers, vb);
                    let tessellator = &mut t::FillTessellator::new();
                    t::basic_shapes::fill_polyline(points, tessellator, &options, builder)
                }
                DrawMode::Stroke(options) => {
                    let builder = &mut t::BuffersBuilder::new(buffers, vb);
                    t::basic_shapes::stroke_polyline(points, is_closed, &options, builder)
                }
            }
            .map_err(|e| anyhow!("error during tessellation: {:?}", e))?;
        }
        Ok(self)
    }

    /// Create a new mesh for a rectangle.
    pub fn rectangle(&mut self, mode: DrawMode, bounds: Box2<f32>, color: Color) -> &mut Self {
        {
            let buffers = &mut self.buffer;
            let extents = bounds.extents();
            let rect = t::math::rect(bounds.mins.x, bounds.mins.y, extents.x, extents.y);
            let vb = VertexBuilder {
                color: LinearColor::from(color),
            };
            match mode {
                DrawMode::Fill(fill_options) => {
                    let builder = &mut t::BuffersBuilder::new(buffers, vb);
                    let _ = t::basic_shapes::fill_rectangle(&rect, &fill_options, builder);
                }
                DrawMode::Stroke(options) => {
                    let builder = &mut t::BuffersBuilder::new(buffers, vb);
                    let _ = t::basic_shapes::stroke_rectangle(&rect, &options, builder);
                }
            };
        }
        self
    }

    /// Creates a `Mesh` from a raw list of triangles defined from vertices
    /// and indices.  You may also
    /// supply an `Image` to use as a texture, if you pass `None`, it will
    /// just use a pure white texture.
    ///
    /// This is the most primitive mesh-creation method, but allows you full
    /// control over the tesselation and texturing.  It has the same constraints
    /// as `Mesh::from_raw()`.
    pub fn raw<V, T>(&mut self, verts: &[V], indices: &[u16], texture: T) -> &mut Self
    where
        V: Into<Vertex> + Clone,
        T: Into<Option<Cached<Texture>>>,
    {
        assert!(self.buffer.vertices.len() + verts.len() < (std::u16::MAX as usize));
        assert!(self.buffer.indices.len() + indices.len() < (std::u16::MAX as usize));
        let next_idx = self.buffer.vertices.len() as u16;
        // Can we remove the clone here?
        // I can't find a way to, because `into()` consumes its source and
        // `Borrow` or `AsRef` aren't really right.
        let vertices = verts.iter().cloned().map(|v: V| -> Vertex { v.into() });
        let indices = indices.iter().map(|i| (*i) + next_idx);
        self.buffer.vertices.extend(vertices);
        self.buffer.indices.extend(indices);

        if let Some(tex) = texture.into() {
            self.texture = tex;
        }

        self
    }

    pub fn build(&self, ctx: &mut Graphics) -> Mesh {
        let vertex_buffer = mq::Buffer::immutable(
            &mut ctx.mq,
            mq::BufferType::VertexBuffer,
            &self.buffer.vertices,
        );

        let index_buffer = mq::Buffer::immutable(
            &mut ctx.mq,
            mq::BufferType::IndexBuffer,
            &self.buffer.indices,
        );

        let instance = mq::Buffer::stream(
            &mut ctx.mq,
            mq::BufferType::VertexBuffer,
            mem::size_of::<InstanceProperties>(),
        );

        let aabb = Box2::from_points(
            &self
                .buffer
                .vertices
                .iter()
                .map(|v| Point2::from(v.pos.xy()))
                .collect::<Vec<_>>(),
        );

        Mesh {
            texture: self.texture.clone(),
            bindings: mq::Bindings {
                vertex_buffers: vec![vertex_buffer, instance],
                index_buffer,
                images: vec![self.texture.load().handle],
            },
            len: self.buffer.indices.len() as i32,
            aabb,
        }
    }
}

/// A type used to store additional basic parameters for types that need it
#[derive(Debug, Copy, Clone)]
pub struct InstanceParam {
    pub src: Box2<f32>,
    pub tx: Transform3<f32>,
    pub color: Color,
}

impl Default for InstanceParam {
    fn default() -> Self {
        Self {
            src: Box2::new(0., 0., 1., 1.),
            tx: Transform3::identity(),
            color: Color::WHITE,
        }
    }
}

impl InstanceParam {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn src(self, src: Box2<f32>) -> Self {
        Self { src, ..self }
    }

    #[inline]
    pub fn color(self, color: Color) -> Self {
        Self { color, ..self }
    }

    #[inline]
    pub fn rotate2(self, angle: f32) -> Self {
        Self {
            tx: self.tx
                * Transform3::from_matrix_unchecked(homogeneous_mat3_to_mat4(
                    &Rotation2::new(angle).to_homogeneous(),
                )),
            ..self
        }
    }

    #[inline]
    pub fn translate2(self, v: Vector2<f32>) -> Self {
        Self {
            tx: self.tx * Translation3::new(v.x, v.y, 0.),
            ..self
        }
    }

    #[inline]
    pub fn scale2(self, v: Vector2<f32>) -> Self {
        Self {
            tx: self.tx
                * Transform3::from_matrix_unchecked(Matrix4::from_diagonal(&v.push(1.).push(1.))),
            ..self
        }
    }

    #[inline]
    pub fn translate3(self, v: Vector3<f32>) -> Self {
        Self {
            tx: self.tx * Translation3::from(v),
            ..self
        }
    }

    #[inline]
    pub fn prepend_transform(self, tx: &Transform3<f32>) -> Self {
        Self {
            tx: self.tx * tx,
            ..self
        }
    }

    #[inline]
    pub fn to_instance_properties(&self) -> InstanceProperties {
        let mins = self.src.mins;
        let extents = self.src.extents();
        InstanceProperties {
            src: Vector4::new(mins.x, mins.y, extents.x, extents.y),
            tx: *self.tx.matrix(),
            color: LinearColor::from(self.color),
        }
    }

    #[inline]
    pub fn transform_aabb(&self, aabb: &Box2<f32>) -> Box2<f32> {
        aabb.transformed_by(self.tx.matrix())
    }
}

fn quad_vertices() -> [Vertex; 4] {
    [
        Vertex {
            pos: Vector3::new(0., 0., 0.),
            uv: Vector2::new(0., 0.),
            color: Color::WHITE.into(),
        },
        Vertex {
            pos: Vector3::new(1., 0., 0.),
            uv: Vector2::new(1., 0.),
            color: Color::WHITE.into(),
        },
        Vertex {
            pos: Vector3::new(1., 1., 0.),
            uv: Vector2::new(1., 1.),
            color: Color::WHITE.into(),
        },
        Vertex {
            pos: Vector3::new(0., 1., 0.),
            uv: Vector2::new(0., 1.),
            color: Color::WHITE.into(),
        },
    ]
}

fn quad_indices() -> [u16; 6] {
    [0, 1, 2, 0, 2, 3]
}

/// Represents the index of a `Sprite` within a `SpriteBatch`
#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct SpriteId(Index);

impl<'a> SmartComponent<ScContext<'a>> for SpriteId {}

pub struct SpriteBatchIter<'a> {
    iter: thunderdome::Iter<'a, InstanceParam>,
}

impl<'a> Iterator for SpriteBatchIter<'a> {
    type Item = (SpriteId, &'a InstanceParam);

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|(i, v)| (SpriteId(i), v))
    }
}

pub struct SpriteBatchIterMut<'a> {
    iter: thunderdome::IterMut<'a, InstanceParam>,
}

impl<'a> Iterator for SpriteBatchIterMut<'a> {
    type Item = (SpriteId, &'a mut InstanceParam);

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|(i, v)| (SpriteId(i), v))
    }
}

#[derive(Debug)]
struct SpriteBatchInner {
    // Used to store the result of converting InstanceParams to InstanceProperties
    instances: Vec<InstanceProperties>,
    /// Capacity is used to store the length of the buffers inside of mq::Bindings
    capacity: usize,
    bindings: mq::Bindings,
}

#[derive(Debug)]
pub struct SpriteBatch {
    sprites: Arena<InstanceParam>,
    inner: RwLock<SpriteBatchInner>,
    dirty: AtomicBool,
    texture: Cached<Texture>,
}

impl ops::Index<SpriteId> for SpriteBatch {
    type Output = InstanceParam;

    #[inline]
    fn index(&self, index: SpriteId) -> &Self::Output {
        &self.sprites[index.0]
    }
}

impl ops::IndexMut<SpriteId> for SpriteBatch {
    #[inline]
    fn index_mut(&mut self, index: SpriteId) -> &mut Self::Output {
        self.dirty = AtomicBool::new(true);
        &mut self.sprites[index.0]
    }
}

impl SpriteBatch {
    pub fn new<T>(ctx: &mut Graphics, texture: T) -> Self
    where
        T: Into<Cached<Texture>>,
    {
        const DEFAULT_SPRITEBATCH_CAPACITY: usize = 64;
        Self::with_capacity(ctx, texture, DEFAULT_SPRITEBATCH_CAPACITY)
    }

    pub fn with_capacity<T>(ctx: &mut Graphics, texture: T, capacity: usize) -> Self
    where
        T: Into<Cached<Texture>>,
    {
        let mut texture = texture.into();

        let instances = mq::Buffer::stream(
            &mut ctx.mq,
            mq::BufferType::VertexBuffer,
            capacity * mem::size_of::<InstanceProperties>(),
        );

        let bindings = mq::Bindings {
            vertex_buffers: vec![ctx.quad_bindings.vertex_buffers[0], instances],
            index_buffer: ctx.quad_bindings.index_buffer,
            images: vec![texture.load_cached().handle],
        };

        Self {
            sprites: Arena::new(),
            inner: SpriteBatchInner {
                instances: Vec::new(),
                capacity,
                bindings,
            }
            .into(),
            dirty: AtomicBool::new(true),
            texture,
        }
    }

    #[inline]
    pub fn insert(&mut self, param: InstanceParam) -> SpriteId {
        *self.dirty.get_mut() = true;
        SpriteId(self.sprites.insert(param))
    }

    #[inline]
    pub fn remove(&mut self, index: SpriteId) {
        *self.dirty.get_mut() = true;
        self.sprites.remove(index.0);
    }

    #[inline]
    pub fn clear(&mut self) {
        *self.dirty.get_mut() = true;
        self.sprites.clear();
    }

    #[inline]
    pub fn texture(&self) -> &Cached<Texture> {
        &self.texture
    }

    #[inline]
    pub fn set_texture(&mut self, texture: impl Into<Cached<Texture>>) {
        *self.dirty.get_mut() = true;
        self.texture = texture.into();
    }

    pub fn flush(&self, ctx: &mut Graphics) {
        if !self.dirty.load(atomic::Ordering::Relaxed) {
            return;
        }

        let inner = &mut *self.inner.write().unwrap();
        let texture = self.texture.load();

        inner.instances.clear();
        inner
            .instances
            .extend(self.sprites.iter().map(|(_, param)| {
                param
                    .scale2(param.src.extents())
                    .scale2(Vector2::new(
                        texture.width() as f32,
                        texture.height() as f32,
                    ))
                    .to_instance_properties()
            }));

        if inner.instances.len() > inner.capacity {
            let new_capacity = inner.instances.len().checked_next_power_of_two().unwrap();
            let new_buffer = mq::Buffer::stream(
                &mut ctx.mq,
                mq::BufferType::VertexBuffer,
                new_capacity * mem::size_of::<InstanceProperties>(),
            );

            let old_buffer = mem::replace(&mut inner.bindings.vertex_buffers[1], new_buffer);
            old_buffer.delete();

            inner.capacity = new_capacity;
        }

        inner.bindings.vertex_buffers[1].update(&mut ctx.mq, &inner.instances);
        inner.bindings.images[0] = texture.handle;

        self.dirty.store(false, atomic::Ordering::Relaxed);
    }

    pub fn iter(&self) -> SpriteBatchIter<'_> {
        SpriteBatchIter {
            iter: self.sprites.iter(),
        }
    }

    pub fn iter_mut(&mut self) -> SpriteBatchIterMut<'_> {
        SpriteBatchIterMut {
            iter: self.sprites.iter_mut(),
        }
    }
}

/// TODO: FIXME(sleffy) maybe? This implementation ignores the color and src parameters
/// of the `InstanceParam`. Not sure there's much to be done about that, though, since
/// the spritebatch has its own instance parameters.
impl Drawable for SpriteBatch {
    fn draw(&self, ctx: &mut Graphics, instance: InstanceParam) {
        self.flush(ctx);
        let inner = self.inner.read().unwrap();

        ctx.push_multiplied_transform(instance.tx.to_homogeneous());
        ctx.mq.apply_bindings(&inner.bindings);
        ctx.apply_transforms();
        // 6 here because a quad is 6 vertices
        ctx.mq.draw(0, 6, inner.instances.len() as i32);
        ctx.pop_transform();
        ctx.apply_transforms();
    }
}

#[derive(Debug)]
pub struct Canvas {
    pub render_pass: RenderPass,
    pub color_buffer: Texture,
    pub depth_buffer: Texture,
}

impl AsRef<RenderPass> for Canvas {
    fn as_ref(&self) -> &RenderPass {
        &self.render_pass
    }
}

impl Canvas {
    pub fn new(ctx: &mut Graphics, width: u32, height: u32) -> Self {
        let color_img = Texture::from_inner(mq::Texture::new_render_texture(
            &mut ctx.mq,
            mq::TextureParams {
                width,
                height,
                format: mq::TextureFormat::RGBA8,
                filter: mq::FilterMode::Nearest,
                ..Default::default()
            },
        ));

        let depth_img = Texture::from_inner(mq::Texture::new_render_texture(
            &mut ctx.mq,
            mq::TextureParams {
                width,
                height,
                format: mq::TextureFormat::Depth,
                filter: mq::FilterMode::Nearest,
                ..Default::default()
            },
        ));

        let render_pass = RenderPass::from_parts(ctx, color_img.handle, Some(depth_img.handle));

        Self {
            render_pass,
            color_buffer: color_img,
            depth_buffer: depth_img,
        }
    }
}

impl Drawable for Canvas {
    fn draw(&self, ctx: &mut Graphics, instance: InstanceParam) {
        self.color_buffer.draw(ctx, instance);
    }
}

#[derive(Debug, Clone)]
pub struct Sprite {
    pub params: InstanceParam,
    pub texture: Cached<Texture>,
}

impl Sprite {
    pub fn new(texture: Texture, params: InstanceParam) -> Self {
        Self {
            params,
            texture: Cached::new(texture),
        }
    }

    pub fn from_cached(texture: Cached<Texture>, params: InstanceParam) -> Self {
        Self { params, texture }
    }
}

/// FIXME(sleffy): same issue as the SpriteBatch implementation, ignoring
/// the passed-in src/color params
impl Drawable for Sprite {
    fn draw(&self, ctx: &mut Graphics, instance: InstanceParam) {
        let params = InstanceParam {
            tx: instance.tx * self.params.tx,
            ..self.params
        };
        self.texture.load().draw(ctx, params);
    }
}

pub trait Drawable {
    fn draw(&self, ctx: &mut Graphics, instance: InstanceParam);
}

impl Drawable for () {
    fn draw(&self, _ctx: &mut Graphics, _instance: InstanceParam) {}
}

/// Shorthand trait for types that are `Drawable` and `Any`, as well as
/// `Send + Sync`. This is blanket-impl'd and you should never have to implement
/// it manually.
pub trait AnyDrawable: Drawable + Any + Send + Sync {
    #[doc(hidden)]
    fn as_any(&self) -> &dyn Any;

    #[doc(hidden)]
    fn as_any_mut(&mut self) -> &mut dyn Any;

    #[doc(hidden)]
    fn to_box_any(self: Box<Self>) -> Box<dyn Any>;

    #[doc(hidden)]
    fn as_drawable(&self) -> &dyn Drawable;
}

impl<T: Drawable + Any + Send + Sync> AnyDrawable for T {
    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self as &mut dyn Any
    }

    fn to_box_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }

    fn as_drawable(&self) -> &dyn Drawable {
        self as &dyn Drawable
    }
}

impl dyn AnyDrawable {
    #[doc(hidden)]
    pub fn downcast<T: Any>(self: Box<Self>) -> Option<T> {
        Box::<dyn Any>::downcast(self.to_box_any())
            .map(|boxed| *boxed)
            .ok()
    }
}

pub struct DrawableId<T: AnyDrawable + ?Sized, C>(
    pub(crate) Index,
    pub(crate) PhantomData<&'static T>,
    pub(crate) PhantomData<C>,
);

impl<T: AnyDrawable + ?Sized, C> fmt::Debug for DrawableId<T, C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple(&format!(
            "DrawableId<{}, {}>",
            any::type_name::<T>(),
            any::type_name::<C>()
        ))
        .field(&self.0)
        .finish()
    }
}

impl<T: AnyDrawable + ?Sized, C> Copy for DrawableId<T, C> {}
impl<T: AnyDrawable + ?Sized, C> Clone for DrawableId<T, C> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: AnyDrawable + ?Sized, C> PartialEq for DrawableId<T, C> {
    fn eq(&self, rhs: &Self) -> bool {
        self.0 == rhs.0
    }
}

impl<T: AnyDrawable + ?Sized, C> Eq for DrawableId<T, C> {}

impl<T: AnyDrawable + ?Sized, C> PartialOrd for DrawableId<T, C> {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.0.cmp(&rhs.0))
    }
}

impl<T: AnyDrawable + ?Sized, C> Ord for DrawableId<T, C> {
    fn cmp(&self, rhs: &Self) -> Ordering {
        self.0.cmp(&rhs.0)
    }
}

impl<T: AnyDrawable + ?Sized, C> Hash for DrawableId<T, C> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<T: AnyDrawable + ?Sized, C> DrawableId<T, C> {
    pub(crate) fn new(index: Index) -> Self {
        Self(index, PhantomData, PhantomData)
    }
}

pub struct ErasedDrawableId<C>(pub(crate) Index, pub(crate) PhantomData<C>);

impl<C> fmt::Debug for ErasedDrawableId<C> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_tuple(&format!("ErasedDrawableId<{}>", any::type_name::<C>()))
            .field(&self.0)
            .finish()
    }
}

impl<C> Copy for ErasedDrawableId<C> {}
impl<C> Clone for ErasedDrawableId<C> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<C> PartialEq for ErasedDrawableId<C> {
    fn eq(&self, rhs: &Self) -> bool {
        self.0 == rhs.0
    }
}

impl<C> Eq for ErasedDrawableId<C> {}

impl<C> PartialOrd for ErasedDrawableId<C> {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.0.cmp(&rhs.0))
    }
}

impl<C> Ord for ErasedDrawableId<C> {
    fn cmp(&self, rhs: &Self) -> Ordering {
        self.0.cmp(&rhs.0)
    }
}

impl<C> Hash for ErasedDrawableId<C> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl<C> ErasedDrawableId<C> {
    pub(crate) fn new(index: Index) -> Self {
        Self(index, PhantomData)
    }
}

impl<T: AnyDrawable + ?Sized, C> From<DrawableId<T, C>> for ErasedDrawableId<C> {
    fn from(id: DrawableId<T, C>) -> ErasedDrawableId<C> {
        Self::new(id.0)
    }
}

impl<T: AnyDrawable + ?Sized, C> From<DrawableId<T, C>> for Option<ErasedDrawableId<C>> {
    fn from(id: DrawableId<T, C>) -> Option<ErasedDrawableId<C>> {
        Some(ErasedDrawableId::new(id.0))
    }
}

impl Asset for Texture {
    fn load<'a, R: Resources<'a>>(
        key: &Key,
        _cache: &Cache<'a, R>,
        resources: &R,
    ) -> Result<Loaded<Self>> {
        let path = key
            .to_path()
            .with_context(|| anyhow!("bad key for Texture"))?;
        let (fs, gfx) = resources.fetch::<(Filesystem, Graphics)>()?;
        let mut file = fs.borrow_mut().open(path)?;
        let texture = Texture::from_reader(&mut gfx.borrow_mut(), &mut file)
            .with_context(|| anyhow!("failed to create a texture using {:?}", path))?;
        Ok(Loaded::new(texture))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LuaDrawableIdUserData {
    drawable_id: Index,
    value_type: TypeId,
    context_type: TypeId,
}

impl LuaUserData for LuaDrawableIdUserData {}

impl<'lua, T: AnyDrawable + ?Sized, C: 'static> ToLua<'lua> for DrawableId<T, C> {
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        LuaDrawableIdUserData {
            drawable_id: self.0,
            value_type: TypeId::of::<T>(),
            context_type: TypeId::of::<C>(),
        }
        .to_lua(lua)
    }
}

impl<'lua, T: AnyDrawable + ?Sized, C: 'static> FromLua<'lua> for DrawableId<T, C> {
    fn from_lua(value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        let ldiu = LuaDrawableIdUserData::from_lua(value, lua)?;

        if TypeId::of::<T>() != ldiu.value_type {
            return Err(LuaError::FromLuaConversionError {
                from: "LuaDrawableIdUserData.value_type",
                to: any::type_name::<T>(),
                message: None,
            });
        }

        if TypeId::of::<C>() != ldiu.context_type {
            return Err(LuaError::FromLuaConversionError {
                from: "LuaDrawableIdUserData.context_type",
                to: any::type_name::<C>(),
                message: None,
            });
        }

        Ok(DrawableId::new(ldiu.drawable_id))
    }
}

impl<'lua, C: 'static> ToLua<'lua> for ErasedDrawableId<C> {
    fn to_lua(self, lua: LuaContext<'lua>) -> LuaResult<LuaValue<'lua>> {
        LuaDrawableIdUserData {
            drawable_id: self.0,
            value_type: TypeId::of::<dyn AnyDrawable>(),
            context_type: TypeId::of::<C>(),
        }
        .to_lua(lua)
    }
}

impl<'lua, C: 'static> FromLua<'lua> for ErasedDrawableId<C> {
    fn from_lua(value: LuaValue<'lua>, lua: LuaContext<'lua>) -> LuaResult<Self> {
        let ldiu = LuaDrawableIdUserData::from_lua(value, lua)?;

        if TypeId::of::<dyn AnyDrawable>() != ldiu.value_type {
            return Err(LuaError::FromLuaConversionError {
                from: "LuaDrawableIdUserData.value_type",
                to: any::type_name::<dyn AnyDrawable>(),
                message: None,
            });
        }

        if TypeId::of::<C>() != ldiu.context_type {
            return Err(LuaError::FromLuaConversionError {
                from: "LuaDrawableIdUserData.context_type",
                to: any::type_name::<C>(),
                message: None,
            });
        }

        Ok(ErasedDrawableId::new(ldiu.drawable_id))
    }
}
