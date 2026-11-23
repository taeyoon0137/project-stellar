//! attached backend의 앱 연결: 편집 tool 호출을 bridge→app frame으로 보내고 응답 frame을 받습니다 (XIX.9.4 일부,
//! XXIX.3.4, ADR-0124 결정 3, ADR-0132 결정 3).
//!
//! handshake(XIX.1.3)를 마친 앱 연결에서 [`AttachedLink`]가 요청 frame(`stellar_local_ipc::frame`)을 쓰고 같은 요청 id의
//! `Reply` frame 하나를 기다립니다. 연결 하나에 처리 중 요청은 하나입니다(ADR-0085 `MAX_IN_FLIGHT` 1).
//!
//! - **값 변환**: tool 인자 해석은 기존 함수 하나씩입니다. `document.begin_agent_edit`은 [`parse_begin_agent_edit`],
//!   `document.propose_commands`는 [`parse_transaction`](`stellar apply`와 같은 `command_input`)입니다. command는
//!   `decode_commands`로 닫힌 enum을 만든 뒤 commands codec의 canonical bytes(`encode_command`)와 종류 이름으로 싣고,
//!   앱은 같은 codec으로 다시 읽습니다. 기준 revision은 `ProjectRevision`의 표기, 전역 resource는 `ExternalKind::name`입니다.
//! - **앱이 정하는 것**: lease id·transaction id·완료 항목 id와 lease·승인·첫 연결 승인 판정은 앱이 합니다. bridge는 결과
//!   종류 이름과 고정 문장만 받습니다(문서 내용 없음).
//! - **fail closed**: 응답 id가 다르거나, 응답이 아닌 frame이거나, 모르는 결과 종류 이름이면 연결을 더 쓰지 않습니다
//!   ([`LinkError`]). frame 상한(1 MiB)을 넘는 제안은 보내지 않고 `too-large`입니다.
//!
//! MCP 서버 루프(`server`)와 실행 진입점(`entry`)이 attached 모드에서 이 link로 `tools/call`을 보내는 연결은 후속입니다
//! (ADR-0132 Consequences). 지금 attached 세션의 앱 연결 감시(`attach::watch_app_connection`)는 frame을 쓰지 않으므로
//! 그대로입니다.

use std::fmt;
use std::io::{Read, Write};

use stellar_local_ipc::frame::{read_frame, write_frame, Frame, FrameError};
use stellar_ops::apply::{decode_commands, ApplyRequest, MAX_COMMANDS};
use stellar_ops::commands::codec::encode_command;

use crate::failure::{fail_text, ResultKind};
use crate::json::JsonValue;
use crate::jsonrpc::RpcError;
use crate::lease::{parse_begin_agent_edit, BeginAgentEdit, BEGIN_AGENT_EDIT};
use crate::proposal::parse_transaction;
use crate::result::tool_success;

/// `document.propose_commands` tool 이름(attached catalog).
pub const PROPOSE_COMMANDS: &str = stellar_agent_protocol::catalog::tools::PROPOSE_COMMANDS.name;

/// 앱 응답.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkReply {
    /// 결과 종류(ADR-0085).
    pub kind: ResultKind,
    /// 앱이 만든 고정 문장(`lease-requested`, `connection-not-approved` 등).
    pub message: String,
}

/// 앱 연결 실패. 이 뒤로는 연결을 쓰지 않습니다(fail closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    /// frame 쓰기·읽기 실패(연결 끝 포함).
    Frame(FrameError),
    /// 응답 id가 요청 id와 다릅니다.
    Mismatch {
        /// 보낸 요청 id.
        expected: u64,
        /// 받은 응답 id.
        found: u64,
    },
    /// 응답이 아닌 frame을 받았습니다.
    NotReply,
    /// 모르는 결과 종류 이름.
    UnknownResult(String),
    /// 연결이 이미 실패했습니다.
    Broken,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::Frame(e) => write!(f, "앱 연결 frame 오류: {e}"),
            LinkError::Mismatch { expected, found } => {
                write!(f, "요청 {expected}의 응답 대신 {found}를 받았습니다")
            }
            LinkError::NotReply => f.write_str("앱이 응답이 아닌 frame을 보냈습니다"),
            LinkError::UnknownResult(_) => f.write_str("앱이 모르는 결과 종류를 보냈습니다"),
            LinkError::Broken => f.write_str("앱 연결이 이미 실패했습니다"),
        }
    }
}

impl std::error::Error for LinkError {}

/// `document.begin_agent_edit` 요청 frame.
pub fn begin_frame(request: u64, begin: &BeginAgentEdit) -> Frame {
    Frame::BeginEdit {
        request,
        document: begin.document,
        targets: begin.targets.clone(),
        externals: begin
            .externals
            .iter()
            .map(|resource| (resource.kind.name().to_string(), resource.id))
            .collect(),
        description: begin.description.clone(),
    }
}

/// `document.propose_commands` 요청 frame. command를 닫힌 enum으로 바꿀 수 없으면 `invalid-proposal` 실패입니다.
pub fn propose_frame(request: u64, apply: &ApplyRequest) -> Result<Frame, RpcError> {
    let commands = decode_commands(&apply.commands)
        .map_err(|e| fail_text(ResultKind::InvalidArguments, e.to_string()))?;
    Ok(Frame::Propose {
        request,
        base_revision: apply.base_revision.to_string(),
        document: apply.document,
        commands: commands
            .iter()
            .map(|command| (command.kind().name().to_string(), encode_command(command)))
            .collect(),
    })
}

/// 앱 연결 하나(handshake 뒤 stream).
#[derive(Debug)]
pub struct AttachedLink<S> {
    stream: S,
    next: u64,
    broken: bool,
}

impl<S: Read + Write> AttachedLink<S> {
    /// handshake를 마친 stream으로 link를 만듭니다. 요청 id는 1부터 셉니다.
    pub fn new(stream: S) -> AttachedLink<S> {
        AttachedLink {
            stream,
            next: 1,
            broken: false,
        }
    }

    /// 요청 frame 하나를 보내고 응답을 기다립니다. `frame`의 요청 id는 이 함수가 정합니다.
    fn call(&mut self, make: impl FnOnce(u64) -> Frame) -> Result<LinkReply, LinkError> {
        if self.broken {
            return Err(LinkError::Broken);
        }
        let request = self.next;
        self.next += 1;
        let outcome = self.exchange(make(request), request);
        if outcome.is_err() {
            self.broken = true;
        }
        outcome
    }

    fn exchange(&mut self, frame: Frame, request: u64) -> Result<LinkReply, LinkError> {
        write_frame(&mut self.stream, &frame).map_err(LinkError::Frame)?;
        match read_frame(&mut self.stream).map_err(LinkError::Frame)? {
            Frame::Reply {
                request: found,
                result,
                message,
            } => {
                if found != request {
                    return Err(LinkError::Mismatch {
                        expected: request,
                        found,
                    });
                }
                let kind =
                    ResultKind::from_name(&result).ok_or(LinkError::UnknownResult(result))?;
                Ok(LinkReply { kind, message })
            }
            _ => Err(LinkError::NotReply),
        }
    }

    /// 대상 선언·lease 요청.
    pub fn begin_edit(&mut self, begin: &BeginAgentEdit) -> Result<LinkReply, LinkError> {
        self.call(|request| begin_frame(request, begin))
    }

    /// command 제안(사용자 승인 대기). 바깥 `Err`는 보내기 전 입력 실패(닫힌 enum으로 바꿀 수 없음, frame 상한 초과)이고
    /// 연결은 그대로 씁니다. 안쪽 `Err`는 연결 실패입니다.
    pub fn propose(
        &mut self,
        apply: &ApplyRequest,
    ) -> Result<Result<LinkReply, LinkError>, RpcError> {
        let frame = propose_frame(0, apply)?;
        if let Err(error) = stellar_local_ipc::frame::encode(&frame) {
            return Err(fail_text(frame_kind(&error), error.to_string()));
        }
        Ok(self.call(|id| with_request(frame, id)))
    }

    /// 정상 완료(CompleteAgentEdit).
    pub fn complete(&mut self) -> Result<LinkReply, LinkError> {
        self.call(|request| Frame::Complete { request })
    }

    /// 연결이 실패해 더 쓰지 않는지.
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    /// attached 편집 tool 호출 하나를 앱에 보내고 MCP tool 결과를 만듭니다. 앱이 성공을 돌려주면
    /// `{"result": <종류 이름>, "message": <고정 문장>}` 성공 결과, 거부하면 그 결과 종류의 실패입니다. 이 link가
    /// 다루지 않는 tool이면 `None`입니다.
    pub fn call_tool(
        &mut self,
        name: &str,
        args: &JsonValue,
    ) -> Option<Result<JsonValue, RpcError>> {
        let outcome = if name == BEGIN_AGENT_EDIT.name {
            parse_begin_agent_edit(args)
                .map_err(|e| fail_text(e.kind, e.message))
                .and_then(|begin| self.begin_edit(&begin).map_err(link_failure))
        } else if name == PROPOSE_COMMANDS {
            parse_transaction(args, MAX_COMMANDS)
                .map_err(|e| fail_text(ResultKind::InvalidArguments, e.message))
                .and_then(|apply| self.propose(&apply)?.map_err(link_failure))
        } else {
            return None;
        };
        Some(outcome.and_then(reply_result))
    }
}

fn with_request(frame: Frame, id: u64) -> Frame {
    match frame {
        Frame::Propose {
            base_revision,
            document,
            commands,
            ..
        } => Frame::Propose {
            request: id,
            base_revision,
            document,
            commands,
        },
        other => other,
    }
}

fn frame_kind(error: &FrameError) -> ResultKind {
    match error {
        FrameError::TooLarge(_) => ResultKind::TooLarge,
        _ => ResultKind::InvalidArguments,
    }
}

fn link_failure(error: LinkError) -> RpcError {
    fail_text(ResultKind::Unavailable, error.to_string())
}

fn reply_result(reply: LinkReply) -> Result<JsonValue, RpcError> {
    if reply.kind == ResultKind::Ok {
        Ok(tool_success(JsonValue::object(vec![
            ("result", JsonValue::string(reply.kind.name())),
            ("message", JsonValue::string(reply.message)),
        ])))
    } else {
        Err(fail_text(reply.kind, reply.message))
    }
}

#[cfg(test)]
#[path = "attached_tests.rs"]
mod tests;
