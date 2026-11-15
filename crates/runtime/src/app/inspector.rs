//! 속성 패널: 확정 선택의 CSS 속성 행 게시와 CSS 값 편집 (UX.2.2, ADR-0070).
//!
//! 행 snapshot은 창 상태를 게시할 때 확정 선택·revision·history 위치가 바뀐 경우에만 다시 만듭니다(선택 크기에
//! 비례, 문서 크기와 무관). 편집은 확정 선택 전체를 한 transaction으로 바꾸며 창 history를 거칩니다.
//! 연속 조작(scrub·화살표 반복)은 history preview로 묶어 끝에 한 항목이 됩니다(§23.3).
//! 편집은 신뢰와 무관합니다(D1).

use std::sync::Arc;

use super::frame_move::command_id;
use super::*;
use stellar_commands::id::CommandOrigin;
use stellar_commands::inspector::{edit_commands, inspector_rows, InspectorSnapshot};
use stellar_commands::transaction::CommandTransaction;

pub use stellar_commands::inspector::{parse_input, InspectorInput, InspectorProperty};

/// 편집 단계(셸 입력의 Undo 단위).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyEditPhase {
    /// 확정 입력(Return, 포커스 이동, popup·색 선택). 그 자체로 Undo 한 항목.
    Commit,
    /// 연속 조작의 중간 값. 문서에는 반영하지만 history 항목은 아직 만들지 않습니다.
    Continuous,
    /// 연속 조작의 끝. 마지막 값을 한 항목으로 확정합니다.
    Ended,
    /// 연속 조작 취소(Escape). 시작 전 값으로 돌아가고 항목을 만들지 않습니다.
    Cancel,
}

impl PropertyEditPhase {
    /// C ABI 코드(0 확정, 1 연속, 2 끝, 3 취소).
    pub fn from_code(code: u32) -> Option<PropertyEditPhase> {
        Some(match code {
            0 => PropertyEditPhase::Commit,
            1 => PropertyEditPhase::Continuous,
            2 => PropertyEditPhase::Ended,
            3 => PropertyEditPhase::Cancel,
            _ => return None,
        })
    }
}

/// 마지막으로 만든 행 snapshot과 그 근거.
#[derive(Debug, Default)]
pub(crate) struct InspectorCache {
    key: Option<(DocumentRevision, u64, Vec<StableId>)>,
    snapshot: Arc<InspectorSnapshot>,
}

/// 창의 확정 선택에 대한 행 snapshot. 근거가 같으면 이전 snapshot을 그대로 씁니다.
pub(crate) fn snapshot(
    cache: &mut InspectorCache,
    session: &ProjectWindowSession,
) -> Arc<InspectorSnapshot> {
    let selection = session.view().selection().ids();
    let key = (
        session.revision(),
        session.history_sequence(),
        selection.to_vec(),
    );
    if cache.key.as_ref() != Some(&key) {
        cache.snapshot = Arc::new(match session.document() {
            Some(document) if !selection.is_empty() => inspector_rows(document.model(), selection),
            _ => InspectorSnapshot::default(),
        });
        cache.key = Some(key);
    }
    Arc::clone(&cache.snapshot)
}

impl AppRuntime {
    /// 창의 확정 선택 전체에 속성 값 하나를 적용합니다. `target`은 셸이 표시한 snapshot의 view·revision·history
    /// 위치이고, 연속 조작 중에는 preview가 revision을 바꾸므로 view와 history 위치만 확인합니다.
    pub fn edit_selection_property(
        &self,
        target: SelectionEditTarget,
        property: InspectorProperty,
        input: InspectorInput,
        phase: PropertyEditPhase,
    ) -> Result<RequestId, RequestError> {
        let session = target.session;
        self.submit(move |ctx, request| {
            let window = window_mut(ctx, session)?;
            edit_property(window, request, target, property, &input, phase)?;
            Ok(RuntimeEvent::WindowViewChanged { request, session })
        })
    }
}

fn stale() -> Failure {
    Failure::new(RequestFailure::UnknownView, "표시한 선택이 변경되었습니다")
}

fn history_failure(error: stellar_history::entry::HistoryError) -> Failure {
    view_failure(WindowSessionEditError::History(error))
}

fn edit_property(
    window: &mut ProjectWindowSession,
    request: RequestId,
    target: SelectionEditTarget,
    property: InspectorProperty,
    input: &InspectorInput,
    phase: PropertyEditPhase,
) -> Result<(), Failure> {
    let previewing = window.history.history().preview_active();
    if window.history_sequence() != target.history_sequence
        || window.tabs().active_view() != Some(target.view)
        || (!previewing && window.revision() != target.revision)
    {
        return Err(stale());
    }
    let id = command_id(window.window(), request);
    let document = window.document.as_mut().ok_or_else(stale)?;
    // 연속 조작은 늘 시작 전 상태를 기준으로 다시 계산합니다(값·delta 모두 시작 기준).
    if previewing {
        window
            .history
            .cancel_preview(&mut document.state)
            .map_err(history_failure)?;
    }
    if phase == PropertyEditPhase::Cancel {
        return Ok(());
    }
    let selection = window.history.view().selection().ids().to_vec();
    let commands = edit_commands(
        document.state.model(),
        &selection,
        property,
        input,
        id.to_bytes(),
    )
    .map_err(|error| Failure::new(RequestFailure::UnknownView, error.0))?;
    if commands.is_empty() {
        return Ok(());
    }
    let revision = document.state.revision();
    let mut transaction = CommandTransaction::new(id, CommandOrigin::User);
    for command in commands {
        transaction = transaction.with_command(revision, command);
    }
    match phase {
        PropertyEditPhase::Commit => {
            window.apply(&transaction).map_err(view_failure)?;
        }
        PropertyEditPhase::Continuous | PropertyEditPhase::Ended => {
            // scrub control id: 속성마다 하나(같은 속성의 연속 조작만 묶임).
            let control = property
                .name()
                .bytes()
                .fold(0u64, |h, b| h.rotate_left(5) ^ u64::from(b));
            window
                .history
                .begin_scrub(control)
                .map_err(history_failure)?;
            let document = window.document.as_mut().ok_or_else(stale)?;
            if let Err(error) = window
                .history
                .update_preview(&mut document.state, &transaction)
            {
                let _ = window.history.cancel_preview(&mut document.state);
                return Err(history_failure(error));
            }
            if phase == PropertyEditPhase::Ended {
                window.history.end_preview().map_err(history_failure)?;
            }
        }
        PropertyEditPhase::Cancel => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_session::test_fixtures::{identity, saved_project, sid, window};
    use stellar_commands::css_value::CssValue;
    use stellar_commands::selection::Selection;

    fn target(session: &ProjectWindowSession) -> SelectionEditTarget {
        SelectionEditTarget {
            session: WindowSessionHandle(1),
            view: session.tabs().active_view().unwrap(),
            revision: session.revision(),
            history_sequence: session.history_sequence(),
        }
    }

    fn width(session: &ProjectWindowSession) -> String {
        let rows = inspector_rows(
            session.document().unwrap().model(),
            session.view().selection().ids(),
        );
        rows.rows
            .iter()
            .find(|r| r.property == InspectorProperty::Width)
            .unwrap()
            .value
            .clone()
    }

    #[test]
    fn scrub_previews_then_records_one_undo_entry_and_commit_records_one() {
        let project = saved_project("inspector-scrub");
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let start = width(&session);
        let depth = session.history().history().len();
        let t = target(&session);
        for (index, delta) in [1.0, 5.0, 12.0].into_iter().enumerate() {
            let phase = if index == 2 {
                PropertyEditPhase::Ended
            } else {
                PropertyEditPhase::Continuous
            };
            edit_property(
                &mut session,
                RequestId(900 + index as u64),
                t,
                InspectorProperty::Width,
                &InspectorInput::Adjust(delta),
                phase,
            )
            .ok()
            .unwrap();
        }
        // delta는 시작 값 기준이라 마지막 +12만 남습니다.
        let start_px: f32 = start.trim_end_matches("px").parse().unwrap();
        assert_eq!(width(&session), format!("{}px", start_px + 12.0));
        assert_eq!(session.history().history().len(), depth + 1);
        assert!(session.undo().unwrap());
        assert_eq!(width(&session), start);
        // 확정 입력 한 번 = Undo 한 번. 오래된 history 위치는 거부합니다.
        let t = target(&session);
        edit_property(
            &mut session,
            RequestId(910),
            t,
            InspectorProperty::Width,
            &InspectorInput::Set(CssValue::Length(320.0)),
            PropertyEditPhase::Commit,
        )
        .ok()
        .unwrap();
        assert_eq!(width(&session), "320px");
        assert!(edit_property(
            &mut session,
            RequestId(911),
            t,
            InspectorProperty::Width,
            &InspectorInput::Set(CssValue::Length(10.0)),
            PropertyEditPhase::Commit,
        )
        .is_err());
        assert!(session.undo().unwrap());
        assert_eq!(width(&session), start);
        // 취소는 시작 값으로 돌아가고 항목을 만들지 않습니다.
        let depth = session.history().history().len();
        let t = target(&session);
        edit_property(
            &mut session,
            RequestId(920),
            t,
            InspectorProperty::Width,
            &InspectorInput::Adjust(30.),
            PropertyEditPhase::Continuous,
        )
        .ok()
        .unwrap();
        edit_property(
            &mut session,
            RequestId(921),
            t,
            InspectorProperty::Width,
            &InspectorInput::Adjust(0.),
            PropertyEditPhase::Cancel,
        )
        .ok()
        .unwrap();
        assert_eq!(width(&session), start);
        assert_eq!(session.history().history().len(), depth);
    }
}
