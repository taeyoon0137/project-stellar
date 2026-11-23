//! attached link test (ADR-0132 결정 3).

use std::os::unix::net::UnixStream;
use std::thread::JoinHandle;

use stellar_local_ipc::frame::{read_frame, write_frame, Frame};
use stellar_ops::commands::codec::decode_command;
use stellar_ops::commands::command::Command;
use stellar_ops::core_types::ids::StableId;

use super::*;
use crate::json::{parse, JsonLimits};

const DOC: &str = "01890000-0000-7000-8000-000000000001";
const NODE: &str = "01890000-0000-7000-8000-000000000011";

fn args(text: &str) -> JsonValue {
    parse(text, JsonLimits::default()).unwrap()
}

/// 받은 frame을 기록하고, 정해 둔 응답(결과 종류, 문장, 응답 id 보정)을 쓰는 가짜 앱.
fn fake_app(
    replies: Vec<(&'static str, &'static str, u64)>,
) -> (UnixStream, JoinHandle<Vec<Frame>>) {
    let (bridge, mut app) = UnixStream::pair().unwrap();
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (result, message, shift) in replies {
            let Ok(frame) = read_frame(&mut app) else {
                break;
            };
            let request = frame.request() + shift;
            seen.push(frame);
            write_frame(
                &mut app,
                &Frame::Reply {
                    request,
                    result: result.into(),
                    message: message.into(),
                },
            )
            .unwrap();
        }
        seen
    });
    (bridge, handle)
}

fn begin_args(targets: &str) -> JsonValue {
    args(&format!(
        r#"{{"document":"{DOC}","targets":{targets},"description":"작업"}}"#
    ))
}

#[test]
fn tool_calls_become_frames_and_app_replies_become_tool_results() {
    let (stream, app) = fake_app(vec![
        ("ok", "lease-requested", 0),
        ("ok", "queued", 0),
        ("not-trusted", "connection-not-approved", 0),
        ("ok", "completed", 0),
    ]);
    let mut link = AttachedLink::new(stream);
    let begin = args(&format!(
        r#"{{"document":"{DOC}","targets":["{NODE}"],"description":"작업","externals":[{{"kind":"asset","id":"{NODE}"}}]}}"#
    ));
    let ok = link
        .call_tool(BEGIN_AGENT_EDIT.name, &begin)
        .unwrap()
        .unwrap();
    let data = ok
        .get("structuredContent")
        .and_then(|c| c.get("data"))
        .unwrap();
    assert_eq!(
        data.get("message").and_then(JsonValue::as_str),
        Some("lease-requested")
    );
    let propose = args(&format!(
        r#"{{"base_revision":"g0","document":"{DOC}","commands":[{{"kind":"rename","node":"{NODE}","name":"A"}}]}}"#
    ));
    assert!(link.call_tool(PROPOSE_COMMANDS, &propose).unwrap().is_ok());
    // 앱의 거부는 같은 결과 종류의 tool 실패입니다.
    let refused = link
        .call_tool(BEGIN_AGENT_EDIT.name, &begin)
        .unwrap()
        .unwrap_err();
    assert_eq!(
        crate::failure::kind_of(&refused),
        Some(ResultKind::NotTrusted)
    );
    assert_eq!(link.complete().unwrap().message, "completed");
    assert!(link.call_tool("document.semantic_diff", &begin).is_none());
    drop(link);
    let frames = app.join().unwrap();
    assert_eq!(frames.len(), 4);
    let node: StableId = NODE.parse().unwrap();
    assert_eq!(
        frames[0],
        Frame::BeginEdit {
            request: 1,
            document: DOC.parse().unwrap(),
            targets: vec![node],
            externals: vec![("asset".to_string(), node)],
            description: "작업".to_string(),
        }
    );
    // command는 `stellar apply`와 같은 해석 뒤 commands codec의 canonical bytes로 실립니다.
    match &frames[1] {
        Frame::Propose {
            request,
            base_revision,
            commands,
            ..
        } => {
            assert_eq!(*request, 2);
            assert_eq!(base_revision, "g0");
            assert_eq!(commands.len(), 1);
            let decoded = decode_command(&commands[0].1).unwrap();
            assert_eq!(decoded, Command::rename_node(node, "A".to_string()));
            assert_eq!(commands[0].0, decoded.kind().name());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(frames[3], Frame::Complete { request: 4 });
}

#[test]
fn mismatched_or_unknown_replies_break_the_link() {
    let (stream, _app) = fake_app(vec![("ok", "x", 1)]);
    let mut link = AttachedLink::new(stream);
    assert_eq!(
        link.complete(),
        Err(LinkError::Mismatch {
            expected: 1,
            found: 2
        })
    );
    assert!(link.is_broken());
    assert_eq!(link.complete(), Err(LinkError::Broken));

    let (stream, _app) = fake_app(vec![("maybe", "x", 0)]);
    let mut link = AttachedLink::new(stream);
    assert_eq!(
        link.complete(),
        Err(LinkError::UnknownResult("maybe".into()))
    );

    // 앱이 닫힌 연결은 frame 오류이고 tool 결과는 unavailable입니다.
    let (stream, _app) = fake_app(Vec::new());
    let mut link = AttachedLink::new(stream);
    let error = link
        .call_tool(
            BEGIN_AGENT_EDIT.name,
            &begin_args(&format!(r#"["{NODE}"]"#)),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(
        crate::failure::kind_of(&error),
        Some(ResultKind::Unavailable)
    );
}

#[test]
fn invalid_input_is_refused_before_sending_and_keeps_the_link() {
    let (stream, app) = fake_app(vec![("ok", "completed", 0)]);
    let mut link = AttachedLink::new(stream);
    let error = link
        .call_tool(BEGIN_AGENT_EDIT.name, &begin_args("[]"))
        .unwrap()
        .unwrap_err();
    assert_eq!(
        crate::failure::kind_of(&error),
        Some(ResultKind::InvalidArguments)
    );
    let unknown = args(&format!(
        r#"{{"base_revision":"g0","document":"{DOC}","commands":[{{"kind":"raw-patch","canonical":"0000"}}]}}"#
    ));
    let error = link
        .call_tool(PROPOSE_COMMANDS, &unknown)
        .unwrap()
        .unwrap_err();
    assert_eq!(
        crate::failure::kind_of(&error),
        Some(ResultKind::InvalidArguments)
    );
    assert!(!link.is_broken());
    // 거부한 입력은 앱에 보내지 않았으므로 첫 frame의 요청 id가 1입니다.
    assert_eq!(link.complete().unwrap().message, "completed");
    drop(link);
    assert_eq!(app.join().unwrap(), vec![Frame::Complete { request: 1 }]);
}
