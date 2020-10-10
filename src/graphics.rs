use crate::math::*;
use {
    anyhow::*,
    lyon::{
        math::*,
        tessellation::{self as t, FillOptions, StrokeOptions},
    },
    miniquad as mq,
    serde::{Deserialize, Serialize},
    std::{mem, ops, sync::Arc},
    thunderdome::{Arena, Index},
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
        pub pos: Vector2<f32>,
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

#[derive(Debug)]
pub struct OwnedBuffer {
    pub buffer: mq::Buffer,
}

impl From<mq::Buffer> for OwnedBuffer {
    fn from(buffer: mq::Buffer) -> Self {
        Self { buffer }
    }
}

impl ops::Deref for OwnedBuffer {
    type Target = mq::Buffer;

    fn deref(&self) -> &Self::Target {
        &self.buffer
    }
}

impl Drop for OwnedBuffer {
    fn drop(&mut self) {
        self.buffer.delete();
    }
}

#[derive(Debug, Clone)]
pub struct Buffer {
    pub shared: Arc<OwnedBuffer>,
}

impl ops::Deref for Buffer {
    type Target = mq::Buffer;

    fn deref(&self) -> &Self::Target {
        &self.shared.buffer
    }
}

impl From<mq::Buffer> for Buffer {
    fn from(buffer: mq::Buffer) -> Self {
        Self {
            shared: Arc::new(OwnedBuffer::from(buffer)),
        }
    }
}

#[derive(Debug)]
pub struct OwnedTexture {
    pub texture: mq::Texture,
}

impl From<mq::Texture> for OwnedTexture {
    fn from(texture: mq::Texture) -> Self {
        Self { texture }
    }
}

impl ops::Deref for OwnedTexture {
    type Target = mq::Texture;

    fn deref(&self) -> &Self::Target {
        &self.texture
    }
}

impl Drop for OwnedTexture {
    fn drop(&mut self) {
        self.texture.delete();
    }
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub shared: Arc<OwnedTexture>,
}

impl ops::Deref for Texture {
    type Target = mq::Texture;

    fn deref(&self) -> &Self::Target {
        &self.shared.texture
    }
}

impl From<mq::Texture> for Texture {
    fn from(texture: mq::Texture) -> Self {
        Self {
            shared: Arc::new(OwnedTexture::from(texture)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Pipeline {
    pub mq: mq::Pipeline,
}

#[derive(Debug, Clone)]
pub struct RenderPass {
    pub shared: Arc<mq::RenderPass>,
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
        let render_pass =
            mq::RenderPass::new(&mut ctx.mq, *color_img, depth_img.into().map(|di| *di));
        let this = Self {
            shared: Arc::new(render_pass),
        };
        ctx.register_render_pass(this.clone());
        this
    }
}

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
            color: Some(Color::BLACK.into()),
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
    fn new_vertex(&mut self, point: Point) -> Vertex {
        Vertex {
            pos: Vector2::new(point.x, point.y),
            uv: Vector2::new(point.x, point.y),
            color: self.color,
        }
    }
}

impl t::FillVertexConstructor<Vertex> for VertexBuilder {
    fn new_vertex(&mut self, point: Point, _attributes: t::FillAttributes) -> Vertex {
        Vertex {
            pos: Vector2::new(point.x, point.y),
            uv: Vector2::new(point.x, point.y),
            color: self.color,
        }
    }
}

impl t::StrokeVertexConstructor<Vertex> for VertexBuilder {
    fn new_vertex(&mut self, point: Point, _attributes: t::StrokeAttributes) -> Vertex {
        Vertex {
            pos: Vector2::new(point.x, point.y),
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

    pub fn top(&self) -> &Matrix4<f32> {
        self.ts.last().unwrap()
    }

    pub fn top_mut(&mut self) -> &mut Matrix4<f32> {
        self.ts.last_mut().unwrap()
    }

    pub fn translate2(&mut self, v: &Vector2<f32>) -> &mut Self {
        *self.top_mut() *= Translation3::from(v.push(0.)).to_homogeneous();
        self
    }

    pub fn scale2(&mut self, v: &Vector2<f32>) -> &mut Self {
        *self.top_mut() *= Matrix3::from_diagonal(&v.push(1.)).to_homogeneous();
        self
    }

    pub fn rotate2(&mut self, angle: f32) -> &mut Self {
        *self.top_mut() *= UnitComplex::new(angle).to_homogeneous().to_homogeneous();
        self
    }

    pub fn push(&mut self) {
        self.ts.push(*self.top());
    }

    pub fn pop(&mut self) {
        self.ts.pop().expect("popped empty transform stack");
    }

    pub fn scope<T, F>(&mut self, thunk: F) -> T
    where
        F: FnOnce(&mut TransformStack) -> T,
    {
        self.push();
        let result = thunk(self);
        self.pop();
        result
    }
}

pub struct Graphics {
    pub mq: mq::Context,
    pub pipeline: mq::Pipeline,
    pub null_texture: Texture,
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

        let pipeline = mq::Pipeline::new(
            &mut mq,
            &[
                mq::BufferLayout::default(),
                mq::BufferLayout {
                    step_func: mq::VertexStep::PerInstance,
                    ..mq::BufferLayout::default()
                },
            ],
            &[
                mq::VertexAttribute::with_buffer("a_Pos", mq::VertexFormat::Float2, 0),
                mq::VertexAttribute::with_buffer("a_Uv", mq::VertexFormat::Float2, 0),
                mq::VertexAttribute::with_buffer("a_VertColor", mq::VertexFormat::Float4, 0),
                mq::VertexAttribute::with_buffer("a_Src", mq::VertexFormat::Float4, 1),
                mq::VertexAttribute::with_buffer("a_Tx", mq::VertexFormat::Mat4, 1),
                mq::VertexAttribute::with_buffer("a_Color", mq::VertexFormat::Float4, 1),
            ],
            shader,
        );

        let null_texture = Texture::from(mq::Texture::from_rgba8(
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
            images: vec![*null_texture],
        };

        Ok(Self {
            mq,
            pipeline,
            null_texture,
            projection: Matrix4::identity(),
            modelview: TransformStack::new(),
            quad_bindings,
            render_passes: Vec::new(),
        })
    }

    pub(crate) fn register_render_pass(&mut self, pass: RenderPass) {
        self.render_passes.push(pass);
    }

    pub(crate) fn expire_render_passes(&mut self) {
        for pass in self
            .render_passes
            .drain_filter(|rp| Arc::strong_count(&rp.shared) == 1)
        {
            pass.shared.delete(&mut self.mq);
        }
    }

    pub fn transforms(&mut self) -> &mut TransformStack {
        &mut self.modelview
    }

    pub fn apply_transforms(&mut self) {
        let mvp = self.projection * self.modelview.top();
        self.mq.apply_uniforms(&shader::Uniforms { mvp });
    }

    pub fn set_projection<M>(&mut self, projection: M)
    where
        M: Into<Matrix4<f32>>,
    {
        self.projection = projection.into();
    }

    pub fn apply_default_pipeline(&mut self) {
        self.mq.apply_pipeline(&self.pipeline);
    }

    pub fn apply_pipeline(&mut self, pipeline: &Pipeline) {
        self.mq.apply_pipeline(&pipeline.mq);
    }

    pub fn commit_frame(&mut self) {
        self.mq.commit_frame();
        self.expire_render_passes();
    }

    pub fn begin_default_pass(&mut self, action: PassAction) {
        self.mq.begin_default_pass(action.into());
    }

    pub fn begin_pass(&mut self, pass: &RenderPass, action: PassAction) {
        self.mq.begin_pass(**pass, mq::PassAction::from(action));
    }

    pub fn end_pass(&mut self) {
        self.mq.end_render_pass();
    }
}

pub struct Mesh {
    /// The shared reference to the texture, so that it doesn't get dropped and deleted.
    /// The inner data is already in `bindings` so this is really just to keep it from
    /// being dropped.
    pub texture: Texture,
    pub bindings: mq::Bindings,
    pub len: i32,
}

impl Mesh {
    pub fn draw(&self, ctx: &mut Graphics) {
        ctx.mq.apply_bindings(&self.bindings);
        ctx.mq.draw(0, self.len, 1);
    }
}

pub struct MeshBuilder {
    pub buffer: t::geometry_builder::VertexBuffers<Vertex, u16>,
    pub texture: Texture,
}

impl MeshBuilder {
    pub fn new<T>(texture: T) -> Self
    where
        T: Into<Texture>,
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

    // /// Create a new mesh for an ellipse.
    // ///
    // /// For the meaning of the `tolerance` parameter, [see here](https://docs.rs/lyon_geom/0.11.0/lyon_geom/#flattening).
    // pub fn ellipse<P>(
    //     &mut self,
    //     mode: DrawMode,
    //     point: P,
    //     radius1: f32,
    //     radius2: f32,
    //     tolerance: f32,
    //     color: Color,
    // ) -> &mut Self
    // where
    //     P: Into<mint::Point2<f32>>,
    // {
    //     {
    //         let buffers = &mut self.buffer;
    //         let point = point.into();
    //         let vb = VertexBuilder {
    //             color: LinearColor::from(color),
    //         };
    //         match mode {
    //             DrawMode::Fill(fill_options) => {
    //                 let builder = &mut t::BuffersBuilder::new(buffers, vb);
    //                 let _ = t::basic_shapes::fill_ellipse(
    //                     t::math::point(point.x, point.y),
    //                     t::math::vector(radius1, radius2),
    //                     t::math::Angle { radians: 0.0 },
    //                     &fill_options.with_tolerance(tolerance),
    //                     builder,
    //                 );
    //             }
    //             DrawMode::Stroke(options) => {
    //                 let builder = &mut t::BuffersBuilder::new(buffers, vb);
    //                 let _ = t::basic_shapes::stroke_ellipse(
    //                     t::math::point(point.x, point.y),
    //                     t::math::vector(radius1, radius2),
    //                     t::math::Angle { radians: 0.0 },
    //                     &options.with_tolerance(tolerance),
    //                     builder,
    //                 );
    //             }
    //         };
    //     }
    //     self
    // }

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
            let rect = t::math::rect(bounds.x(), bounds.y(), bounds.w(), bounds.h());
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

        Mesh {
            texture: self.texture.clone(),
            bindings: mq::Bindings {
                vertex_buffers: vec![vertex_buffer],
                index_buffer,
                images: vec![*self.texture],
            },
            len: self.buffer.indices.len() as i32,
        }
    }
}

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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn translate(self, v: Vector2<f32>) -> Self {
        Self {
            tx: self.tx * Translation3::new(v.x, v.y, 0.),
            ..self
        }
    }

    pub fn scale(self, v: Vector2<f32>) -> Self {
        Self {
            tx: self.tx
                * Transform3::from_matrix_unchecked(Matrix4::from_diagonal(&v.push(1.).push(1.))),
            ..self
        }
    }

    pub fn to_instance_properties(&self) -> InstanceProperties {
        let mins = self.src.mins;
        let maxs = self.src.mins + self.src.extent;
        InstanceProperties {
            src: Vector4::new(mins.x, mins.y, maxs.x, maxs.y),
            tx: *self.tx.matrix(),
            color: LinearColor::from(self.color),
        }
    }
}

fn quad_vertices() -> [Vertex; 4] {
    [
        Vertex {
            pos: Vector2::new(0., 0.),
            uv: Vector2::new(0., 0.),
            color: Color::WHITE.into(),
        },
        Vertex {
            pos: Vector2::new(1., 0.),
            uv: Vector2::new(1., 0.),
            color: Color::WHITE.into(),
        },
        Vertex {
            pos: Vector2::new(1., 1.),
            uv: Vector2::new(1., 1.),
            color: Color::WHITE.into(),
        },
        Vertex {
            pos: Vector2::new(0., 1.),
            uv: Vector2::new(0., 1.),
            color: Color::WHITE.into(),
        },
    ]
}

fn quad_indices() -> [u16; 6] {
    [0, 1, 2, 0, 2, 3]
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct SpriteIdx(Index);

pub struct SpriteBatch {
    sprites: Arena<InstanceParam>,
    instances: Vec<InstanceProperties>,
    bindings: mq::Bindings,
    capacity: usize,
    dirty: bool,
    /// Shared reference to keep the texture alive.
    _texture: Texture,
}

impl ops::Index<SpriteIdx> for SpriteBatch {
    type Output = InstanceParam;

    fn index(&self, index: SpriteIdx) -> &Self::Output {
        &self.sprites[index.0]
    }
}

impl ops::IndexMut<SpriteIdx> for SpriteBatch {
    fn index_mut(&mut self, index: SpriteIdx) -> &mut Self::Output {
        self.dirty = true;
        &mut self.sprites[index.0]
    }
}

impl SpriteBatch {
    pub fn with_capacity(ctx: &mut Graphics, texture: Texture, capacity: usize) -> Self {
        let instances = mq::Buffer::stream(
            &mut ctx.mq,
            mq::BufferType::VertexBuffer,
            capacity * mem::size_of::<InstanceProperties>(),
        );

        let bindings = mq::Bindings {
            vertex_buffers: vec![ctx.quad_bindings.vertex_buffers[0], instances],
            index_buffer: ctx.quad_bindings.index_buffer,
            images: vec![*texture],
        };

        Self {
            sprites: Arena::new(),
            instances: Vec::new(),
            bindings,
            capacity,
            dirty: true,
            _texture: texture,
        }
    }

    pub fn insert(&mut self, param: InstanceParam) -> SpriteIdx {
        self.dirty = true;
        SpriteIdx(self.sprites.insert(param))
    }

    pub fn remove(&mut self, index: SpriteIdx) {
        self.sprites.remove(index.0);
    }

    pub fn clear(&mut self) {
        self.sprites.clear();
    }

    pub fn flush(&mut self, ctx: &mut Graphics) {
        if !self.dirty {
            return;
        }

        self.instances.clear();
        self.instances.extend(
            self.sprites
                .iter()
                .map(|(_, param)| param.to_instance_properties()),
        );

        if self.instances.len() > self.capacity {
            self.capacity = self.capacity * 2;
            self.bindings.vertex_buffers[1] = mq::Buffer::stream(
                &mut ctx.mq,
                mq::BufferType::VertexBuffer,
                self.capacity * mem::size_of::<InstanceProperties>(),
            );
        }

        self.bindings.vertex_buffers[1].update(&mut ctx.mq, &self.instances);

        self.dirty = false;
    }

    pub fn draw(&mut self, ctx: &mut Graphics) {
        self.flush(ctx);
        ctx.mq.apply_bindings(&self.bindings);
        ctx.mq.draw(0, 6, self.instances.len() as i32);
    }
}

pub struct Canvas {
    pub render_pass: RenderPass,
    pub bindings: mq::Bindings,
    pub texture: Texture,
}

impl Canvas {
    pub fn new(ctx: &mut Graphics, width: u32, height: u32) -> Self {
        let color_img = Texture::from(mq::Texture::new_render_texture(
            &mut ctx.mq,
            mq::TextureParams {
                width,
                height,
                format: mq::TextureFormat::RGBA8,
                filter: mq::FilterMode::Nearest,
                ..Default::default()
            },
        ));
        let depth_img = Texture::from(mq::Texture::new_render_texture(
            &mut ctx.mq,
            mq::TextureParams {
                width,
                height,
                format: mq::TextureFormat::Depth,
                filter: mq::FilterMode::Nearest,
                ..Default::default()
            },
        ));

        let render_pass = RenderPass::new(ctx, color_img.clone(), depth_img);

        let quad_vertices =
            mq::Buffer::immutable(&mut ctx.mq, mq::BufferType::VertexBuffer, &quad_vertices());
        let quad_indices =
            mq::Buffer::immutable(&mut ctx.mq, mq::BufferType::IndexBuffer, &quad_indices());
        let instances = mq::Buffer::stream(
            &mut ctx.mq,
            mq::BufferType::VertexBuffer,
            mem::size_of::<InstanceProperties>(),
        );

        let bindings = mq::Bindings {
            vertex_buffers: vec![quad_vertices, instances],
            index_buffer: quad_indices,
            images: vec![*color_img],
        };

        Self {
            render_pass,
            bindings,
            texture: color_img,
        }
    }

    pub fn draw(&mut self, ctx: &mut Graphics, instance: InstanceParam) {
        self.bindings.vertex_buffers[1].update(&mut ctx.mq, &[instance.to_instance_properties()]);
        ctx.mq.apply_bindings(&self.bindings);
        ctx.mq.draw(0, 6, 1);
    }
}
