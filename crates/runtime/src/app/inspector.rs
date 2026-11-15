//! 속성 패널: 확정 선택의 CSS 속성 행 게시와 CSS 값 편집 (UX.2.2, ADR-0070).
//!
//! 행 snapshot은 창 상태를 게시할 때 확정 선택·revision·history 위치·active View가 바뀐 경우에만 다시 만듭니다
//! (선택 크기에 비례, 문서 크기와 무관). 선택이 없으면 행 없이 현재 Page·View 이름만 둡니다(UX 명세 6.1).
//! 편집은 확정 선택 전체를 한 transaction으로 바꾸며 창 history를 거칩니다(§23.3).
//!
//! - 포인터 제스처(레이블 scrub)만 history preview를 씁니다. 포인터를 누르고 있는 동안이라 다른 창 요청과 겹치지
//!   않습니다. 끝(`Ended`)에서 한 항목으로 확정하고, 취소(`Cancel`)와 거부된 끝 요청은 preview를 닫습니다.
//! - 화살표 반복·색 견본·색 패널 변경은 preview를 열지 않고 요청마다 바로 기록하되, 같은 속성의 연속 입력을
//!   history grouping(VII.4.4 키보드 반복 정책)으로 한 항목에 합칩니다. 그 사이 Undo·선택·다른 편집은 막히지
//!   않습니다.
//!
//! 편집은 신뢰와 무관합니다(D1).

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use super::frame_move::{command_id, FreshIds};
use super::*;
use stellar_commands::id::CommandOrigin;
use stellar_commands::inspector::{edit_commands, inspector_rows, InspectorSnapshot, NodeClass};
use stellar_commands::kind::CommandKind;
use stellar_commands::transaction::CommandTransaction;
use stellar_core_types::clock::MonotonicClock;
use stellar_history::entry::{CommandSignature, GroupingKey};

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
    /// 반복 입력 하나(화살표, 색 견본·색 패널). 바로 기록하고, 같은 속성의 직전 반복 입력과 0.5초 안이면 그
    /// 항목에 합칩니다(Undo 한 번).
    Repeat,
}

impl PropertyEditPhase {
    /// C ABI 코드(0 확정, 1 연속, 2 끝, 3 취소, 4 반복).
    pub fn from_code(code: u32) -> Option<PropertyEditPhase> {
        Some(match code {
            0 => PropertyEditPhase::Commit,
            1 => PropertyEditPhase::Continuous,
            2 => PropertyEditPhase::Ended,
            3 => PropertyEditPhase::Cancel,
            4 => PropertyEditPhase::Repeat,
            _ => return None,
        })
    }
}

/// 반복 입력 grouping의 시각. 프로세스에서 처음 읽은 시점 이후 경과 nanoseconds입니다(단조).
pub(super) struct InstantClock;

impl MonotonicClock for InstantClock {
    fn now_nanos(&self) -> u64 {
        static ORIGIN: OnceLock<Instant> = OnceLock::new();
        let elapsed = ORIGIN.get_or_init(Instant::now).elapsed().as_nanos();
        u64::try_from(elapsed).unwrap_or(u64::MAX)
    }
}

type CacheKey = (DocumentRevision, u64, Option<StableId>, Vec<StableId>);

/// 마지막으로 만든 행 snapshot과 그 근거.
#[derive(Debug, Default)]
pub(crate) struct InspectorCache {
    key: Option<CacheKey>,
    snapshot: Arc<InspectorSnapshot>,
}

/// 창의 확정 선택에 대한 행 snapshot. 근거가 같으면 이전 snapshot을 그대로 씁니다.
pub(crate) fn snapshot(
    cache: &mut InspectorCache,
    session: &ProjectWindowSession,
) -> Arc<InspectorSnapshot> {
    let selection = session.view().selection().ids();
    let view = session.tabs().active_view();
    let key = (
        session.revision(),
        session.history_sequence(),
        view,
        selection.to_vec(),
    );
    if cache.key.as_ref() != Some(&key) {
        cache.snapshot = Arc::new(match session.document() {
            Some(document) if !selection.is_empty() => inspector_rows(document.model(), selection),
            Some(document) => InspectorSnapshot {
                display_name: view
                    .and_then(|view| page_summary(document.model(), view))
                    .unwrap_or_default(),
                ..InspectorSnapshot::default()
            },
            None => InspectorSnapshot::default(),
        });
        cache.key = Some(key);
    }
    Arc::clone(&cache.snapshot)
}

/// 선택이 없을 때 머리에 보일 현재 Page 요약(`Page · View`). View가 문서에 없으면 `None`입니다.
fn page_summary(model: &stellar_document::model::DocumentModel, view: StableId) -> Option<String> {
    model.definition().pages.iter().find_map(|page| {
        page.views
            .iter()
            .find(|v| v.id == view)
            .map(|v| format!("{} · {}", page.display_name(), v.display_name()))
    })
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
    // 취소는 표시 상태와 무관하게 열린 preview를 닫습니다. 셸이 끝 요청을 잃거나 선택이 바뀐 뒤에도 preview가
    // 남아 창의 Undo·선택·편집을 막지 않게 합니다.
    if phase == PropertyEditPhase::Cancel {
        if previewing {
            let document = window.document.as_mut().ok_or_else(stale)?;
            window
                .history
                .cancel_preview(&mut document.state)
                .map_err(history_failure)?;
        }
        return Ok(());
    }
    if window.history_sequence() != target.history_sequence
        || window.tabs().active_view() != Some(target.view)
        || (!previewing && window.revision() != target.revision)
    {
        // 거부한 요청이 preview를 열어 둔 채 끝나지 않게 합니다.
        if previewing {
            if let Some(document) = window.document.as_mut() {
                let _ = window.history.cancel_preview(&mut document.state);
            }
        }
        return Err(stale());
    }
    let id = command_id(window.window(), request);
    let document = window.document.as_mut().ok_or_else(stale)?;
    // 연속 조작은 늘 시작 전 상태를 기준으로 다시 계산합니다(값·delta 모두 시작 기준). 그 밖의 단계는 남은
    // preview를 먼저 닫습니다.
    if previewing {
        window
            .history
            .cancel_preview(&mut document.state)
            .map_err(history_failure)?;
    }
    let selection = window.history.view().selection().ids().to_vec();
    // 새 Fill·Stroke·위치 Transform modifier ID는 UUIDv7(OS 난수)입니다. 연속 조작은 단계마다 시작 상태에서 다시
    // 계산하므로 단계마다 새 ID여도 확정되는 문서에는 마지막 단계의 ID 하나만 남습니다.
    let mut ids = FreshIds::default();
    let commands = edit_commands(
        document.state.model(),
        &selection,
        property,
        input,
        &mut || ids.next(),
    );
    ids.finish()?;
    let commands = commands.map_err(|error| Failure::new(RequestFailure::UnknownView, error.0))?;
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
        PropertyEditPhase::Repeat => {
            let key = repeat_key(document.state.model(), &selection, property);
            let document = window.document.as_mut().ok_or_else(stale)?;
            window
                .history
                .record_grouped(&mut document.state, &transaction, key, &InstantClock)
                .map_err(history_failure)?;
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

/// 반복 입력의 grouping key: 같은 선택(첫 node)·같은 속성 저장 key의 반복만 합칩니다. 선택 변경·다른 편집은
/// history가 열린 grouping을 닫으므로 그 뒤 입력은 새 항목입니다. `left`·`top`은 같은 저장 key(이동 성분)라
/// 0.5초 안에 이어 바꾸면 이동 한 번으로 합쳐집니다.
pub(super) fn repeat_key(
    model: &stellar_document::model::DocumentModel,
    selection: &[StableId],
    property: InspectorProperty,
) -> GroupingKey {
    let target = selection.first().copied().unwrap_or(StableId::NIL);
    let class = model
        .payload(target)
        .map_or(NodeClass::Other, NodeClass::of);
    GroupingKey::KeyboardRepeat(CommandSignature {
        kind: CommandKind::SetProperty,
        target,
        key: Some(property.key(class)),
    })
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

    /// 속성 탭이 처음 넣는 modifier(Fill, 위치 Transform)의 ID는 서로 독립인 UUIDv7이고, 같은 창·요청 순번으로
    /// 다시 해도 같은 ID가 나오지 않습니다(command ID에서 파생하지 않음, ADR-0028 "새 영속 ID").
    #[test]
    fn new_modifiers_from_the_property_tab_get_independent_uuid_v7_ids() {
        use crate::window_session::test_fixtures::{sample_document, saved_project_with};
        use stellar_document::records::NodePayload;
        use stellar_document::shape::{RectangleParams, ShapeGeometry};
        let mut model = sample_document();
        let mut record = model.record(sid(12)).unwrap();
        record.id = sid(14);
        record.parent = Some(sid(11));
        record.payload = NodePayload::Shape(ShapeGeometry::Rectangle(RectangleParams {
            size: stellar_core_types::value::Size2::new(40., 30.).unwrap(),
            corner_radius: stellar_document::frame::CornerRadius::ZERO,
        }));
        model.insert_node(record).unwrap();
        let project = saved_project_with("inspector-new-ids", &[&model]);
        let run = || {
            let mut session =
                ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
            session.select(Selection::new([sid(14)], None)).unwrap();
            for (request, property, text) in [
                (940, InspectorProperty::Fill, "#ff0000"),
                (941, InspectorProperty::Left, "5"),
            ] {
                let t = target(&session);
                let input = InspectorInput::Set(parse_input(property, text).unwrap());
                edit_property(
                    &mut session,
                    RequestId(request),
                    t,
                    property,
                    &input,
                    PropertyEditPhase::Commit,
                )
                .ok()
                .unwrap();
            }
            let model = session.document().unwrap().model();
            let ids: Vec<StableId> = model
                .modifier_stack(sid(14))
                .unwrap()
                .entries()
                .iter()
                .map(|entry| entry.id.stable_id())
                .collect();
            assert_eq!(ids.len(), 2);
            assert!(ids.iter().all(|id| (id.version(), id.variant()) == (7, 2)));
            assert_ne!(ids[0], ids[1]);
            ids
        };
        let first = run();
        let second = run();
        assert!(first.iter().all(|id| !second.contains(id)));
    }

    #[test]
    fn repeats_merge_into_one_entry_without_blocking_undo_or_selection() {
        let project = saved_project("inspector-repeat");
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let start: f32 = width(&session).trim_end_matches("px").parse().unwrap();
        let depth = session.history().history().len();
        for index in 0..3 {
            // 반복 입력은 preview 없이 바로 기록되므로 다음 요청은 새 snapshot(revision) 기준입니다.
            let t = target(&session);
            edit_property(
                &mut session,
                RequestId(930 + index),
                t,
                InspectorProperty::Width,
                &InspectorInput::Adjust(10.),
                PropertyEditPhase::Repeat,
            )
            .ok()
            .unwrap();
            assert!(!session.history().history().preview_active());
        }
        assert_eq!(width(&session), format!("{}px", start + 30.0));
        assert_eq!(session.history().history().len(), depth + 1);
        // preview가 없으니 곧바로 Undo와 선택이 됩니다. Undo 한 번이 반복 전체를 되돌립니다.
        assert!(session.undo().unwrap());
        assert_eq!(width(&session), format!("{start}px"));
        session.select(Selection::new([sid(12)], None)).unwrap();
    }

    #[test]
    fn cancel_and_rejected_end_close_a_dangling_preview() {
        let project = saved_project("inspector-dangling");
        let mut session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        session.select(Selection::new([sid(11)], None)).unwrap();
        let start = width(&session);
        let t = target(&session);
        let scrub = |session: &mut ProjectWindowSession, request, target, phase| {
            edit_property(
                session,
                RequestId(request),
                target,
                InspectorProperty::Width,
                &InspectorInput::Adjust(7.),
                phase,
            )
        };
        scrub(&mut session, 940, t, PropertyEditPhase::Continuous)
            .ok()
            .unwrap();
        assert!(session.history().history().preview_active());
        // 셸이 다른 View·오래된 위치로 취소를 보내도 preview를 닫습니다.
        let wrong = SelectionEditTarget {
            history_sequence: t.history_sequence + 99,
            ..t
        };
        scrub(&mut session, 941, wrong, PropertyEditPhase::Cancel)
            .ok()
            .unwrap();
        assert!(!session.history().history().preview_active());
        assert_eq!(width(&session), start);
        // 거부된 끝 요청도 preview를 남기지 않습니다(취소가 revision을 올렸으므로 새 기준).
        let t = target(&session);
        let wrong = SelectionEditTarget {
            history_sequence: t.history_sequence + 99,
            ..t
        };
        scrub(&mut session, 942, t, PropertyEditPhase::Continuous)
            .ok()
            .unwrap();
        assert!(scrub(&mut session, 943, wrong, PropertyEditPhase::Ended).is_err());
        assert!(!session.history().history().preview_active());
        assert_eq!(width(&session), start);
    }

    #[test]
    fn empty_selection_snapshot_names_the_current_page_and_view() {
        let project = saved_project("inspector-page");
        let session = ProjectWindowSession::load(identity(project.path()), window(1)).unwrap();
        let mut cache = InspectorCache::default();
        let snapshot = snapshot(&mut cache, &session);
        assert_eq!(snapshot.count, 0);
        assert!(snapshot.rows.is_empty());
        assert!(snapshot.display_name.contains(" · "), "{snapshot:?}");
    }
}
