//! graph transient resource의 alias slot별 texture pool (XI.8.5).
//!
//! [`CompiledRenderGraph`]는 수명이 겹치지 않고 descriptor가 같은 transient에 같은 alias slot을
//! 줍니다(XI.4.1). pool은 slot마다 texture 하나를 frame 사이에 재사용하고, slot의 descriptor(format,
//! 크기 해석 결과, sample 수)가 바뀌면(예: surface resize) 다시 만듭니다. 쓰지 않는 transient
//! (`ResourceLiveness::Unused`)는 할당하지 않습니다.
//!
//! 이전 frame이 GPU에서 아직 같은 texture를 쓰는 중이어도 안전합니다. 모든 resource는 Metal hazard
//! tracking(기본값)이라 같은 queue의 뒤 command buffer는 앞 command buffer의 쓰기·읽기 뒤에 실행됩니다.
//! 교체한 texture는 그것을 참조한 command buffer가 retain하므로 즉시 놓아도 됩니다.

use stellar_renderer_core::backend::{BackendError, BackendErrorKind, SurfaceSize};
use stellar_renderer_core::render_graph::{
    AttachmentSize, CompiledRenderGraph, ResourceId, ResourceLiveness,
};

use crate::objc::Id;
use crate::texture::{create_texture, render_target_desc, MetalTexture};

/// alias slot별 transient texture.
#[derive(Debug, Default)]
pub(crate) struct TransientPool {
    slots: Vec<Option<MetalTexture>>,
}

impl TransientPool {
    /// graph가 쓰는 모든 transient slot에 맞는 texture를 준비합니다.
    pub(crate) fn prepare(
        &mut self,
        device: &Id,
        graph: &CompiledRenderGraph,
        surface: SurfaceSize,
    ) -> Result<(), BackendError> {
        for (index, liveness) in graph.liveness().iter().enumerate() {
            let ResourceLiveness::Transient { slot, .. } = *liveness else {
                continue;
            };
            let resource = u32::try_from(index)
                .ok()
                .and_then(|id| graph.resource(ResourceId(id)))
                .ok_or(BackendError::new(
                    BackendErrorKind::InvalidUsage,
                    "없는 transient resource",
                ))?;
            let (width, height) = match resource.desc.size {
                AttachmentSize::Surface => (surface.width, surface.height),
                AttachmentSize::Fixed { width, height } => (width, height),
            };
            if width == 0 || height == 0 {
                return Err(BackendError::new(
                    BackendErrorKind::InvalidUsage,
                    "0 크기 transient resource",
                ));
            }
            let desc =
                render_target_desc(resource.desc.format, width, height, resource.desc.samples);
            let slot = slot as usize;
            if self.slots.len() <= slot {
                self.slots.resize_with(slot + 1, || None);
            }
            if self.slots[slot]
                .as_ref()
                .is_some_and(|texture| texture.desc == desc)
            {
                continue;
            }
            let texture = create_texture(device, &desc)?;
            self.slots[slot] = Some(MetalTexture { texture, desc });
        }
        Ok(())
    }

    /// slot의 texture.
    pub(crate) fn get(&self, slot: u32) -> Option<&MetalTexture> {
        self.slots.get(slot as usize).and_then(Option::as_ref)
    }

    /// 할당된 slot 수.
    pub(crate) fn allocated(&self) -> usize {
        self.slots.iter().filter(|slot| slot.is_some()).count()
    }

    /// 모든 transient texture를 놓습니다(memory pressure·device 재생성).
    pub(crate) fn purge(&mut self) {
        self.slots.clear();
    }
}
