//! frame plan의 효과 pass: layer blur, drop shadow, clip mask, 합성 program (XI.10.2·XI.10.4, ADR-0102 결정 3·5,
//! ADR-0116 결정 3).
//!
//! pass compiler는 draw 순서·layer 경계·clip 방식을 정한 뒤, 효과가 필요한 draw와 layer에 효과 pass를 graph에 더하고
//! backend가 실행할 값(device 단위 σ·offset, 합성 program)을 붙입니다. 수식은 ADR-0102 결정 5이고 순서도 같습니다.
//!
//! - **합성 program**([`Composite`]): 모든 draw의 합성 blend mode를 [`resolve_or_normal`]로 풉니다. 지원 목록 밖 tag는
//!   `render.blend.unsupported_mode` 진단([`FramePlan::blend_diagnostics`])을 남기고 Normal로 그립니다. layer 합성 draw는
//!   경계 node 자기 blend가 Normal이 아니면 그것, 아니면 접힌 blend입니다(batching과 같은 규칙). program·blend state는
//!   [`composite_program`]·[`composite_blend_state`]입니다.
//! - **layer blur**: 경계 node에 layer blur가 있으면 layer pass 뒤에 `GaussianBlur` 가로·세로 pass 두 개를 두고, 부모는 세로
//!   pass 결과를 합성합니다([`LayerEffects::blurred`]). σ = 반경(CSS `filter: blur()`).
//! - **shadow**: shadow가 있는 내용 draw는 원본 pass([`EffectStep::ShadowSource`]) 하나와, shadow마다 `GaussianBlur`
//!   가로(alpha를 모든 채널로 퍼뜨림)·세로(shadow 색을 곱함) pass 두 개를 둡니다. target pass는 내용 draw 바로 아래에 shadow
//!   결과를 offset만큼 옮겨 목록 순서(앞이 아래)로 합성합니다([`DrawEffects::shadows`]). σ = blur 반경 / 2. 원본 모양은
//!   CSS 대응을 따릅니다([`ShadowShape`], ADR-0130 결정 1). Frame(`SceneContent::Rect`)은 CSS `box-shadow`라 원본이 테두리
//!   상자(모서리 반경 포함)를 불투명하게 칠한 모양이고 합성할 때 상자 안을 비웁니다. 그래서 배경이 없거나 반투명해도 상자
//!   모양 그림자이고 상자 안에는 비치지 않습니다. 그 밖의 내용(Path·Text·Image)은 `filter: drop-shadow()`라 node 자기
//!   내용(shadow 제외)의 alpha입니다.
//! - **낮은 해상도 효과**(ADR-0130 결정 2): device σ가 [`MAX_SIGMA`](crate::effects::blur::MAX_SIGMA)를 넘으면 blur 입력을 [`effect_downsample`] 배수로
//!   줄이는 pass([`EffectStep::Downsample`])를 두고 줄인 크기 texture에서 σ / d로 blur합니다. shadow는 target 합성이,
//!   layer blur는 키우기 pass([`EffectStep::Upsample`])가 bilinear로 다시 키웁니다([`DeviceBlur::downsample`]).
//! - **clip mask**: clip 방식이 [`ClipPlan::Mask`]인 draw는 복합 clip coverage(나열한 clip의 곱, 곧 교집합)를 `R8Unorm`
//!   texture에 그리는 pass([`EffectStep::Mask`])를 두고 draw가 그 texture를 곱합니다([`DrawEffects::mask`]). 같은 clip
//!   stack·같은 복합 clip 목록의 draw는 mask 하나를 같이 씁니다.
//!
//! ## device 값
//!
//! blur·shadow 값은 node local 단위입니다. node world → device 선형 부분 `M`(열 벡터 local x축 `(a, b)`, y축 `(c, d)`)으로
//! 옮깁니다. shadow offset은 `M·offset`입니다. σ는 축마다 `σx = σ·√(a² + c²)`, `σy = σ·√(b² + d²)`이고, 분리 blur는 device
//! 가로·세로로 적용합니다. `M·Mᵀ`가 대각(축 정렬 변환, 또는 회전 + 균일 배율)이면 local 등방 gaussian을 옮긴 결과와
//! 같고, 기울임·비균일 회전은 대각 성분만 쓰는 근사입니다([`DeviceBlur::exact`]가 거짓). σ가
//! [`MAX_DEVICE_SIGMA`]를 넘으면 상한으로 그리고 [`EffectDiagnostic::BlurTooLarge`]를 남깁니다(근사를 알림).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use stellar_scene::node::SceneContent;
use stellar_scene::stack::{Opacity, SceneBlendMode};
use stellar_scene::visibility::Viewport;
use stellar_scene::{SceneNode, ZOrder};

use crate::backend::{BlendState, ShaderProgram};
use crate::blend::{composite_blend_state, composite_program, resolve_or_normal, BlendDiagnostic};
use crate::clip_plan::ClipPlan;
use crate::effects::blur::{
    downsampled_extent, effect_downsample, layer_blur_sigma, shadow_sigma, MAX_DEVICE_SIGMA,
    MAX_DOWNSAMPLE,
};
use crate::render_graph::{
    AttachmentSize, ClearColor, GraphBuildError, LoadOp, PassId, PassKind, RenderGraphBuilder,
    ResourceVersion, StoreOp, TextureDesc, TextureFormat,
};

use super::{DrawSource, LayerId, PlannedDraw, LAYER_FORMAT};

/// clip mask texture format(coverage).
pub const MASK_FORMAT: TextureFormat = TextureFormat::R8Unorm;

/// 합성 program(XI.10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Composite {
    /// 지원 목록으로 푼 blend mode.
    pub mode: SceneBlendMode,
    /// backend program.
    pub program: ShaderProgram,
    /// pipeline blend state.
    pub state: BlendState,
}

impl Composite {
    /// blend mode를 지원 목록으로 풀어(목록 밖은 진단 후 Normal) program을 정합니다.
    pub fn resolve(mode: SceneBlendMode, diagnostics: &mut Vec<BlendDiagnostic>) -> Composite {
        let mode = resolve_or_normal(mode.tag(), diagnostics);
        Composite {
            mode,
            program: composite_program(mode),
            state: composite_blend_state(mode),
        }
    }
}

/// device 축별 blur 표준편차.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeviceBlur {
    /// 가로 σ(device pixel).
    pub sigma_x: f32,
    /// 세로 σ(device pixel).
    pub sigma_y: f32,
    /// 분리 blur가 local 등방 gaussian과 정확히 같은지(`M·Mᵀ`가 대각).
    pub exact: bool,
    /// 효과를 그리는 낮은 해상도 배수(2의 거듭제곱, 1이면 device 해상도). 큰 축 σ로 [`effect_downsample`]가 정합니다.
    pub downsample: u32,
}

/// shadow 원본 모양(ADR-0130 결정 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadowShape {
    /// node 자기 내용의 alpha(CSS `filter: drop-shadow()`). Path·Text·Image.
    ContentAlpha,
    /// 테두리 상자(CSS `box-shadow`): 모서리 반경 포함 상자를 불투명하게 칠한 모양이고 합성할 때 상자 안을 비웁니다. Frame.
    BorderBox,
}

impl ShadowShape {
    /// node 내용의 CSS 대응으로 정합니다: `SceneContent::Rect`(Frame)는 `box-shadow`, 나머지는 `drop-shadow()`.
    pub fn of(node: &SceneNode) -> ShadowShape {
        match node.content() {
            SceneContent::Rect(_) => ShadowShape::BorderBox,
            _ => ShadowShape::ContentAlpha,
        }
    }
}

/// 효과 진단.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EffectDiagnostic {
    /// device σ가 상한을 넘어 상한으로 그립니다.
    BlurTooLarge {
        /// node paint 순서 key.
        z: ZOrder,
        /// 요청 device σ.
        sigma: f32,
    },
    /// 기울임·비균일 회전 변환이라 분리 blur가 근사입니다.
    AnisotropicBlur {
        /// node paint 순서 key.
        z: ZOrder,
    },
}

impl EffectDiagnostic {
    /// 정적 진단 code.
    pub const fn code(&self) -> &'static str {
        match self {
            EffectDiagnostic::BlurTooLarge { .. } => "render.effect.blur_too_large",
            EffectDiagnostic::AnisotropicBlur { .. } => "render.effect.anisotropic_blur",
        }
    }
}

/// 효과 pass 하나가 하는 일.
#[derive(Debug, Clone)]
pub enum EffectStep {
    /// shadow 원본을 투명 layer(`LAYER_FORMAT`)에 그립니다. [`ShadowShape::ContentAlpha`]는 node 자기 내용(paint layer·
    /// image·text, shadow 제외), [`ShadowShape::BorderBox`]는 테두리 상자를 불투명하게 칠한 모양입니다. clip은 적용하지
    /// 않습니다(target 합성 draw가 적용).
    ShadowSource {
        /// node.
        node: Arc<SceneNode>,
        /// 원본 모양.
        shape: ShadowShape,
    },
    /// 입력을 `factor`배(2의 거듭제곱) 줄입니다(단계마다 2×2 상자 평균, ADR-0130 결정 2). 출력은 줄인 고정 크기입니다.
    Downsample {
        /// 입력.
        source: ResourceVersion,
        /// 배수.
        factor: u32,
    },
    /// 줄인 blur 결과를 surface 크기로 bilinear 키웁니다(layer blur).
    Upsample {
        /// 입력.
        source: ResourceVersion,
        /// 입력이 줄인 배수.
        factor: u32,
    },
    /// 한 방향 gaussian blur(`GaussianBlur` program, 밖은 투명).
    Blur {
        /// 입력.
        source: ResourceVersion,
        /// device σ.
        sigma: f32,
        /// 방향(`[1, 0]` 가로, `[0, 1]` 세로).
        direction: [f32; 2],
        /// 참이면 texel alpha를 모든 채널로 퍼뜨려 읽습니다(shadow 1단계).
        alpha_to_all: bool,
        /// 곱할 shadow 색(node, shadow 번호). 없으면 tint 1.
        shadow: Option<(Arc<SceneNode>, usize)>,
    },
    /// 복합 clip coverage(나열한 clip의 곱)를 `MASK_FORMAT`에 그립니다.
    Mask {
        /// clip을 가진 node.
        node: Arc<SceneNode>,
        /// clip stack 바깥 → 안 index.
        clips: Arc<[usize]>,
    },
}

/// graph의 효과 pass.
#[derive(Debug, Clone)]
pub struct EffectPass {
    /// graph pass.
    pub pass: PassId,
    /// 하는 일.
    pub step: EffectStep,
    /// 생산한 texture version.
    pub output: ResourceVersion,
}

/// 내용 draw 아래에 합성할 shadow 하나.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadowComposite {
    /// node paint stack의 shadow 번호(목록 순서, 앞이 아래).
    pub index: usize,
    /// device offset.
    pub offset: [f32; 2],
    /// device σ.
    pub blur: DeviceBlur,
    /// 원본 모양. [`ShadowShape::BorderBox`]면 합성할 때 상자 안을 비웁니다.
    pub shape: ShadowShape,
    /// 세로 blur pass 결과(shadow 색을 곱한 premultiplied, `blur.downsample`배 줄인 크기).
    pub output: ResourceVersion,
}

/// draw 하나에 붙은 효과.
#[derive(Debug, Clone)]
pub struct DrawEffects {
    /// draw가 있는 pass.
    pub pass: PassId,
    /// 그 pass 안 paint 순위(`PlannedDraw::depth`).
    pub depth: u32,
    /// 합성 program.
    pub composite: Composite,
    /// layer 합성 draw의 경계 node 불투명도(내용 draw는 1). draw의 접힌 `opacity`와 곱합니다.
    pub node_opacity: Opacity,
    /// 내용 draw 아래 shadow(앞이 아래).
    pub shadows: Vec<ShadowComposite>,
    /// clip mask(`ClipPlan::Mask`).
    pub mask: Option<ResourceVersion>,
}

/// layer 하나의 효과.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerEffects {
    /// layer.
    pub layer: LayerId,
    /// layer blur(device σ).
    pub blur: Option<DeviceBlur>,
    /// blur 뒤 결과(부모가 합성하는 version). blur가 없으면 `None`(layer 출력 그대로).
    pub blurred: Option<ResourceVersion>,
}

/// 효과 pass 구성 상태.
pub(super) struct EffectBuilder<'v> {
    pub(super) viewport: &'v Viewport,
    pub(super) passes: Vec<EffectPass>,
    pub(super) draws: Vec<DrawEffects>,
    pub(super) layers: Vec<LayerEffects>,
    pub(super) blend_diagnostics: Vec<BlendDiagnostic>,
    pub(super) diagnostics: Vec<EffectDiagnostic>,
    /// (clip stack 저장소 주소, 복합 clip 목록) → mask. snapshot이 stack을 붙잡은 compile 동안만 씁니다.
    masks: HashMap<(usize, Arc<[usize]>), ResourceVersion>,
    /// 이미 read를 건 (pass, mask).
    mask_reads: HashSet<(PassId, ResourceVersion)>,
}

impl<'v> EffectBuilder<'v> {
    pub(super) fn new(viewport: &'v Viewport) -> EffectBuilder<'v> {
        EffectBuilder {
            viewport,
            passes: Vec::new(),
            draws: Vec::new(),
            layers: Vec::new(),
            blend_diagnostics: Vec::new(),
            diagnostics: Vec::new(),
            masks: HashMap::new(),
            mask_reads: HashSet::new(),
        }
    }

    /// node local → device 선형 부분 (a, b, c, d).
    fn linear(&self, node: &SceneNode) -> Option<(f32, f32, f32, f32)> {
        let (a, b, c, d, _, _) = node
            .world_transform()
            .then(&self.viewport.view())
            .ok()?
            .components();
        Some((a, b, c, d))
    }

    /// local σ를 device 축별 σ로 옮깁니다(모듈 문서 "device 값").
    fn device_blur(&mut self, node: &SceneNode, sigma: f32) -> DeviceBlur {
        let (a, b, c, d) = self.linear(node).unwrap_or((1.0, 0.0, 0.0, 1.0));
        let scale_x = (a * a + c * c).sqrt();
        let scale_y = (b * b + d * d).sqrt();
        // M·Mᵀ의 비대각 성분 a·b + c·d가 0이면 정확합니다.
        let off = a * b + c * d;
        let exact = off.abs() <= 1e-6 * (scale_x * scale_y).max(1e-12);
        if !exact && sigma > 0.0 {
            self.diagnostics
                .push(EffectDiagnostic::AnisotropicBlur { z: node.z() });
        }
        let finite = |value: f32| if value.is_finite() { value } else { 0.0 };
        let (mut sigma_x, mut sigma_y) = (finite(sigma * scale_x), finite(sigma * scale_y));
        let downsample = match effect_downsample(sigma_x.max(sigma_y)) {
            Some(d) => d,
            None => {
                self.diagnostics.push(EffectDiagnostic::BlurTooLarge {
                    z: node.z(),
                    sigma: sigma_x.max(sigma_y),
                });
                sigma_x = sigma_x.min(MAX_DEVICE_SIGMA);
                sigma_y = sigma_y.min(MAX_DEVICE_SIGMA);
                MAX_DOWNSAMPLE
            }
        };
        DeviceBlur {
            sigma_x,
            sigma_y,
            exact,
            downsample,
        }
    }

    /// viewport(surface) 크기 pixel.
    fn surface_extent(&self) -> (u32, u32) {
        let size = self.viewport.size();
        let side = |v: f32| (v.ceil().max(1.0)) as u32;
        (side(size.width()), side(size.height()))
    }

    /// 새 transient(surface 크기)에 그리는 효과 pass를 graph에 더합니다.
    fn pass(
        &mut self,
        builder: &mut RenderGraphBuilder,
        label: &'static str,
        format: TextureFormat,
        reads: &[ResourceVersion],
        step: EffectStep,
    ) -> Result<ResourceVersion, GraphBuildError> {
        self.sized_pass(builder, label, TextureDesc::surface(format), reads, step)
    }

    /// 새 transient(`desc`)에 그리는 효과 pass를 graph에 더합니다.
    fn sized_pass(
        &mut self,
        builder: &mut RenderGraphBuilder,
        label: &'static str,
        desc: TextureDesc,
        reads: &[ResourceVersion],
        step: EffectStep,
    ) -> Result<ResourceVersion, GraphBuildError> {
        let texture = builder.transient(label, desc);
        let pass = builder.pass(label, PassKind::Raster);
        for version in reads {
            builder.read(pass, *version)?;
        }
        let output = builder.color(
            pass,
            texture,
            LoadOp::Clear,
            ClearColor::TRANSPARENT,
            StoreOp::Store,
        )?;
        self.passes.push(EffectPass { pass, step, output });
        Ok(output)
    }

    /// (줄이기 →) 가로 → 세로 blur pass를 더하고 세로 결과를 돌려줍니다. 줄였으면 결과도 줄인 크기입니다.
    fn blur_pair(
        &mut self,
        builder: &mut RenderGraphBuilder,
        source: ResourceVersion,
        blur: DeviceBlur,
        alpha_to_all: bool,
        shadow: Option<(Arc<SceneNode>, usize)>,
    ) -> Result<ResourceVersion, GraphBuildError> {
        let d = blur.downsample.max(1);
        let mut input = source;
        let desc = if d == 1 {
            TextureDesc::surface(LAYER_FORMAT)
        } else {
            let (w, h) = self.surface_extent();
            TextureDesc {
                format: LAYER_FORMAT,
                size: AttachmentSize::Fixed {
                    width: downsampled_extent(w, d),
                    height: downsampled_extent(h, d),
                },
                samples: crate::render_graph::SampleCount::X1,
            }
        };
        if d > 1 {
            input = self.sized_pass(
                builder,
                "effect downsample",
                desc,
                &[source],
                EffectStep::Downsample { source, factor: d },
            )?;
        }
        let horizontal = self.sized_pass(
            builder,
            "effect blur",
            desc,
            &[input],
            EffectStep::Blur {
                source: input,
                sigma: blur.sigma_x / d as f32,
                direction: [1.0, 0.0],
                alpha_to_all,
                shadow: None,
            },
        )?;
        self.sized_pass(
            builder,
            "effect blur",
            desc,
            &[horizontal],
            EffectStep::Blur {
                source: horizontal,
                sigma: blur.sigma_y / d as f32,
                direction: [0.0, 1.0],
                alpha_to_all: false,
                shadow,
            },
        )
    }

    /// layer 경계 node의 layer blur를 더하고 부모가 합성할 version을 돌려줍니다.
    pub(super) fn layer(
        &mut self,
        builder: &mut RenderGraphBuilder,
        layer: LayerId,
        node: &Arc<SceneNode>,
        output: ResourceVersion,
    ) -> Result<ResourceVersion, GraphBuildError> {
        let radius = node.paint().get().layer_blur.get();
        if radius <= 0.0 {
            self.layers.push(LayerEffects {
                layer,
                blur: None,
                blurred: None,
            });
            return Ok(output);
        }
        let blur = self.device_blur(node, layer_blur_sigma(radius));
        let mut blurred = self.blur_pair(builder, output, blur, false, None)?;
        if blur.downsample > 1 {
            blurred = self.pass(
                builder,
                "effect upsample",
                LAYER_FORMAT,
                &[blurred],
                EffectStep::Upsample {
                    source: blurred,
                    factor: blur.downsample,
                },
            )?;
        }
        self.layers.push(LayerEffects {
            layer,
            blur: Some(blur),
            blurred: Some(blurred),
        });
        Ok(blurred)
    }

    /// pass의 draw마다 합성 program, shadow, mask를 붙이고 target pass가 읽을 version을 graph에 연결합니다.
    pub(super) fn draws(
        &mut self,
        builder: &mut RenderGraphBuilder,
        pass: PassId,
        draws: &[PlannedDraw],
    ) -> Result<(), GraphBuildError> {
        for draw in draws {
            let (mode, node_opacity) = match &draw.source {
                DrawSource::Layer { node, .. } => {
                    let paint = node.paint().get();
                    let mode = if paint.blend != SceneBlendMode::Normal {
                        paint.blend
                    } else {
                        draw.blend
                    };
                    (mode, paint.opacity)
                }
                DrawSource::Content(_) => (draw.blend, Opacity::OPAQUE),
            };
            let composite = Composite::resolve(mode, &mut self.blend_diagnostics);
            let mut shadows = Vec::new();
            if let DrawSource::Content(node) = &draw.source {
                let paint = node.paint().get();
                if !paint.shadows.is_empty() {
                    let shape = ShadowShape::of(node);
                    let source = self.pass(
                        builder,
                        "shadow source",
                        LAYER_FORMAT,
                        &[],
                        EffectStep::ShadowSource {
                            node: Arc::clone(node),
                            shape,
                        },
                    )?;
                    let (a, b, c, d) = self.linear(node).unwrap_or((1.0, 0.0, 0.0, 1.0));
                    for (index, shadow) in paint.shadows.iter().enumerate() {
                        let blur = self.device_blur(node, shadow_sigma(shadow.blur().get()));
                        let output = self.blur_pair(
                            builder,
                            source,
                            blur,
                            true,
                            Some((Arc::clone(node), index)),
                        )?;
                        builder.read(pass, output)?;
                        let (x, y) = (shadow.offset().x(), shadow.offset().y());
                        shadows.push(ShadowComposite {
                            index,
                            offset: [a * x + c * y, b * x + d * y],
                            blur,
                            shape,
                            output,
                        });
                    }
                }
            }
            let mask = match &draw.clip {
                ClipPlan::Mask { clips, .. } => {
                    let node = draw.source.node();
                    // 같은 clip stack·같은 복합 clip 목록의 draw는 mask 하나를 같이 씁니다(형제 draw가 많아도 texture 하나).
                    let key = (node.clip().storage_addr(), Arc::clone(clips));
                    let mask = match self.masks.get(&key) {
                        Some(mask) => *mask,
                        None => {
                            let mask = self.pass(
                                builder,
                                "clip mask",
                                MASK_FORMAT,
                                &[],
                                EffectStep::Mask {
                                    node: Arc::clone(node),
                                    clips: Arc::clone(clips),
                                },
                            )?;
                            self.masks.insert(key, mask);
                            mask
                        }
                    };
                    if self.mask_reads.insert((pass, mask)) {
                        builder.read(pass, mask)?;
                    }
                    Some(mask)
                }
                _ => None,
            };
            self.draws.push(DrawEffects {
                pass,
                depth: draw.depth,
                composite,
                node_opacity,
                shadows,
                mask,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
