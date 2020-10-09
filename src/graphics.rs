use crate::math::*;
use {
    anyhow::*,
    miniquad as mq,
    std::{mem, ops},
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
        pub color: Vector4<f32>,
    }

    #[derive(Debug, Clone, Copy)]
    #[repr(C)]
    pub struct InstanceProperties {
        pub src: Vector4<f32>,
        pub tx: Matrix4<f32>,
        pub color: Vector4<f32>,
    }
}

pub struct Context {
    pub mq: mq::Context,
    pub pipeline: mq::Pipeline,
    pub null_texture: mq::Texture,
}

impl Context {
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

        let null_texture = mq::Texture::from_rgba8(&mut mq, 1, 1, &[255, 255, 255, 255]);

        Ok(Self {
            mq,
            pipeline,
            null_texture,
        })
    }
}

pub struct Mesh {
    bindings: mq::Bindings,
}

pub struct MeshBuilder {
    vertices: Vec<shader::Vertex>,
    indices: Vec<u16>,
    texture: mq::Texture,
}

impl MeshBuilder {
    pub fn new(texture: mq::Texture) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            texture,
        }
    }

    pub fn build(&self, ctx: &mut Context) -> Mesh {
        let vertex_buffer =
            mq::Buffer::immutable(&mut ctx.mq, mq::BufferType::VertexBuffer, &self.vertices);
        let index_buffer =
            mq::Buffer::immutable(&mut ctx.mq, mq::BufferType::IndexBuffer, &self.indices);

        Mesh {
            bindings: mq::Bindings {
                vertex_buffers: vec![vertex_buffer],
                index_buffer,
                images: vec![self.texture],
            },
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct InstanceParam {
    pub src: Box2<f32>,
    pub tx: Transform3<f32>,
    pub color: Vector4<f32>,
}

impl Default for InstanceParam {
    fn default() -> Self {
        Self {
            src: Box2::new(0., 0., 1., 1.),
            tx: Transform3::identity(),
            color: Vector4::repeat(1.),
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

    pub fn to_instance_properties(&self) -> shader::InstanceProperties {
        let mins = self.src.mins;
        let maxs = self.src.mins + self.src.extent;
        shader::InstanceProperties {
            src: Vector4::new(mins.x, mins.y, maxs.x, maxs.y),
            tx: *self.tx.matrix(),
            color: self.color,
        }
    }
}

fn quad_vertices() -> [shader::Vertex; 4] {
    [
        shader::Vertex {
            pos: Vector2::new(0., 0.),
            uv: Vector2::new(0., 0.),
            color: Vector4::repeat(1.),
        },
        shader::Vertex {
            pos: Vector2::new(1., 0.),
            uv: Vector2::new(1., 0.),
            color: Vector4::repeat(1.),
        },
        shader::Vertex {
            pos: Vector2::new(1., 1.),
            uv: Vector2::new(1., 1.),
            color: Vector4::repeat(1.),
        },
        shader::Vertex {
            pos: Vector2::new(0., 1.),
            uv: Vector2::new(0., 1.),
            color: Vector4::repeat(1.),
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
    instances: Vec<shader::InstanceProperties>,
    bindings: mq::Bindings,
    capacity: usize,
    dirty: bool,
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
    pub fn with_capacity(ctx: &mut mq::Context, texture: mq::Texture, capacity: usize) -> Self {
        let quad_vertices =
            mq::Buffer::immutable(ctx, mq::BufferType::VertexBuffer, &quad_vertices());
        let quad_indices = mq::Buffer::immutable(ctx, mq::BufferType::IndexBuffer, &quad_indices());

        let instances = mq::Buffer::stream(
            ctx,
            mq::BufferType::VertexBuffer,
            capacity * mem::size_of::<shader::InstanceProperties>(),
        );

        Self {
            sprites: Arena::new(),
            instances: Vec::new(),
            bindings: mq::Bindings {
                vertex_buffers: vec![quad_vertices, instances],
                index_buffer: quad_indices,
                images: vec![texture],
            },
            capacity,
            dirty: true,
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

    pub fn flush(&mut self, ctx: &mut mq::Context) {
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
                ctx,
                mq::BufferType::VertexBuffer,
                self.capacity * mem::size_of::<shader::InstanceProperties>(),
            );
        }

        self.bindings.vertex_buffers[1].update(ctx, &self.instances);

        self.dirty = false;
    }

    pub fn draw(&mut self, ctx: &mut mq::Context) {
        self.flush(ctx);
        ctx.apply_bindings(&self.bindings);
        ctx.draw(0, 6, self.instances.len() as i32);
    }
}

pub struct Canvas {
    pub render_pass: mq::RenderPass,
    pub bindings: mq::Bindings,
}

impl Canvas {
    pub fn new(ctx: &mut mq::Context, width: u32, height: u32) -> Self {
        let color_img = mq::Texture::new_render_texture(
            ctx,
            mq::TextureParams {
                width,
                height,
                format: mq::TextureFormat::RGBA8,
                filter: mq::FilterMode::Nearest,
                ..Default::default()
            },
        );
        let depth_img = mq::Texture::new_render_texture(
            ctx,
            mq::TextureParams {
                width,
                height,
                format: mq::TextureFormat::Depth,
                filter: mq::FilterMode::Nearest,
                ..Default::default()
            },
        );

        let render_pass = mq::RenderPass::new(ctx, color_img, depth_img);

        let quad_vertices =
            mq::Buffer::immutable(ctx, mq::BufferType::VertexBuffer, &quad_vertices());
        let quad_indices = mq::Buffer::immutable(ctx, mq::BufferType::IndexBuffer, &quad_indices());
        let instances = mq::Buffer::stream(
            ctx,
            mq::BufferType::VertexBuffer,
            mem::size_of::<shader::InstanceProperties>(),
        );

        let bindings = mq::Bindings {
            vertex_buffers: vec![quad_vertices, instances],
            index_buffer: quad_indices,
            images: vec![color_img],
        };

        Self {
            render_pass,
            bindings,
        }
    }

    pub fn draw(&mut self, ctx: &mut mq::Context, instance: InstanceParam) {
        self.bindings.vertex_buffers[1].update(ctx, &[instance.to_instance_properties()]);
        ctx.apply_bindings(&self.bindings);
        ctx.draw(0, 6, 1);
    }
}
