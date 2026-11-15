//! 사용자 파일 선택으로 문서 기본 font를 교체하는 창 편집.
use super::*;
use crate::document_thread::DocumentSender;
use stellar_commands::{command::Command, id::CommandOrigin, transaction::CommandTransaction};
use stellar_core_types::color::Color;
use stellar_project::fonts::document::{import_default_font, DocumentFontSelection};

impl AppRuntime {
    /// 선택한 파일과 face를 worker에서 검증한 뒤 하나의 Undo 가능한 문서 편집으로 적용합니다.
    pub fn set_document_font(
        &self,
        session: WindowSessionHandle,
        revision: DocumentRevision,
        file: PathBuf,
        face: u32,
        size: u32,
        fill: Color,
    ) -> Result<RequestId, RequestError> {
        let request = self.reserve_request()?;
        if self.shared.importing_font.swap(true, Ordering::AcqRel) {
            self.shared.release_one();
            return Err(RequestError::QueueFull);
        }
        let shared = self.shared.clone();
        let sender = self.sender.clone();
        let worker = std::thread::Builder::new()
            .name("stellar-document-font".into())
            .spawn(move || {
                let input = DocumentFontSelection {
                    file,
                    index: face,
                    size,
                    fill,
                };
                let result = std::panic::catch_unwind(|| {
                    import(&shared, &sender, session, revision, request, input)
                })
                .unwrap_or_else(|_| Err(fail("글꼴 등록 작업이 중단되었습니다")));
                shared.importing_font.store(false, Ordering::Release);
                shared.outbox().push_back(match result {
                    Ok(()) => RuntimeEvent::WindowViewChanged { request, session },
                    Err(e) => RuntimeEvent::RequestFailed {
                        request,
                        failure: e.failure,
                        message: e.message,
                    },
                });
            });
        if worker.is_err() {
            self.shared.importing_font.store(false, Ordering::Release);
            self.shared.release_one();
            return Err(RequestError::Stopped);
        }
        Ok(request)
    }
}

fn fail(message: impl std::fmt::Display) -> Failure {
    Failure::new(RequestFailure::SaveFailed, message.to_string())
}

fn import(
    shared: &Arc<AppShared>,
    sender: &DocumentSender,
    session: WindowSessionHandle,
    revision: DocumentRevision,
    request: RequestId,
    input: DocumentFontSelection,
) -> Result<(), Failure> {
    stellar_renderer_core::glyph::GlyphSize::from_sixty_fourths(input.size).map_err(fail)?;
    let ids: [StableId; 2] = stellar_project::new_ids::new_ids().map_err(fail)?;
    let ticket = sender
        .submit(move |ctx| {
            let window = frame_move::canvas_window(ctx, session)?;
            let document = window
                .document()
                .ok_or_else(|| fail("열린 문서가 없습니다"))?;
            if document.revision() != revision {
                return Err(fail("글꼴을 선택한 뒤 문서가 변경되었습니다"));
            }
            let model = document.model();
            if ids[0] == ids[1] || ids.iter().any(|id| model.contains_persistent_id(*id)) {
                return Err(fail("글꼴 ID 중복"));
            }
            Ok((model.document_id(), model.definition().text_styles.clone()))
        })
        .map_err(fail)?;
    let (id, current) = ticket.wait().map_err(fail)??;
    let state = shared.current_state();
    let project = state
        .window(session)
        .and_then(|w| state.project(w.project))
        .ok_or_else(|| unknown_window(session))?;
    project
        .verify_trust()
        .map_err(|e| Failure::new(RequestFailure::TrustUnavailable, e))?;
    let data_dir = os_user_data_base()
        .filter(|p| p.is_absolute())
        .ok_or_else(|| fail("사용자 데이터 경로 없음"))?
        .join(stellar_ops::lock::APP_DATA_DIR);
    let imported = import_default_font(&project.canonical_root, &data_dir, &current, ids, &input)
        .map_err(fail)?;
    let shared = shared.clone();
    let ticket = sender
        .submit(move |ctx| {
            let window = frame_move::canvas_window(ctx, session)?;
            let document = window
                .document()
                .ok_or_else(|| fail("열린 문서가 없습니다"))?;
            if document.revision() != revision || document.model().document_id() != id {
                return Err(fail("글꼴 등록 중 문서가 변경되었습니다"));
            }
            if current != imported.styles {
                let tx = CommandTransaction::new(
                    frame_move::command_id(window.window(), request),
                    CommandOrigin::User,
                )
                .with_command(
                    revision,
                    Command::SetDocumentTextStyles {
                        document: id,
                        styles: Box::new(imported.styles),
                    },
                );
                window.apply(&tx).map_err(view_failure)?;
            }
            let entry = ctx
                .app_sessions_mut()
                .windows
                .get_mut(&session)
                .ok_or_else(|| unknown_window(session))?;
            let styles = &entry
                .session
                .document()
                .ok_or_else(|| fail("열린 문서가 없습니다"))?
                .model()
                .definition()
                .text_styles;
            entry.canvas.fonts.sync_registrations(styles);
            entry
                .canvas
                .fonts
                .insert(&imported.reference, imported.face);
            shared.publish_state(build_state(ctx));
            Ok(())
        })
        .map_err(fail)?;
    ticket.wait().map_err(fail)?
}
