//! 이미지 선택 시 디코드한 PNG와 같은 bytes만 프로젝트로 가져오고 한 번의 문서 편집으로 배치합니다.
use super::*;
use stellar_assets::{
    media_type::MediaType, policy::AssetImportPolicy, record::AssetRecord, store::AssetStore,
};
use stellar_commands::{
    command::Command, id::CommandOrigin, node_ops::InsertNode, transaction::CommandTransaction,
};
use stellar_core_types::{
    order_key::OrderKey,
    reference::AssetRef,
    value::{Size2, Transform2D, Vec2},
};
use stellar_document::{
    image::ImagePayload,
    modifier::{ModifierEntry, ModifierId, ModifierKind, ModifierPayload, ModifierStack},
    records::{NodePayload, NodeRecord},
};
use stellar_storage::{
    chunk::{kinds::KIND_ASSET_METADATA, store::ChunkStore},
    hash::ContentHash,
    source_file::read_resolved,
};

const MAX_PNG_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PNG_PIXELS: u64 = 16 * 1024 * 1024;

/// 선택한 원본과 캔버스 hit의 화면 기준입니다.
pub struct ImagePlacement {
    /// 이미지가 배치될 창·View·문서 revision 및 selection history.
    pub target: SelectionEditTarget,
    /// 클릭한 편집 가능한 Frame.
    pub parent: StableId,
    /// 부모 로컬 좌표.
    pub position: Vec2,
    /// 사용자가 고른 파일의 절대 경로.
    pub source: PathBuf,
    /// 플랫폼 디코더가 실제로 연 PNG의 SHA-256.
    pub digest: [u8; 32],
}

fn invalid(message: impl Into<String>) -> Failure {
    Failure::new(RequestFailure::SaveFailed, message)
}

fn png_size(bytes: &[u8]) -> Option<Size2> {
    if bytes.get(..16)? != b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"
        || !bytes.ends_with(b"\0\0\0\0IEND\xaeB`\x82")
    {
        return None;
    }
    let w = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let h = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    if w == 0 || h == 0 || w > 16384 || h > 16384 || u64::from(w) * u64::from(h) > MAX_PNG_PIXELS {
        return None;
    }
    Size2::new(w as f32, h as f32).ok()
}

impl AppRuntime {
    /// 원본 hash와 보안 경계를 worker에서 다시 확인합니다. 최종 명령은 document thread에서 실행합니다.
    pub fn place_png(&self, input: ImagePlacement) -> Result<RequestId, RequestError> {
        let request = self.reserve_request()?;
        if self.shared.importing_image.swap(true, Ordering::AcqRel) {
            self.shared.release_one();
            return Err(RequestError::QueueFull);
        }
        let sender = self.sender.clone();
        let shared = self.shared.clone();
        let session = input.target.session;
        let result = std::thread::Builder::new()
            .name("stellar-image-import".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| -> Result<(), Failure> {
                    let state = shared.current_state();
                    let window = state
                        .window(session)
                        .ok_or_else(|| unknown_window(session))?;
                    let owner = state
                        .project(window.project)
                        .ok_or_else(|| unknown_window(session))?;
                    owner.verify_trust().map_err(invalid)?;
                    if owner.trust != TrustStatus::Trusted {
                        return Err(invalid("프로젝트 신뢰가 필요합니다"));
                    }
                    if window.active_view != Some(input.target.view)
                        || window.revision != input.target.revision
                        || window.history_sequence != input.target.history_sequence
                    {
                        return Err(invalid("배치할 문서가 바뀌었습니다"));
                    }
                    let root = &owner.canonical_root;
                    let generation = window.image_generation;
                    let current = || {
                        stellar_storage::save::optimistic::check_base_manifest(root, generation)
                            .map_err(|e| invalid(e.to_string()))
                    };
                    current()?;
                    if !input.source.is_absolute() {
                        return Err(invalid("외부 PNG 파일의 절대 경로가 필요합니다"));
                    }
                    let source = input
                        .source
                        .canonicalize()
                        .map_err(|e| invalid(e.to_string()))?;
                    if let Ok(rel) = source.strip_prefix(root) {
                        if rel.to_str().is_none_or(|s| {
                            stellar_assets::project_source::protected_project_path(s).is_some()
                        }) {
                            return Err(invalid("프로젝트 저장 영역의 파일은 가져올 수 없습니다"));
                        }
                    }
                    let bytes = read_resolved(&source, MAX_PNG_BYTES)
                        .map_err(|e| invalid(e.to_string()))?;
                    let size = png_size(&bytes)
                        .ok_or_else(|| invalid("지원 크기 이내의 PNG 파일이 필요합니다"))?;
                    let hash = ContentHash::compute(&bytes);
                    if hash.digest() != &input.digest {
                        return Err(invalid("선택한 PNG가 바뀌었습니다"));
                    }
                    let len = bytes.len() as u64;
                    drop(bytes);
                    let policy = AssetImportPolicy::from_config_str(include_str!(
                        "../../../../config/asset-import.yaml"
                    ))
                    .map_err(|e| invalid(e.to_string()))?;
                    let media = MediaType::new("image/png").map_err(|e| invalid(e.to_string()))?;
                    let import = policy
                        .request(&source, &media)
                        .map_err(|e| invalid(e.to_string()))?
                        .without_source_metadata()
                        .with_expected_content(hash, len)
                        .map_err(|e| invalid(e.to_string()))?;
                    owner.verify_trust().map_err(invalid)?;
                    let record = AssetStore::new(root)
                        .import_file(&import)
                        .map_err(|e| invalid(e.to_string()))?
                        .record;
                    let ids =
                        stellar_project::new_ids::new_ids().map_err(|e| invalid(e.to_string()))?;
                    current()?;
                    owner.verify_trust().map_err(invalid)?;
                    let publish = shared.clone();
                    sender
                        .submit(move |ctx| {
                            let window = super::frame_move::trusted_canvas_window(ctx, session)?;
                            input.target.verify(window)?;
                            if window.loaded_generation().map(|g| g.manifest_hash) != generation {
                                return Err(invalid("열린 이미지 배치 세대가 바뀌었습니다"));
                            }
                            insert(window, request, &input, ids, size, record)?;
                            publish.publish_state(build_state(ctx));
                            Ok(())
                        })
                        .map_err(|e| invalid(e.to_string()))?
                        .wait()
                        .map_err(|e| invalid(e.to_string()))?
                })
                .unwrap_or_else(|_| Err(invalid("이미지 가져오기를 완료하지 못했습니다")));
                shared.importing_image.store(false, Ordering::Release);
                shared.outbox().push_back(match result {
                    Ok(()) => RuntimeEvent::WindowViewChanged { request, session },
                    Err(e) => RuntimeEvent::RequestFailed {
                        request,
                        failure: e.failure,
                        message: e.message,
                    },
                });
            });
        if result.is_err() {
            self.shared.importing_image.store(false, Ordering::Release);
            self.shared.release_one();
            return Err(RequestError::Stopped);
        }
        Ok(request)
    }
}

fn insert(
    window: &mut ProjectWindowSession,
    request: RequestId,
    input: &ImagePlacement,
    [node, transform, asset]: [StableId; 3],
    size: Size2,
    record: AssetRecord,
) -> Result<(), Failure> {
    let model = window
        .document()
        .ok_or_else(|| invalid("문서가 닫혔습니다"))?
        .model();
    super::frame_move::editable_frame(model, input.target.view, input.parent)
        .ok_or_else(|| invalid("편집 가능한 Frame이 필요합니다"))?;
    let mut command = InsertNode::single(NodeRecord {
        id: node,
        parent: Some(input.parent),
        order_key: model.next_child_order_key(input.parent),
        visible: true,
        locked: false,
        display_name: input
            .source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Image")
            .to_owned(),
        payload: NodePayload::Image(ImagePayload::new(AssetRef::new(asset), size)),
    });
    let mut stack = ModifierStack::new(node);
    stack
        .insert(ModifierEntry {
            id: ModifierId::new(transform),
            order: OrderKey::between(None, None),
            kind: ModifierKind::Transform,
            enabled: true,
            payload: ModifierPayload::Transform(
                Transform2D::new(1., 0., 0., 1., input.position.x(), input.position.y())
                    .map_err(|e| invalid(e.to_string()))?,
            ),
        })
        .map_err(|e| invalid(e.to_string()))?;
    command.modifier_stacks.push(stack);
    let tx = CommandTransaction::new(
        super::frame_move::command_id(window.window(), request),
        CommandOrigin::User,
    )
    .with_command(input.target.revision, Command::InsertNode(command));
    window.insert_and_select(&tx, node).map_err(view_failure)?;
    Arc::make_mut(&mut window.image_records).insert(asset, record);
    Ok(())
}

impl RuntimeState {
    /// Worker 전용. 현재 캔버스 행이 실제로 참조하는 메타데이터와 내용 hash를 확인합니다.
    pub fn canvas_png(
        &self,
        session: WindowSessionHandle,
        revision: u64,
        row: usize,
        node: StableId,
        get_bytes: bool,
    ) -> Option<(ContentHash, u64, Vec<u8>)> {
        let window = self.window(session)?;
        if window.revision.0 != revision {
            return None;
        }
        let frame = window.canvas.as_ref()?.frames.get(row)?;
        if frame.id != node {
            return None;
        }
        let asset = frame.image_asset?;
        let project = self.project(window.project)?;
        if project.trust != TrustStatus::Trusted {
            return None;
        }
        project.verify_trust().ok()?;
        let root = &project.canonical_root;
        let record = if let Some(record) = window.image_records.get(&asset) {
            record.clone()
        } else {
            let hash = window.asset_chunks.get(&asset)?;
            let bytes = ChunkStore::new(root)
                .read_chunk_limited(hash, KIND_ASSET_METADATA, 64 * 1024)
                .ok()?;
            AssetRecord::decode(&bytes).ok()?
        };
        if record.media_type.as_str() != "image/png" || record.size > MAX_PNG_BYTES {
            return None;
        }
        let bytes = if get_bytes {
            AssetStore::new(root)
                .read_content(&record.content_hash, record.size, MAX_PNG_BYTES)
                .ok()??
        } else {
            Vec::new()
        };
        Some((record.content_hash, record.size, bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{identity, saved_project, sid, window};
    #[test]
    fn undo_before_first_save_excludes_the_record_and_redo_saves_the_exact_reference() {
        let fixture = saved_project("placed-image-undo");
        let mut session = ProjectWindowSession::load(identity(fixture.path()), window(1)).unwrap();
        let input = ImagePlacement {
            target: SelectionEditTarget {
                session: WindowSessionHandle(1),
                view: sid(3),
                revision: session.revision(),
                history_sequence: session.history_sequence(),
            },
            parent: sid(10),
            position: Vec2::new(15., -12.).unwrap(),
            source: "logo.png".into(),
            digest: [0; 32],
        };
        let meta = AssetRecord::new(
            ContentHash::compute(b"png"),
            MediaType::new("image/png").unwrap(),
            3,
            None,
        );
        let size = Size2::new(24., 16.).unwrap();
        insert(
            &mut session,
            RequestId(1),
            &input,
            [sid(701), sid(702), sid(703)],
            size,
            meta.clone(),
        )
        .unwrap_or_else(|e| panic!("{}", e.message));
        assert!(session.undo().unwrap());
        let lock = || Ok(fixture.path().join("locks"));
        session.save_with_lock_dir(lock).unwrap();
        assert!(!session
            .loaded_generation()
            .unwrap()
            .manifest
            .chunk_pointers
            .iter()
            .any(|p| p.kind == KIND_ASSET_METADATA));
        assert!(session.redo().unwrap());
        session.save_with_lock_dir(lock).unwrap();
        let reopened = ProjectWindowSession::load(identity(fixture.path()), window(2)).unwrap();
        let model = reopened.document().unwrap().model();
        assert_eq!(
            model.payload(sid(701)),
            Some(&NodePayload::Image(ImagePayload::new(
                AssetRef::new(sid(703)),
                size
            )))
        );
        let modifier = &model.modifier_stack(sid(701)).unwrap().entries()[0];
        assert_eq!(
            modifier.payload,
            ModifierPayload::Transform(Transform2D::new(1., 0., 0., 1., 15., -12.).unwrap())
        );
        let ptr = reopened
            .loaded_generation()
            .unwrap()
            .manifest
            .chunk_pointers
            .iter()
            .find(|p| p.kind == KIND_ASSET_METADATA)
            .unwrap();
        assert_eq!(ptr.chunk_id.stable_id(), sid(703));
        assert_eq!(
            AssetRecord::decode(
                &ChunkStore::new(fixture.path())
                    .read_chunk(&ptr.content_hash, ptr.kind)
                    .unwrap()
            )
            .unwrap(),
            meta
        );
    }
}
