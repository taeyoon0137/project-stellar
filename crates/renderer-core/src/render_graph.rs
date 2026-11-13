//! backend 독립 Render Graph (XI.4.1).
//!
//! 한 frame의 GPU 작업을 **pass**와 pass가 읽고 쓰는 **resource**(texture)의 graph로 기술합니다.
//! Metal·D3D12 backend는 [`CompiledRenderGraph`]만 받아 encode하며, graph는 platform GPU 객체를
//! 참조하지 않습니다(아키텍처 §10.1).
//!
//! ## resource version
//!
//! resource 내용은 pass가 쓸 때마다 새 **version**이 됩니다([`ResourceVersion`]). version 0은 graph
//! 시작 시점 내용으로, imported resource는 외부에서 온 내용이고 transient resource는 정의되지
//! 않은 내용입니다. pass는 version `v`(1 이상)를 **생산**하고, 다른 pass는 특정 version을 **읽습니다**.
//! 그래서 pass 간 의존은 선언 순서가 아니라 resource 흐름에서 정해집니다.
//!
//! 같은 resource의 version은 같은 메모리를 덮어쓰므로 다음 순서 제약이 생깁니다.
//!
//! - **RAW**: version `v`를 읽는 pass는 `v`를 생산한 pass 뒤에 실행합니다.
//! - **WAW**: version `v`의 생산 pass는 `v - 1` 생산 pass 뒤에 실행합니다.
//! - **WAR**: version `v`의 생산 pass는 `v - 1`을 읽는 모든 pass 뒤에 실행합니다.
//!
//! [`LoadOp::Load`]로 attachment를 쓰면 이전 version(`v - 1`) 내용을 이어 그리므로 그 version을
//! 읽는 것으로 봅니다.
//!
//! ## 검증과 compile
//!
//! [`RenderGraph`]는 검증 전 기술(field 공개)입니다. [`RenderGraph::compile`]이 구조를 검사하고
//! 위 순서 제약으로 실행 순서를 정해 [`CompiledRenderGraph`]를 만듭니다. 거부 조건은
//! [`RenderGraphError`]에 있으며, 대표적으로 **cycle**(순서를 정할 수 없음)과 **missing resource**
//! (없는 resource id, 생산자 없는 version, 정의되지 않은 transient 읽기)입니다. 실행 순서는
//! 결정론적입니다: 순서 제약을 만족하는 pass 중 선언 index가 가장 작은 pass를 먼저 둡니다.
//!
//! ## resource lifetime
//!
//! - [`ResourceLifetime::Imported`]: graph 밖이 소유합니다(present surface, texture cache 등).
//!   graph는 할당·해제하지 않습니다.
//! - [`ResourceLifetime::Transient`]: graph 안에서만 삽니다. compile 결과의 [`LiveRange`]가 실행
//!   순서상 처음·마지막 사용 단계를 알려 주고, 수명이 겹치지 않고 descriptor가 같은 transient는 같은
//!   alias slot을 받아 backend가 메모리를 재사용할 수 있습니다.

use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap};
use std::fmt;

use stellar_scene::FiniteF32;

/// pass 하나의 최대 color attachment 수.
pub const MAX_COLOR_ATTACHMENTS: usize = 8;

/// 고정 크기 texture 한 변의 최대 pixel 수(decode 상한과 같은 16384).
pub const MAX_TEXTURE_DIMENSION: u32 = 16_384;

/// graph resource 식별자(graph의 `resources` index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId(pub u32);

/// pass 식별자(graph의 `passes` index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PassId(pub u32);

impl ResourceId {
    fn index(self) -> usize {
        self.0 as usize
    }
}

impl PassId {
    fn index(self) -> usize {
        self.0 as usize
    }
}

/// resource의 특정 내용 version(모듈 문서).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceVersion {
    /// resource.
    pub resource: ResourceId,
    /// version(0 = graph 시작 시점 내용).
    pub version: u32,
}

impl ResourceVersion {
    /// version을 만듭니다.
    pub const fn new(resource: ResourceId, version: u32) -> ResourceVersion {
        ResourceVersion { resource, version }
    }
}

/// texture format(backend 중립 이름).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    /// 8-bit RGBA unorm(선형 값 그대로 저장).
    Rgba8Unorm,
    /// 8-bit RGBA, sRGB 전달 함수로 encode.
    Rgba8UnormSrgb,
    /// 8-bit BGRA unorm(일반적인 drawable format).
    Bgra8Unorm,
    /// 8-bit BGRA, sRGB 전달 함수로 encode.
    Bgra8UnormSrgb,
    /// 16-bit float RGBA(선형 working layer).
    Rgba16Float,
    /// 8-bit 단일 채널 unorm(coverage mask).
    R8Unorm,
    /// 32-bit float depth + 8-bit stencil.
    Depth32FloatStencil8,
}

impl TextureFormat {
    /// depth·stencil format이면 참입니다.
    pub const fn is_depth_stencil(self) -> bool {
        matches!(self, TextureFormat::Depth32FloatStencil8)
    }
}

/// attachment 크기.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttachmentSize {
    /// frame 출력 surface와 같은 크기(resize를 따라갑니다).
    Surface,
    /// 고정 pixel 크기(양수, 한 변 [`MAX_TEXTURE_DIMENSION`] 이하).
    Fixed {
        /// 너비.
        width: u32,
        /// 높이.
        height: u32,
    },
}

/// multisample 수.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SampleCount {
    /// multisample 없음.
    #[default]
    X1,
    /// 4x MSAA.
    X4,
}

/// graph texture descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureDesc {
    /// format.
    pub format: TextureFormat,
    /// 크기.
    pub size: AttachmentSize,
    /// multisample 수.
    pub samples: SampleCount,
}

impl TextureDesc {
    /// surface 크기, multisample 없는 descriptor.
    pub const fn surface(format: TextureFormat) -> TextureDesc {
        TextureDesc {
            format,
            size: AttachmentSize::Surface,
            samples: SampleCount::X1,
        }
    }
}

/// imported resource 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImportKind {
    /// present 대상 surface(drawable·back buffer).
    Surface,
    /// graph 밖이 소유하는 texture(cache·offscreen export target 등).
    External,
}

/// resource 수명 분류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceLifetime {
    /// graph 안에서만 사는 resource. backend가 frame 안에서 할당·재사용합니다.
    Transient,
    /// graph 밖이 소유하는 resource.
    Imported(ImportKind),
}

/// graph resource 선언.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphResource {
    /// 진단용 정적 이름(사용자 내용 금지).
    pub label: &'static str,
    /// descriptor.
    pub desc: TextureDesc,
    /// 수명 분류.
    pub lifetime: ResourceLifetime,
}

/// attachment load 동작.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoadOp {
    /// 이전 version 내용을 이어 그립니다(이전 version을 읽음).
    Load,
    /// clear 값으로 채웁니다.
    Clear,
    /// 이전 내용을 쓰지 않습니다(전부 덮어씀).
    DontCare,
}

/// attachment store 동작.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StoreOp {
    /// 결과를 보존합니다.
    Store,
    /// pass가 끝나면 버립니다(이 version은 아무도 읽을 수 없음).
    Discard,
}

/// color clear 값(선형, premultiplied, finite).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClearColor {
    /// red.
    pub r: FiniteF32,
    /// green.
    pub g: FiniteF32,
    /// blue.
    pub b: FiniteF32,
    /// alpha.
    pub a: FiniteF32,
}

impl ClearColor {
    /// 투명 검정.
    pub const TRANSPARENT: ClearColor = ClearColor {
        r: FiniteF32::ZERO,
        g: FiniteF32::ZERO,
        b: FiniteF32::ZERO,
        a: FiniteF32::ZERO,
    };
}

/// color attachment 쓰기.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorAttachment {
    /// 이 pass가 생산하는 version(1 이상).
    pub target: ResourceVersion,
    /// load 동작.
    pub load: LoadOp,
    /// `LoadOp::Clear`일 때 clear 값.
    pub clear: ClearColor,
    /// store 동작.
    pub store: StoreOp,
}

/// depth·stencil attachment 쓰기.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepthStencilAttachment {
    /// 이 pass가 생산하는 version(1 이상).
    pub target: ResourceVersion,
    /// load 동작(depth·stencil 공통).
    pub load: LoadOp,
    /// clear depth(`[0, 1]`).
    pub clear_depth: FiniteF32,
    /// clear stencil.
    pub clear_stencil: u8,
    /// store 동작.
    pub store: StoreOp,
}

/// pass 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PassKind {
    /// rasterization pass(attachment에 그림).
    Raster,
    /// compute pass(blur 등, `writes`로 storage texture를 씀).
    Compute,
    /// copy·upload pass(`writes`로 복사 대상을 씀).
    Transfer,
}

/// graph pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPass {
    /// 진단용 정적 이름(사용자 내용 금지).
    pub label: &'static str,
    /// 종류.
    pub kind: PassKind,
    /// color attachment(Raster 전용).
    pub color: Vec<ColorAttachment>,
    /// depth·stencil attachment(Raster 전용).
    pub depth_stencil: Option<DepthStencilAttachment>,
    /// sampling·copy source로 읽는 version.
    pub reads: Vec<ResourceVersion>,
    /// attachment가 아닌 쓰기(Compute storage·Transfer 대상)로 생산하는 version. 이전 내용을
    /// 읽지 않습니다.
    pub writes: Vec<ResourceVersion>,
}

impl RenderPass {
    /// attachment 없는 빈 pass.
    pub fn new(label: &'static str, kind: PassKind) -> RenderPass {
        RenderPass {
            label,
            kind,
            color: Vec::new(),
            depth_stencil: None,
            reads: Vec::new(),
            writes: Vec::new(),
        }
    }

    /// 이 pass가 생산하는 version(color → depth·stencil → writes 순).
    pub fn produced(&self) -> impl Iterator<Item = ResourceVersion> + '_ {
        self.color
            .iter()
            .map(|attachment| attachment.target)
            .chain(
                self.depth_stencil
                    .iter()
                    .map(|attachment| attachment.target),
            )
            .chain(self.writes.iter().copied())
    }

    /// `Load` attachment가 이어 그리는 이전 version.
    fn loaded(&self) -> impl Iterator<Item = ResourceVersion> + '_ {
        let color = self
            .color
            .iter()
            .filter(|attachment| attachment.load == LoadOp::Load)
            .map(|attachment| attachment.target);
        let depth = self
            .depth_stencil
            .iter()
            .filter(|attachment| attachment.load == LoadOp::Load)
            .map(|attachment| attachment.target);
        color
            .chain(depth)
            .filter(|target| target.version > 0)
            .map(|target| ResourceVersion::new(target.resource, target.version - 1))
    }

    /// 이 pass가 읽는 version(`reads` + `Load` attachment의 이전 version).
    pub fn consumed(&self) -> impl Iterator<Item = ResourceVersion> + '_ {
        self.reads.iter().copied().chain(self.loaded())
    }
}

/// 검증 전 graph 기술.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderGraph {
    /// resource 선언(`ResourceId` = index).
    pub resources: Vec<GraphResource>,
    /// pass(`PassId` = index, 선언 순서).
    pub passes: Vec<RenderPass>,
    /// frame 최종 결과 version(present 또는 export 대상). imported resource의 최신 version이어야
    /// 합니다.
    pub output: Option<ResourceVersion>,
}

/// graph 검증 오류.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderGraphError {
    /// resource·pass 수가 `u32` 범위를 넘었습니다.
    TooLarge,
    /// pass가 선언되지 않은 resource를 참조합니다(missing resource).
    UnknownResource {
        /// 참조한 pass(graph output이면 `None`).
        pass: Option<PassId>,
        /// 없는 resource.
        resource: ResourceId,
    },
    /// 생산자 없는 version을 읽습니다(missing resource).
    MissingProducer {
        /// 읽는 pass(graph output이면 `None`).
        pass: Option<PassId>,
        /// 생산자가 없는 version.
        version: ResourceVersion,
    },
    /// 정의되지 않은 transient 내용(version 0)을 읽습니다(missing resource).
    UndefinedTransientRead {
        /// 읽는 pass.
        pass: PassId,
        /// transient resource.
        resource: ResourceId,
    },
    /// version 0을 쓰려 합니다(쓰기는 1 이상 version을 생산).
    WritesInitialVersion {
        /// 쓰는 pass.
        pass: PassId,
        /// resource.
        resource: ResourceId,
    },
    /// 같은 version을 두 pass가 생산합니다.
    DuplicateProducer {
        /// version.
        version: ResourceVersion,
        /// 먼저 선언된 생산 pass.
        first: PassId,
        /// 나중 생산 pass.
        second: PassId,
    },
    /// version `v`(2 이상)의 생산자는 있지만 `v - 1` 생산자가 없습니다.
    VersionGap {
        /// 앞 version이 없는 version.
        version: ResourceVersion,
    },
    /// pass가 같은 resource를 읽으면서 씁니다(feedback loop) 또는 한 pass에서 두 번 씁니다.
    FeedbackLoop {
        /// pass.
        pass: PassId,
        /// resource.
        resource: ResourceId,
    },
    /// `StoreOp::Discard`로 버린 version을 읽습니다.
    DiscardedRead {
        /// 버린 version.
        version: ResourceVersion,
        /// 읽는 pass(graph output이면 `None`).
        reader: Option<PassId>,
    },
    /// attachment 종류와 resource format이 맞지 않습니다(color에 depth format 등).
    AttachmentFormatMismatch {
        /// pass.
        pass: PassId,
        /// resource.
        resource: ResourceId,
    },
    /// 한 pass의 attachment 크기·sample 수가 서로 다릅니다.
    AttachmentSizeMismatch {
        /// pass.
        pass: PassId,
    },
    /// Raster pass에 attachment가 없거나, Raster가 아닌 pass에 attachment가 있거나, color
    /// attachment가 상한을 넘습니다.
    InvalidAttachments {
        /// pass.
        pass: PassId,
    },
    /// 고정 크기가 0이거나 상한을 넘습니다.
    InvalidSize {
        /// resource.
        resource: ResourceId,
    },
    /// graph output resource가 imported가 아닙니다(transient는 frame 뒤에 사라짐).
    OutputNotImported {
        /// output resource.
        resource: ResourceId,
    },
    /// graph output이 resource의 최신 version이 아닙니다(뒤에 덮어쓰는 pass가 있음).
    OutputNotLatest {
        /// 지정한 output.
        output: ResourceVersion,
        /// 실제 최신 version.
        latest: u32,
    },
    /// pass 순서를 정할 수 없습니다. `passes`는 cycle을 이루는 pass(의존 방향 순서)입니다.
    Cycle {
        /// cycle 경로.
        passes: Vec<PassId>,
    },
}

impl fmt::Display for RenderGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderGraphError::TooLarge => f.write_str("render graph 크기가 상한을 넘었습니다"),
            RenderGraphError::UnknownResource { pass, resource } => {
                write!(
                    f,
                    "선언되지 않은 resource {} 참조(pass {pass:?})",
                    resource.0
                )
            }
            RenderGraphError::MissingProducer { pass, version } => write!(
                f,
                "resource {} version {}의 생산 pass가 없습니다(pass {pass:?})",
                version.resource.0, version.version
            ),
            RenderGraphError::UndefinedTransientRead { pass, resource } => write!(
                f,
                "pass {}가 쓰기 전 transient resource {}를 읽습니다",
                pass.0, resource.0
            ),
            RenderGraphError::WritesInitialVersion { pass, resource } => write!(
                f,
                "pass {}가 resource {}의 version 0을 쓰려 합니다",
                pass.0, resource.0
            ),
            RenderGraphError::DuplicateProducer {
                version,
                first,
                second,
            } => write!(
                f,
                "resource {} version {}을 pass {}와 {}가 함께 생산합니다",
                version.resource.0, version.version, first.0, second.0
            ),
            RenderGraphError::VersionGap { version } => write!(
                f,
                "resource {} version {}의 이전 version 생산자가 없습니다",
                version.resource.0, version.version
            ),
            RenderGraphError::FeedbackLoop { pass, resource } => write!(
                f,
                "pass {}가 resource {}를 읽으면서 씁니다",
                pass.0, resource.0
            ),
            RenderGraphError::DiscardedRead { version, reader } => write!(
                f,
                "버린 resource {} version {}을 읽습니다(pass {reader:?})",
                version.resource.0, version.version
            ),
            RenderGraphError::AttachmentFormatMismatch { pass, resource } => write!(
                f,
                "pass {}의 attachment 종류와 resource {} format이 맞지 않습니다",
                pass.0, resource.0
            ),
            RenderGraphError::AttachmentSizeMismatch { pass } => {
                write!(f, "pass {}의 attachment 크기·sample 수가 다릅니다", pass.0)
            }
            RenderGraphError::InvalidAttachments { pass } => {
                write!(
                    f,
                    "pass {}의 attachment 구성이 pass 종류와 맞지 않습니다",
                    pass.0
                )
            }
            RenderGraphError::InvalidSize { resource } => {
                write!(
                    f,
                    "resource {}의 고정 크기가 범위를 벗어났습니다",
                    resource.0
                )
            }
            RenderGraphError::OutputNotImported { resource } => {
                write!(
                    f,
                    "graph output resource {}가 imported가 아닙니다",
                    resource.0
                )
            }
            RenderGraphError::OutputNotLatest { output, latest } => write!(
                f,
                "graph output version {}이 최신({latest})이 아닙니다",
                output.version
            ),
            RenderGraphError::Cycle { passes } => {
                let ids: Vec<u32> = passes.iter().map(|pass| pass.0).collect();
                write!(f, "pass 의존 cycle: {ids:?}")
            }
        }
    }
}

impl std::error::Error for RenderGraphError {}

/// transient resource의 실행 순서상 사용 구간(양 끝 포함, [`CompiledRenderGraph::order`] index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LiveRange {
    /// 처음 사용 단계.
    pub first: usize,
    /// 마지막 사용 단계.
    pub last: usize,
}

impl LiveRange {
    fn overlaps(&self, other: &LiveRange) -> bool {
        self.first <= other.last && other.first <= self.last
    }
}

/// compile 결과 resource 수명.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceLiveness {
    /// graph 밖이 소유(imported).
    External,
    /// 어떤 pass도 쓰지 않는 transient(할당하지 않음).
    Unused,
    /// transient 사용 구간과 alias slot. 같은 slot의 resource는 descriptor가 같고 구간이 겹치지
    /// 않으므로 같은 메모리를 쓸 수 있습니다.
    Transient {
        /// 사용 구간.
        range: LiveRange,
        /// alias slot.
        slot: u32,
    },
}

/// 검증된 graph와 실행 순서.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledRenderGraph {
    graph: RenderGraph,
    order: Vec<PassId>,
    liveness: Vec<ResourceLiveness>,
    alias_slots: u32,
}

impl CompiledRenderGraph {
    /// 원본 graph.
    pub fn graph(&self) -> &RenderGraph {
        &self.graph
    }

    /// pass 실행 순서.
    pub fn order(&self) -> &[PassId] {
        &self.order
    }

    /// pass.
    pub fn pass(&self, id: PassId) -> Option<&RenderPass> {
        self.graph.passes.get(id.index())
    }

    /// resource.
    pub fn resource(&self, id: ResourceId) -> Option<&GraphResource> {
        self.graph.resources.get(id.index())
    }

    /// resource 수명(`ResourceId` index).
    pub fn liveness(&self) -> &[ResourceLiveness] {
        &self.liveness
    }

    /// transient alias slot 수(backend가 실제 할당할 transient texture 수의 상한).
    pub const fn alias_slot_count(&self) -> u32 {
        self.alias_slots
    }

    /// graph output.
    pub fn output(&self) -> Option<ResourceVersion> {
        self.graph.output
    }
}

/// 검증 중 version 생산 정보.
#[derive(Debug, Clone, Copy)]
struct Producer {
    pass: PassId,
    store: StoreOp,
}

impl RenderGraph {
    /// 빈 graph.
    pub fn new() -> RenderGraph {
        RenderGraph::default()
    }

    /// 검증하고 실행 순서·수명을 계산합니다.
    pub fn compile(self) -> Result<CompiledRenderGraph, RenderGraphError> {
        if u32::try_from(self.resources.len()).is_err() || u32::try_from(self.passes.len()).is_err()
        {
            return Err(RenderGraphError::TooLarge);
        }
        self.check_resources()?;
        let producers = self.collect_producers()?;
        self.check_consumers(&producers)?;
        self.check_output(&producers)?;
        let order = self.schedule(&producers)?;
        let (liveness, alias_slots) = self.liveness(&order);
        Ok(CompiledRenderGraph {
            graph: self,
            order,
            liveness,
            alias_slots,
        })
    }

    fn resource_of(
        &self,
        pass: Option<PassId>,
        id: ResourceId,
    ) -> Result<&GraphResource, RenderGraphError> {
        self.resources
            .get(id.index())
            .ok_or(RenderGraphError::UnknownResource { pass, resource: id })
    }

    fn check_resources(&self) -> Result<(), RenderGraphError> {
        for (index, resource) in self.resources.iter().enumerate() {
            if let AttachmentSize::Fixed { width, height } = resource.desc.size {
                let valid = |v: u32| v > 0 && v <= MAX_TEXTURE_DIMENSION;
                if !valid(width) || !valid(height) {
                    return Err(RenderGraphError::InvalidSize {
                        resource: ResourceId(index as u32),
                    });
                }
            }
        }
        Ok(())
    }

    /// pass별 쓰기 규칙을 검사하고 version → 생산자 표를 만듭니다.
    fn collect_producers(&self) -> Result<Vec<Vec<Option<Producer>>>, RenderGraphError> {
        // resource별 version index → 생산자. version 0은 항상 None(graph 시작 내용).
        let mut producers: Vec<Vec<Option<Producer>>> = vec![vec![None]; self.resources.len()];
        for (index, pass) in self.passes.iter().enumerate() {
            let id = PassId(index as u32);
            self.check_attachments(id, pass)?;
            let mut written: BTreeSet<ResourceId> = BTreeSet::new();
            let stores = pass
                .color
                .iter()
                .map(|attachment| attachment.store)
                .chain(pass.depth_stencil.iter().map(|attachment| attachment.store))
                .chain(pass.writes.iter().map(|_| StoreOp::Store));
            for (target, store) in pass.produced().zip(stores) {
                self.resource_of(Some(id), target.resource)?;
                if target.version == 0 {
                    return Err(RenderGraphError::WritesInitialVersion {
                        pass: id,
                        resource: target.resource,
                    });
                }
                if !written.insert(target.resource) {
                    return Err(RenderGraphError::FeedbackLoop {
                        pass: id,
                        resource: target.resource,
                    });
                }
                let slots = &mut producers[target.resource.index()];
                let slot = target.version as usize;
                if slots.len() <= slot {
                    slots.resize(slot + 1, None);
                }
                if let Some(first) = slots[slot] {
                    return Err(RenderGraphError::DuplicateProducer {
                        version: target,
                        first: first.pass,
                        second: id,
                    });
                }
                slots[slot] = Some(Producer { pass: id, store });
            }
            // sampling으로 읽는 resource를 같은 pass가 쓰면 feedback loop입니다.
            for read in &pass.reads {
                if written.contains(&read.resource) {
                    return Err(RenderGraphError::FeedbackLoop {
                        pass: id,
                        resource: read.resource,
                    });
                }
            }
        }
        for (index, slots) in producers.iter().enumerate() {
            for version in 2..slots.len() {
                if slots[version].is_some() && slots[version - 1].is_none() {
                    return Err(RenderGraphError::VersionGap {
                        version: ResourceVersion::new(ResourceId(index as u32), version as u32),
                    });
                }
            }
        }
        Ok(producers)
    }

    fn check_attachments(&self, id: PassId, pass: &RenderPass) -> Result<(), RenderGraphError> {
        let has_attachments = !pass.color.is_empty() || pass.depth_stencil.is_some();
        let valid_shape = match pass.kind {
            PassKind::Raster => has_attachments && pass.color.len() <= MAX_COLOR_ATTACHMENTS,
            PassKind::Compute | PassKind::Transfer => !has_attachments,
        };
        if !valid_shape {
            return Err(RenderGraphError::InvalidAttachments { pass: id });
        }
        let mut extent: Option<(AttachmentSize, SampleCount)> = None;
        let color = pass.color.iter().map(|a| (a.target.resource, false));
        let depth = pass.depth_stencil.iter().map(|a| (a.target.resource, true));
        for (resource, wants_depth) in color.chain(depth) {
            let desc = self.resource_of(Some(id), resource)?.desc;
            if desc.format.is_depth_stencil() != wants_depth {
                return Err(RenderGraphError::AttachmentFormatMismatch { pass: id, resource });
            }
            match extent {
                None => extent = Some((desc.size, desc.samples)),
                Some(first) if first != (desc.size, desc.samples) => {
                    return Err(RenderGraphError::AttachmentSizeMismatch { pass: id });
                }
                Some(_) => {}
            }
        }
        Ok(())
    }

    /// version 읽기가 가리키는 내용이 존재하는지 검사합니다.
    fn check_read(
        &self,
        producers: &[Vec<Option<Producer>>],
        reader: Option<PassId>,
        version: ResourceVersion,
    ) -> Result<(), RenderGraphError> {
        let resource = self.resource_of(reader, version.resource)?;
        if version.version == 0 {
            return match (resource.lifetime, reader) {
                (ResourceLifetime::Imported(_), _) => Ok(()),
                (ResourceLifetime::Transient, Some(pass)) => {
                    Err(RenderGraphError::UndefinedTransientRead {
                        pass,
                        resource: version.resource,
                    })
                }
                (ResourceLifetime::Transient, None) => Err(RenderGraphError::MissingProducer {
                    pass: None,
                    version,
                }),
            };
        }
        let producer = producers[version.resource.index()]
            .get(version.version as usize)
            .copied()
            .flatten()
            .ok_or(RenderGraphError::MissingProducer {
                pass: reader,
                version,
            })?;
        if producer.store == StoreOp::Discard {
            return Err(RenderGraphError::DiscardedRead { version, reader });
        }
        Ok(())
    }

    fn check_consumers(&self, producers: &[Vec<Option<Producer>>]) -> Result<(), RenderGraphError> {
        for (index, pass) in self.passes.iter().enumerate() {
            let id = PassId(index as u32);
            for version in pass.consumed() {
                self.check_read(producers, Some(id), version)?;
            }
        }
        Ok(())
    }

    fn check_output(&self, producers: &[Vec<Option<Producer>>]) -> Result<(), RenderGraphError> {
        let Some(output) = self.output else {
            return Ok(());
        };
        let resource = self.resource_of(None, output.resource)?;
        if !matches!(resource.lifetime, ResourceLifetime::Imported(_)) {
            return Err(RenderGraphError::OutputNotImported {
                resource: output.resource,
            });
        }
        self.check_read(producers, None, output)?;
        let latest = producers[output.resource.index()]
            .iter()
            .rposition(Option::is_some)
            .unwrap_or(0) as u32;
        if output.version != latest {
            return Err(RenderGraphError::OutputNotLatest { output, latest });
        }
        Ok(())
    }

    /// RAW·WAW·WAR 제약으로 실행 순서를 정합니다(Kahn, 작은 PassId 우선).
    fn schedule(
        &self,
        producers: &[Vec<Option<Producer>>],
    ) -> Result<Vec<PassId>, RenderGraphError> {
        let pass_count = self.passes.len();
        let producer_of = |version: ResourceVersion| -> Option<PassId> {
            producers
                .get(version.resource.index())
                .and_then(|slots| slots.get(version.version as usize))
                .copied()
                .flatten()
                .map(|producer| producer.pass)
        };
        // version별 reader 목록(WAR).
        let mut readers: Vec<Vec<Vec<PassId>>> = producers
            .iter()
            .map(|slots| vec![Vec::new(); slots.len()])
            .collect();
        for (index, pass) in self.passes.iter().enumerate() {
            for version in pass.consumed() {
                if let Some(slot) =
                    readers[version.resource.index()].get_mut(version.version as usize)
                {
                    slot.push(PassId(index as u32));
                }
            }
        }
        let mut edges: BTreeSet<(PassId, PassId)> = BTreeSet::new();
        for (index, pass) in self.passes.iter().enumerate() {
            let id = PassId(index as u32);
            for version in pass.consumed() {
                if let Some(producer) = producer_of(version) {
                    edges.insert((producer, id));
                }
            }
            for target in pass.produced() {
                let previous = ResourceVersion::new(target.resource, target.version - 1);
                if let Some(producer) = producer_of(previous) {
                    edges.insert((producer, id));
                }
                for &reader in &readers[target.resource.index()][previous.version as usize] {
                    edges.insert((reader, id));
                }
            }
        }
        edges.retain(|(from, to)| from != to);

        let mut successors: Vec<Vec<PassId>> = vec![Vec::new(); pass_count];
        let mut predecessors: Vec<Vec<PassId>> = vec![Vec::new(); pass_count];
        let mut in_degree = vec![0usize; pass_count];
        for &(from, to) in &edges {
            successors[from.index()].push(to);
            predecessors[to.index()].push(from);
            in_degree[to.index()] += 1;
        }
        let mut ready: BinaryHeap<Reverse<PassId>> = (0..pass_count)
            .filter(|&index| in_degree[index] == 0)
            .map(|index| Reverse(PassId(index as u32)))
            .collect();
        let mut order = Vec::with_capacity(pass_count);
        while let Some(Reverse(pass)) = ready.pop() {
            order.push(pass);
            for &next in &successors[pass.index()] {
                in_degree[next.index()] -= 1;
                if in_degree[next.index()] == 0 {
                    ready.push(Reverse(next));
                }
            }
        }
        if order.len() == pass_count {
            return Ok(order);
        }
        Err(RenderGraphError::Cycle {
            passes: find_cycle(&in_degree, &predecessors),
        })
    }

    /// 실행 순서 기준 transient 사용 구간과 alias slot을 계산합니다.
    fn liveness(&self, order: &[PassId]) -> (Vec<ResourceLiveness>, u32) {
        let mut ranges: Vec<Option<LiveRange>> = vec![None; self.resources.len()];
        for (step, pass) in order.iter().enumerate() {
            let pass = &self.passes[pass.index()];
            for version in pass.produced().chain(pass.consumed()) {
                let range = &mut ranges[version.resource.index()];
                *range = Some(match *range {
                    None => LiveRange {
                        first: step,
                        last: step,
                    },
                    Some(existing) => LiveRange {
                        first: existing.first.min(step),
                        last: existing.last.max(step),
                    },
                });
            }
        }
        // 처음 사용 단계 순으로 slot을 배정합니다(같은 descriptor, 겹치지 않는 구간 재사용).
        let mut transients: Vec<(usize, LiveRange)> = ranges
            .iter()
            .enumerate()
            .filter(|(index, _)| self.resources[*index].lifetime == ResourceLifetime::Transient)
            .filter_map(|(index, range)| range.map(|range| (index, range)))
            .collect();
        transients.sort_by_key(|(index, range)| (range.first, *index));
        let mut slots: Vec<(TextureDesc, Vec<LiveRange>)> = Vec::new();
        let mut assigned: Vec<Option<u32>> = vec![None; self.resources.len()];
        for (index, range) in transients {
            let desc = self.resources[index].desc;
            let reuse = slots.iter().position(|(slot_desc, used)| {
                *slot_desc == desc && used.iter().all(|other| !other.overlaps(&range))
            });
            let slot = match reuse {
                Some(slot) => {
                    slots[slot].1.push(range);
                    slot
                }
                None => {
                    slots.push((desc, vec![range]));
                    slots.len() - 1
                }
            };
            assigned[index] = Some(slot as u32);
        }
        let liveness = self
            .resources
            .iter()
            .enumerate()
            .map(|(index, resource)| match resource.lifetime {
                ResourceLifetime::Imported(_) => ResourceLiveness::External,
                ResourceLifetime::Transient => match (ranges[index], assigned[index]) {
                    (Some(range), Some(slot)) => ResourceLiveness::Transient { range, slot },
                    _ => ResourceLiveness::Unused,
                },
            })
            .collect();
        (liveness, slots.len() as u32)
    }
}

/// Kahn 이후 남은 pass에서 cycle 하나를 찾습니다.
///
/// 남은 pass는 모두 남은 선행 pass를 가지므로, 가장 작은 id의 남은 선행 pass를 따라 거슬러 가면
/// 반드시 같은 pass를 다시 만납니다. 반복 구간을 의존 방향으로 뒤집고 가장 작은 id가 앞에 오도록
/// 회전해 돌려줍니다(결정론적).
fn find_cycle(in_degree: &[usize], predecessors: &[Vec<PassId>]) -> Vec<PassId> {
    let mut cycle = walk_back_to_cycle(in_degree, predecessors);
    if let Some(min_at) = cycle
        .iter()
        .enumerate()
        .min_by_key(|(_, pass)| **pass)
        .map(|(index, _)| index)
    {
        cycle.rotate_left(min_at);
    }
    cycle
}

fn walk_back_to_cycle(in_degree: &[usize], predecessors: &[Vec<PassId>]) -> Vec<PassId> {
    let remaining = |pass: PassId| in_degree[pass.index()] > 0;
    let Some(start) = (0..in_degree.len())
        .map(|index| PassId(index as u32))
        .find(|&pass| remaining(pass))
    else {
        return Vec::new();
    };
    let mut visited_at: Vec<Option<usize>> = vec![None; in_degree.len()];
    let mut path = Vec::new();
    let mut current = start;
    loop {
        if let Some(position) = visited_at[current.index()] {
            let mut cycle: Vec<PassId> = path[position..].to_vec();
            cycle.reverse();
            return cycle;
        }
        visited_at[current.index()] = Some(path.len());
        path.push(current);
        let Some(&previous) = predecessors[current.index()]
            .iter()
            .filter(|&&pass| remaining(pass))
            .min()
        else {
            // 이론상 도달하지 않습니다(남은 pass는 남은 선행 pass를 가짐).
            return path;
        };
        current = previous;
    }
}

/// [`RenderGraph`] 구성 도우미. 쓰기마다 새 version handle을 돌려줍니다.
///
/// 최신 version이 아닌 handle에 쓰면(같은 resource를 갈라 쓰기) 즉시 거부합니다. 최종 검증은
/// [`RenderGraph::compile`]이 합니다.
#[derive(Debug, Clone, Default)]
pub struct RenderGraphBuilder {
    graph: RenderGraph,
    latest: Vec<u32>,
}

/// builder 오류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphBuildError {
    /// 최신 version이 아닌 handle에 쓰려 합니다.
    StaleVersion {
        /// 사용한 handle.
        handle: ResourceVersion,
        /// 현재 최신 version.
        latest: u32,
    },
    /// 없는 pass·resource입니다.
    Unknown,
    /// version이 `u32` 범위를 넘었습니다.
    VersionOverflow,
}

impl fmt::Display for GraphBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphBuildError::StaleVersion { handle, latest } => write!(
                f,
                "resource {} version {}은 최신({latest})이 아닙니다",
                handle.resource.0, handle.version
            ),
            GraphBuildError::Unknown => f.write_str("없는 pass 또는 resource입니다"),
            GraphBuildError::VersionOverflow => f.write_str("resource version 범위를 넘었습니다"),
        }
    }
}

impl std::error::Error for GraphBuildError {}

impl RenderGraphBuilder {
    /// 빈 builder.
    pub fn new() -> RenderGraphBuilder {
        RenderGraphBuilder::default()
    }

    fn declare(&mut self, resource: GraphResource) -> ResourceVersion {
        let id = ResourceId(self.graph.resources.len() as u32);
        self.graph.resources.push(resource);
        self.latest.push(0);
        ResourceVersion::new(id, 0)
    }

    /// imported resource를 선언합니다(version 0 = 외부 내용).
    pub fn import(
        &mut self,
        label: &'static str,
        desc: TextureDesc,
        kind: ImportKind,
    ) -> ResourceVersion {
        self.declare(GraphResource {
            label,
            desc,
            lifetime: ResourceLifetime::Imported(kind),
        })
    }

    /// transient resource를 선언합니다(version 0 = 정의되지 않음).
    pub fn transient(&mut self, label: &'static str, desc: TextureDesc) -> ResourceVersion {
        self.declare(GraphResource {
            label,
            desc,
            lifetime: ResourceLifetime::Transient,
        })
    }

    /// pass를 추가합니다.
    pub fn pass(&mut self, label: &'static str, kind: PassKind) -> PassId {
        let id = PassId(self.graph.passes.len() as u32);
        self.graph.passes.push(RenderPass::new(label, kind));
        id
    }

    fn next_version(
        &mut self,
        handle: ResourceVersion,
    ) -> Result<ResourceVersion, GraphBuildError> {
        let latest = self
            .latest
            .get_mut(handle.resource.index())
            .ok_or(GraphBuildError::Unknown)?;
        if *latest != handle.version {
            return Err(GraphBuildError::StaleVersion {
                handle,
                latest: *latest,
            });
        }
        let next = latest
            .checked_add(1)
            .ok_or(GraphBuildError::VersionOverflow)?;
        *latest = next;
        Ok(ResourceVersion::new(handle.resource, next))
    }

    fn pass_mut(&mut self, pass: PassId) -> Result<&mut RenderPass, GraphBuildError> {
        self.graph
            .passes
            .get_mut(pass.index())
            .ok_or(GraphBuildError::Unknown)
    }

    /// `handle` 내용에 color attachment로 그리고 새 version을 돌려줍니다.
    pub fn color(
        &mut self,
        pass: PassId,
        handle: ResourceVersion,
        load: LoadOp,
        clear: ClearColor,
        store: StoreOp,
    ) -> Result<ResourceVersion, GraphBuildError> {
        self.pass_mut(pass)?;
        let target = self.next_version(handle)?;
        self.pass_mut(pass)?.color.push(ColorAttachment {
            target,
            load,
            clear,
            store,
        });
        Ok(target)
    }

    /// depth·stencil attachment를 쓰고 새 version을 돌려줍니다.
    pub fn depth_stencil(
        &mut self,
        pass: PassId,
        handle: ResourceVersion,
        load: LoadOp,
        store: StoreOp,
    ) -> Result<ResourceVersion, GraphBuildError> {
        self.pass_mut(pass)?;
        let target = self.next_version(handle)?;
        self.pass_mut(pass)?.depth_stencil = Some(DepthStencilAttachment {
            target,
            load,
            clear_depth: FiniteF32::ONE,
            clear_stencil: 0,
            store,
        });
        Ok(target)
    }

    /// attachment가 아닌 쓰기(Compute·Transfer)로 새 version을 생산합니다.
    pub fn write(
        &mut self,
        pass: PassId,
        handle: ResourceVersion,
    ) -> Result<ResourceVersion, GraphBuildError> {
        self.pass_mut(pass)?;
        let target = self.next_version(handle)?;
        self.pass_mut(pass)?.writes.push(target);
        Ok(target)
    }

    /// `handle` version을 읽습니다.
    pub fn read(&mut self, pass: PassId, handle: ResourceVersion) -> Result<(), GraphBuildError> {
        if handle.resource.index() >= self.graph.resources.len() {
            return Err(GraphBuildError::Unknown);
        }
        self.pass_mut(pass)?.reads.push(handle);
        Ok(())
    }

    /// graph output을 지정합니다.
    pub fn set_output(&mut self, handle: ResourceVersion) {
        self.graph.output = Some(handle);
    }

    /// graph 기술을 돌려줍니다.
    pub fn finish(self) -> RenderGraph {
        self.graph
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLOR: TextureDesc = TextureDesc::surface(TextureFormat::Bgra8Unorm);
    const LAYER: TextureDesc = TextureDesc::surface(TextureFormat::Rgba16Float);
    const DEPTH: TextureDesc = TextureDesc::surface(TextureFormat::Depth32FloatStencil8);

    fn surface(label: &'static str) -> GraphResource {
        GraphResource {
            label,
            desc: COLOR,
            lifetime: ResourceLifetime::Imported(ImportKind::Surface),
        }
    }

    fn transient(label: &'static str, desc: TextureDesc) -> GraphResource {
        GraphResource {
            label,
            desc,
            lifetime: ResourceLifetime::Transient,
        }
    }

    fn v(resource: u32, version: u32) -> ResourceVersion {
        ResourceVersion::new(ResourceId(resource), version)
    }

    fn color(target: ResourceVersion, load: LoadOp) -> ColorAttachment {
        ColorAttachment {
            target,
            load,
            clear: ClearColor::TRANSPARENT,
            store: StoreOp::Store,
        }
    }

    fn raster(
        label: &'static str,
        attachments: &[ColorAttachment],
        reads: &[ResourceVersion],
    ) -> RenderPass {
        RenderPass {
            color: attachments.to_vec(),
            reads: reads.to_vec(),
            ..RenderPass::new(label, PassKind::Raster)
        }
    }

    fn ids(passes: &[u32]) -> Vec<PassId> {
        passes.iter().map(|&p| PassId(p)).collect()
    }

    /// offscreen layer → opaque → transparent(layer 합성) 구성, 선언 순서는 뒤섞음.
    fn frame_graph() -> RenderGraph {
        RenderGraph {
            resources: vec![
                surface("surface"),
                transient("layer", LAYER),
                transient("depth", DEPTH),
            ],
            passes: vec![
                // 0: transparent — surface v1을 이어 그리며 layer v1을 합성.
                RenderPass {
                    depth_stencil: Some(DepthStencilAttachment {
                        target: v(2, 2),
                        load: LoadOp::Load,
                        clear_depth: FiniteF32::ONE,
                        clear_stencil: 0,
                        store: StoreOp::Discard,
                    }),
                    ..raster("transparent", &[color(v(0, 2), LoadOp::Load)], &[v(1, 1)])
                },
                // 1: offscreen layer.
                raster("layer", &[color(v(1, 1), LoadOp::Clear)], &[]),
                // 2: opaque — surface clear, depth clear.
                RenderPass {
                    depth_stencil: Some(DepthStencilAttachment {
                        target: v(2, 1),
                        load: LoadOp::Clear,
                        clear_depth: FiniteF32::ONE,
                        clear_stencil: 0,
                        store: StoreOp::Store,
                    }),
                    ..raster("opaque", &[color(v(0, 1), LoadOp::Clear)], &[])
                },
            ],
            output: Some(v(0, 2)),
        }
    }

    #[test]
    fn dependencies_follow_resource_flow_not_declaration_order() {
        let compiled = frame_graph().compile().unwrap();
        // layer(1)와 opaque(2)는 서로 독립이라 작은 id 먼저, transparent(0)는 둘 다 기다립니다.
        assert_eq!(compiled.order(), ids(&[1, 2, 0]).as_slice());
        assert_eq!(compiled.output(), Some(v(0, 2)));
        assert_eq!(compiled.pass(PassId(0)).unwrap().label, "transparent");
        assert_eq!(
            compiled.liveness(),
            &[
                ResourceLiveness::External,
                ResourceLiveness::Transient {
                    range: LiveRange { first: 0, last: 2 },
                    slot: 0
                },
                ResourceLiveness::Transient {
                    range: LiveRange { first: 1, last: 2 },
                    slot: 1
                },
            ]
        );
        assert_eq!(compiled.alias_slot_count(), 2);
    }

    #[test]
    fn cycles_are_rejected_with_deterministic_path() {
        // 두 pass가 서로의 결과를 읽음: 0 → 1 → 0.
        let two = RenderGraph {
            resources: vec![transient("a", LAYER), transient("b", LAYER)],
            passes: vec![
                raster("p0", &[color(v(0, 1), LoadOp::Clear)], &[v(1, 1)]),
                raster("p1", &[color(v(1, 1), LoadOp::Clear)], &[v(0, 1)]),
            ],
            output: None,
        };
        assert_eq!(
            two.compile(),
            Err(RenderGraphError::Cycle {
                passes: ids(&[0, 1])
            })
        );

        // 세 pass cycle과 cycle 밖 하류 pass: 0 → 1 → 2 → 0, 2 → 3.
        let three = RenderGraph {
            resources: vec![
                transient("a", LAYER),
                transient("b", LAYER),
                transient("c", LAYER),
                transient("d", LAYER),
            ],
            passes: vec![
                raster("p0", &[color(v(0, 1), LoadOp::Clear)], &[v(2, 1)]),
                raster("p1", &[color(v(1, 1), LoadOp::Clear)], &[v(0, 1)]),
                raster("p2", &[color(v(2, 1), LoadOp::Clear)], &[v(1, 1)]),
                raster("p3", &[color(v(3, 1), LoadOp::Clear)], &[v(2, 1)]),
            ],
            output: None,
        };
        let error = three.compile().unwrap_err();
        assert_eq!(
            error,
            RenderGraphError::Cycle {
                passes: ids(&[0, 1, 2])
            }
        );
        assert!(error.to_string().contains("cycle"));

        // Load가 만드는 cycle: p1은 a v1을 이어 그려 a v2를 만들면서 p0이 만든 b v1을 읽고,
        // p0은 p1이 만든 a v2를 읽습니다(p0 → p1 → p0). seed(a v1)는 cycle 밖입니다.
        let war = RenderGraph {
            resources: vec![transient("a", LAYER), transient("b", LAYER)],
            passes: vec![
                raster("p0", &[color(v(1, 1), LoadOp::Clear)], &[v(0, 2)]),
                raster("p1", &[color(v(0, 2), LoadOp::Load)], &[v(1, 1)]),
                raster("seed", &[color(v(0, 1), LoadOp::Clear)], &[]),
            ],
            output: None,
        };
        assert_eq!(
            war.compile(),
            Err(RenderGraphError::Cycle {
                passes: ids(&[0, 1])
            })
        );
    }

    #[test]
    fn missing_resources_are_rejected() {
        let mut unknown = frame_graph();
        unknown.passes[0].reads.push(v(9, 1));
        assert_eq!(
            unknown.compile(),
            Err(RenderGraphError::UnknownResource {
                pass: Some(PassId(0)),
                resource: ResourceId(9)
            })
        );

        let mut unknown_target = frame_graph();
        unknown_target.passes[1].color[0].target = v(7, 1);
        assert_eq!(
            unknown_target.compile(),
            Err(RenderGraphError::UnknownResource {
                pass: Some(PassId(1)),
                resource: ResourceId(7)
            })
        );

        // layer pass 제거: transparent가 읽는 layer v1 생산자가 없습니다.
        let mut no_producer = frame_graph();
        no_producer.passes[1] = raster("noop", &[color(v(0, 3), LoadOp::Load)], &[]);
        no_producer.output = Some(v(0, 3));
        assert_eq!(
            no_producer.compile(),
            Err(RenderGraphError::MissingProducer {
                pass: Some(PassId(0)),
                version: v(1, 1)
            })
        );

        // 쓰기 전 transient(version 0) 읽기.
        let mut undefined = frame_graph();
        undefined.passes[0].reads = vec![v(1, 0)];
        assert_eq!(
            undefined.compile(),
            Err(RenderGraphError::UndefinedTransientRead {
                pass: PassId(0),
                resource: ResourceId(1)
            })
        );

        // transient를 Load로 이어 그리기 시작(이전 내용 없음).
        let load_undefined = RenderGraph {
            resources: vec![transient("a", LAYER)],
            passes: vec![raster("p0", &[color(v(0, 1), LoadOp::Load)], &[])],
            output: None,
        };
        assert_eq!(
            load_undefined.compile(),
            Err(RenderGraphError::UndefinedTransientRead {
                pass: PassId(0),
                resource: ResourceId(0)
            })
        );

        // imported version 0 읽기는 외부 내용이라 허용합니다.
        let imported = RenderGraph {
            resources: vec![surface("surface")],
            passes: vec![raster("p0", &[color(v(0, 1), LoadOp::Load)], &[])],
            output: Some(v(0, 1)),
        };
        assert!(imported.compile().is_ok());

        // output version 생산자 없음.
        let mut output = frame_graph();
        output.output = Some(v(0, 5));
        assert_eq!(
            output.compile(),
            Err(RenderGraphError::MissingProducer {
                pass: None,
                version: v(0, 5)
            })
        );
    }

    #[test]
    fn structural_rules_are_enforced() {
        let mut duplicate = frame_graph();
        duplicate.passes[1].color[0].target = v(0, 1);
        duplicate.passes[0].reads.clear();
        assert_eq!(
            duplicate.compile(),
            Err(RenderGraphError::DuplicateProducer {
                version: v(0, 1),
                first: PassId(1),
                second: PassId(2)
            })
        );

        let gap = RenderGraph {
            resources: vec![transient("a", LAYER)],
            passes: vec![raster("p0", &[color(v(0, 2), LoadOp::Clear)], &[])],
            output: None,
        };
        assert_eq!(
            gap.compile(),
            Err(RenderGraphError::VersionGap { version: v(0, 2) })
        );

        let zero = RenderGraph {
            resources: vec![transient("a", LAYER)],
            passes: vec![raster("p0", &[color(v(0, 0), LoadOp::Clear)], &[])],
            output: None,
        };
        assert_eq!(
            zero.compile(),
            Err(RenderGraphError::WritesInitialVersion {
                pass: PassId(0),
                resource: ResourceId(0)
            })
        );

        // layer v1을 읽으면서 같은 layer v2를 attachment로 씀.
        let feedback = RenderGraph {
            resources: vec![transient("a", LAYER)],
            passes: vec![
                raster("p0", &[color(v(0, 1), LoadOp::Clear)], &[]),
                raster("p1", &[color(v(0, 2), LoadOp::Clear)], &[v(0, 1)]),
            ],
            output: None,
        };
        assert_eq!(
            feedback.compile(),
            Err(RenderGraphError::FeedbackLoop {
                pass: PassId(1),
                resource: ResourceId(0)
            })
        );

        let discarded = RenderGraph {
            resources: vec![transient("a", LAYER), surface("s")],
            passes: vec![
                raster(
                    "p0",
                    &[ColorAttachment {
                        store: StoreOp::Discard,
                        ..color(v(0, 1), LoadOp::Clear)
                    }],
                    &[],
                ),
                raster("p1", &[color(v(1, 1), LoadOp::Clear)], &[v(0, 1)]),
            ],
            output: Some(v(1, 1)),
        };
        assert_eq!(
            discarded.compile(),
            Err(RenderGraphError::DiscardedRead {
                version: v(0, 1),
                reader: Some(PassId(1))
            })
        );

        let depth_as_color = RenderGraph {
            resources: vec![transient("d", DEPTH)],
            passes: vec![raster("p0", &[color(v(0, 1), LoadOp::Clear)], &[])],
            output: None,
        };
        assert_eq!(
            depth_as_color.compile(),
            Err(RenderGraphError::AttachmentFormatMismatch {
                pass: PassId(0),
                resource: ResourceId(0)
            })
        );

        let fixed = TextureDesc {
            size: AttachmentSize::Fixed {
                width: 64,
                height: 64,
            },
            ..LAYER
        };
        let size_mismatch = RenderGraph {
            resources: vec![transient("a", LAYER), transient("b", fixed)],
            passes: vec![raster(
                "p0",
                &[color(v(0, 1), LoadOp::Clear), color(v(1, 1), LoadOp::Clear)],
                &[],
            )],
            output: None,
        };
        assert_eq!(
            size_mismatch.compile(),
            Err(RenderGraphError::AttachmentSizeMismatch { pass: PassId(0) })
        );

        let empty_raster = RenderGraph {
            resources: vec![],
            passes: vec![RenderPass::new("empty", PassKind::Raster)],
            output: None,
        };
        assert_eq!(
            empty_raster.compile(),
            Err(RenderGraphError::InvalidAttachments { pass: PassId(0) })
        );
        let compute_with_attachment = RenderGraph {
            resources: vec![transient("a", LAYER)],
            passes: vec![RenderPass {
                kind: PassKind::Compute,
                ..raster("c", &[color(v(0, 1), LoadOp::Clear)], &[])
            }],
            output: None,
        };
        assert_eq!(
            compute_with_attachment.compile(),
            Err(RenderGraphError::InvalidAttachments { pass: PassId(0) })
        );

        let zero_size = RenderGraph {
            resources: vec![transient(
                "a",
                TextureDesc {
                    size: AttachmentSize::Fixed {
                        width: 0,
                        height: 4,
                    },
                    ..LAYER
                },
            )],
            passes: vec![],
            output: None,
        };
        assert_eq!(
            zero_size.compile(),
            Err(RenderGraphError::InvalidSize {
                resource: ResourceId(0)
            })
        );

        let mut transient_output = frame_graph();
        transient_output.output = Some(v(1, 1));
        assert_eq!(
            transient_output.compile(),
            Err(RenderGraphError::OutputNotImported {
                resource: ResourceId(1)
            })
        );
        let mut stale_output = frame_graph();
        stale_output.output = Some(v(0, 1));
        assert_eq!(
            stale_output.compile(),
            Err(RenderGraphError::OutputNotLatest {
                output: v(0, 1),
                latest: 2
            })
        );
    }

    #[test]
    fn write_after_read_orders_readers_before_overwrite() {
        // p0: a v1 clear. p1: a v2 clear(덮어씀). p2: a v1을 읽어 s에 그림.
        // p2가 p1보다 뒤에 선언됐어도 a v1을 읽으므로 p1보다 먼저 실행해야 합니다.
        let graph = RenderGraph {
            resources: vec![transient("a", LAYER), surface("s")],
            passes: vec![
                raster("p0", &[color(v(0, 1), LoadOp::Clear)], &[]),
                raster("p1", &[color(v(0, 2), LoadOp::Clear)], &[]),
                raster("p2", &[color(v(1, 1), LoadOp::Clear)], &[v(0, 1)]),
                raster("p3", &[color(v(1, 2), LoadOp::Load)], &[v(0, 2)]),
            ],
            output: Some(v(1, 2)),
        };
        let compiled = graph.compile().unwrap();
        assert_eq!(compiled.order(), ids(&[0, 2, 1, 3]).as_slice());
    }

    #[test]
    fn non_overlapping_transients_share_alias_slots() {
        let mut builder = RenderGraphBuilder::new();
        let surface0 = builder.import("surface", COLOR, ImportKind::Surface);
        let a = builder.transient("a", LAYER);
        let b = builder.transient("b", LAYER);
        let mask = builder.transient("mask", TextureDesc::surface(TextureFormat::R8Unorm));
        let unused = builder.transient("unused", LAYER);
        let _ = unused;

        let pa = builder.pass("a", PassKind::Raster);
        let a1 = builder
            .color(
                pa,
                a,
                LoadOp::Clear,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .unwrap();
        let c1 = builder.pass("composite a", PassKind::Raster);
        let s1 = builder
            .color(
                c1,
                surface0,
                LoadOp::Clear,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .unwrap();
        builder.read(c1, a1).unwrap();
        // a 수명이 끝난 뒤 b와 mask를 씁니다.
        let pb = builder.pass("b", PassKind::Raster);
        let b1 = builder
            .color(
                pb,
                b,
                LoadOp::Clear,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .unwrap();
        let pm = builder.pass("mask", PassKind::Raster);
        let m1 = builder
            .color(
                pm,
                mask,
                LoadOp::Clear,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .unwrap();
        let c2 = builder.pass("composite b", PassKind::Raster);
        let s2 = builder
            .color(
                c2,
                s1,
                LoadOp::Load,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .unwrap();
        builder.read(c2, b1).unwrap();
        builder.read(c2, m1).unwrap();
        builder.set_output(s2);

        // 이미 덮어쓴 version에 다시 쓰기는 즉시 거부합니다.
        assert_eq!(
            builder.color(
                c2,
                s1,
                LoadOp::Load,
                ClearColor::TRANSPARENT,
                StoreOp::Store
            ),
            Err(GraphBuildError::StaleVersion {
                handle: s1,
                latest: 2
            })
        );
        assert_eq!(builder.read(PassId(99), a1), Err(GraphBuildError::Unknown));

        let compiled = builder.finish().compile().unwrap();
        assert_eq!(compiled.order(), ids(&[0, 1, 2, 3, 4]).as_slice());
        let liveness = compiled.liveness();
        assert_eq!(
            liveness[1],
            ResourceLiveness::Transient {
                range: LiveRange { first: 0, last: 1 },
                slot: 0
            }
        );
        // b는 a와 descriptor가 같고 구간이 겹치지 않아 slot 0을 재사용합니다.
        assert_eq!(
            liveness[2],
            ResourceLiveness::Transient {
                range: LiveRange { first: 2, last: 4 },
                slot: 0
            }
        );
        // mask는 format이 달라 새 slot입니다.
        assert_eq!(
            liveness[3],
            ResourceLiveness::Transient {
                range: LiveRange { first: 3, last: 4 },
                slot: 1
            }
        );
        assert_eq!(liveness[4], ResourceLiveness::Unused);
        assert_eq!(liveness[0], ResourceLiveness::External);
        assert_eq!(compiled.alias_slot_count(), 2);
    }

    #[test]
    fn overlapping_transients_get_distinct_slots() {
        let mut builder = RenderGraphBuilder::new();
        let s0 = builder.import("surface", COLOR, ImportKind::Surface);
        let a = builder.transient("a", LAYER);
        let b = builder.transient("b", LAYER);
        let pa = builder.pass("a", PassKind::Raster);
        let a1 = builder
            .color(
                pa,
                a,
                LoadOp::Clear,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .unwrap();
        let pb = builder.pass("b", PassKind::Raster);
        let b1 = builder
            .color(
                pb,
                b,
                LoadOp::Clear,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .unwrap();
        let pc = builder.pass("composite", PassKind::Raster);
        let s1 = builder
            .color(
                pc,
                s0,
                LoadOp::Clear,
                ClearColor::TRANSPARENT,
                StoreOp::Store,
            )
            .unwrap();
        builder.read(pc, a1).unwrap();
        builder.read(pc, b1).unwrap();
        builder.set_output(s1);
        let compiled = builder.finish().compile().unwrap();
        let slots: Vec<u32> = compiled
            .liveness()
            .iter()
            .filter_map(|liveness| match liveness {
                ResourceLiveness::Transient { slot, .. } => Some(*slot),
                _ => None,
            })
            .collect();
        assert_eq!(slots, vec![0, 1]);
    }
}
