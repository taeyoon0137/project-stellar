//! backend 중립 pipeline key와 draw command (XI.5.1).
//!
//! 공통 계층(compiler·batching, XI.4)은 pass마다 [`DrawCommand`] 목록을 만들고, platform backend는
//! 이를 자기 command encoder 호출로 옮깁니다. command에는 native 객체가 없고 handle·graph version·
//! 값만 있습니다.

use stellar_scene::stack::SceneBlendMode;

use crate::backend::{BufferHandle, TextureHandle};
use crate::placeholder::SamplerDesc;
use crate::render_graph::{PassId, ResourceVersion, SampleCount, TextureFormat};

/// 공통 shader program. backend는 program마다 자기 shader 변형을 둡니다(XI.8.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShaderProgram {
    /// 단색 채우기(vertex color 또는 uniform color).
    SolidFill,
    /// texture sampling(이미지·placeholder).
    Texture,
    /// glyph atlas coverage mask × 색.
    GlyphMask,
    /// offscreen layer 합성(opacity 적용, Normal blend).
    LayerComposite,
    /// backdrop을 읽어 blend mode를 계산하는 합성(XI.10.2).
    BlendComposite(SceneBlendMode),
    /// clip 도형을 stencil에 기록(color 쓰기 없음).
    ClipStencil,
    /// clip 도형 coverage를 mask texture에 기록.
    ClipMask,
    /// separable gaussian blur(shadow·layer blur).
    GaussianBlur,
}

/// color blend 상태(premultiplied alpha 기준).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlendState {
    /// 대상 값을 그대로 덮어씁니다(불투명 그리기).
    Replace,
    /// premultiplied source-over: `src + dst × (1 − src.a)`.
    PremultipliedOver,
}

/// depth test·write 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DepthMode {
    /// depth 사용 안 함.
    #[default]
    Disabled,
    /// 앞에 있는 것만 통과하고 depth를 기록(불투명 pass).
    TestAndWrite,
    /// 앞에 있는 것만 통과, 기록 안 함(반투명 pass).
    TestOnly,
}

/// stencil 상태(clip, XI.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum StencilMode {
    /// stencil 사용 안 함.
    #[default]
    Disabled,
    /// reference와 같은 stencil 값에서만 그리고 stencil을 1 증가(clip 진입 기록).
    IncrementWhereEqual,
    /// reference + 1과 같은 stencil 값을 1 감소(clip 해제 기록).
    DecrementWhereEqual,
    /// stencil 값이 reference와 같은 곳에만 그림.
    TestEqual,
}

/// pipeline state key. 같은 key는 backend에서 같은 pipeline 객체로 대응합니다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineKey {
    /// shader program.
    pub program: ShaderProgram,
    /// blend 상태.
    pub blend: BlendState,
    /// depth 상태.
    pub depth: DepthMode,
    /// stencil 상태.
    pub stencil: StencilMode,
    /// color attachment format.
    pub color_format: TextureFormat,
    /// multisample 수.
    pub samples: SampleCount,
}

/// device pixel 좌표 scissor 사각형(좌상단 원점).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScissorRect {
    /// 왼쪽.
    pub x: u32,
    /// 위.
    pub y: u32,
    /// 너비.
    pub width: u32,
    /// 높이.
    pub height: u32,
}

/// 그리기에 bind할 texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureSource {
    /// backend texture handle(이미지·atlas·placeholder).
    Handle(TextureHandle),
    /// 같은 frame graph의 resource version(offscreen layer·mask).
    Graph(ResourceVersion),
}

/// backend 중립 draw command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DrawCommand {
    /// pipeline 상태를 바꿉니다.
    SetPipeline(PipelineKey),
    /// scissor를 바꿉니다(`None`이면 attachment 전체).
    SetScissor(Option<ScissorRect>),
    /// stencil reference 값을 바꿉니다.
    SetStencilReference(u8),
    /// vertex buffer를 bind합니다.
    SetVertexBuffer {
        /// buffer.
        buffer: BufferHandle,
        /// byte offset.
        offset: u64,
    },
    /// index buffer(u32 index)를 bind합니다.
    SetIndexBuffer {
        /// buffer.
        buffer: BufferHandle,
        /// byte offset.
        offset: u64,
    },
    /// uniform 범위를 bind합니다.
    SetUniforms {
        /// buffer.
        buffer: BufferHandle,
        /// byte offset.
        offset: u64,
        /// byte 길이.
        size: u32,
    },
    /// texture와 sampler를 slot에 bind합니다.
    BindTexture {
        /// slot.
        slot: u8,
        /// texture.
        texture: TextureSource,
        /// sampler 상태.
        sampler: SamplerDesc,
    },
    /// index 없이 그립니다.
    Draw {
        /// 첫 vertex.
        vertex_start: u32,
        /// vertex 수.
        vertex_count: u32,
        /// instance 수.
        instance_count: u32,
    },
    /// index로 그립니다.
    DrawIndexed {
        /// 첫 index.
        index_start: u32,
        /// index 수.
        index_count: u32,
        /// vertex index에 더할 값.
        base_vertex: i32,
        /// instance 수.
        instance_count: u32,
    },
}

/// pass 하나의 command 목록.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassCommands {
    /// 대상 pass.
    pub pass: PassId,
    /// command(실행 순서).
    pub commands: Vec<DrawCommand>,
}

impl PassCommands {
    /// 빈 목록.
    pub fn new(pass: PassId) -> PassCommands {
        PassCommands {
            pass,
            commands: Vec::new(),
        }
    }
}
