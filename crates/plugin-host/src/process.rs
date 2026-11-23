//! 외부 process 플러그인 IPC와 수명 (XXI.5.1·XXI.5.2, ADR-0097 결정 11).
//!
//! 장기 실행·native 플러그인은 앱 process에 적재(dlopen)하지 않고 별도 process로 띄웁니다. 실행 파일은 호출자가
//! 검증한 package 안 파일([`crate::package::PluginPackage::resolve`])로 정합니다.
//!
//! ## IPC 계약
//!
//! - 전송: child의 stdin·stdout, UTF-8 줄 하나가 메시지 하나입니다(JSON writer가 개행을 escape하므로 줄바꿈 없음).
//!   한 줄은 [`ProcessConfig::max_line_bytes`] 이하입니다. stderr는 Host가 읽지 않고 버립니다.
//! - 플러그인 → Host 요청: [`crate::session`]의 RPC envelope 그대로입니다. 같은 [`HostSession`](권한·상한)을 지납니다.
//!   Host는 응답(조각 포함)을 같은 줄 형식으로 stdin에 씁니다.
//! - 수명: Host가 `{"stellar_process":1,"type":"heartbeat","seq":n}`을 보내면 플러그인은
//!   `{"stellar_process":1,"type":"heartbeat-ack","seq":n}`으로 답합니다.
//! - 다음이면 Host가 process를 끝내고(kill) session을 닫습니다: heartbeat 응답이 [`ProcessConfig::heartbeat_timeout`] 안에
//!   없음, JSON이 아니거나 너무 긴 줄(protocol 위반), 권한 철회([`ProcessPlugin::revoke`]), Host 쪽 종료(drop).
//!   process가 스스로 끝나거나 죽어도 Host process에는 영향이 없고 session만 닫힙니다. Host가 끝낸 경우에는 queue에
//!   남은 요청을 실행하지 않습니다([`HostSession::abort`]).
//!
//! 수명 관리는 [`ProcessPlugin::pump`]를 부르는 thread 하나에서 합니다(앱은 plugin supervisor thread에서 부릅니다).

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::json::{parse, write, JsonLimits, JsonNumber, JsonValue as J};
use crate::session::HostSession;

/// 수명 메시지 version.
pub const STELLAR_PROCESS_VERSION: i64 = 1;

/// 수명 설정.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessConfig {
    /// heartbeat 간격.
    pub heartbeat_interval: Duration,
    /// heartbeat 응답 제한 시간.
    pub heartbeat_timeout: Duration,
    /// 줄 최대 byte 수.
    pub max_line_bytes: usize,
}

impl Default for ProcessConfig {
    fn default() -> Self {
        ProcessConfig {
            heartbeat_interval: Duration::from_secs(5),
            heartbeat_timeout: Duration::from_secs(10),
            max_line_bytes: crate::rpc::MAX_MESSAGE_BYTES,
        }
    }
}

/// 강제 종료 이유.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillReason {
    /// heartbeat 응답 없음(멈춤).
    HeartbeatTimeout,
    /// protocol 위반(JSON 아님, 너무 긴 줄, 모르는 수명 메시지).
    ProtocolViolation,
    /// 권한 철회.
    Revoked,
    /// Host 쪽 종료.
    Closed,
}

/// process 상태.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// 실행 중.
    Running,
    /// 스스로 끝남(종료 code, signal이면 `None`).
    Exited(Option<i32>),
    /// Host가 끝냄.
    Killed(KillReason),
}

enum Line {
    Text(String),
    TooLong,
    Eof,
}

/// 한 외부 process 플러그인.
pub struct ProcessPlugin {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Line>,
    session: Option<HostSession>,
    config: ProcessConfig,
    state: ProcessState,
    next_seq: u64,
    pending: Option<(u64, Instant)>,
    last_heartbeat: Instant,
}

impl std::fmt::Debug for ProcessPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessPlugin")
            .field("pid", &self.child.id())
            .field("state", &self.state)
            .finish()
    }
}

fn read_lines(stdout: impl std::io::Read, max: usize, out: std::sync::mpsc::Sender<Line>) {
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    loop {
        let buf = match reader.fill_buf() {
            Ok([]) | Err(_) => {
                let _ = out.send(Line::Eof);
                return;
            }
            Ok(buf) => buf,
        };
        let (taken, done) = match buf.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (buf.len(), false),
        };
        line.extend_from_slice(&buf[..if done { taken - 1 } else { taken }]);
        reader.consume(taken);
        if line.len() > max {
            let _ = out.send(Line::TooLong);
            return;
        }
        if done {
            let text = String::from_utf8(std::mem::take(&mut line)).unwrap_or_default();
            if out.send(Line::Text(text)).is_err() {
                return;
            }
        }
    }
}

impl ProcessPlugin {
    /// `command`를 별도 process로 띄웁니다. stdin·stdout은 IPC, stderr는 버립니다.
    pub fn launch(
        mut command: Command,
        session: HostSession,
        config: ProcessConfig,
    ) -> std::io::Result<ProcessPlugin> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stdin = child.stdin.take();
        let (tx, lines) = channel();
        let max = config.max_line_bytes;
        std::thread::Builder::new()
            .name("stellar-plugin-process-reader".to_owned())
            .spawn(move || read_lines(stdout, max, tx))?;
        Ok(ProcessPlugin {
            child,
            stdin,
            lines,
            session: Some(session),
            config,
            state: ProcessState::Running,
            next_seq: 1,
            pending: None,
            last_heartbeat: Instant::now(),
        })
    }

    /// 현재 상태.
    pub fn state(&self) -> ProcessState {
        self.state
    }

    /// process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    fn send_line(&mut self, line: &str) -> bool {
        match self.stdin.as_mut() {
            Some(stdin) => writeln!(stdin, "{line}")
                .and_then(|()| stdin.flush())
                .is_ok(),
            None => false,
        }
    }

    fn finish(&mut self, state: ProcessState) -> ProcessState {
        if matches!(state, ProcessState::Killed(_)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        self.stdin = None;
        if let Some(mut session) = self.session.take() {
            // Host가 끝낸 process(멈춤·위반·철회·drop)의 queue에 남은 요청은 실행하지 않습니다. 철회한 권한으로 쓰기가
            // 뒤늦게 적용되지 않게 합니다. 스스로 끝난 process는 이미 받은 요청을 끝까지 처리합니다.
            if matches!(state, ProcessState::Killed(_)) {
                session.abort();
            } else {
                session.close();
            }
        }
        self.state = state;
        state
    }

    fn exited(&mut self) -> ProcessState {
        let code = self.child.wait().ok().and_then(|s| s.code());
        self.finish(ProcessState::Exited(code))
    }

    fn handle_line(&mut self, text: &str) -> Result<(), KillReason> {
        let value = parse(
            text,
            JsonLimits {
                max_bytes: self.config.max_line_bytes,
                ..JsonLimits::default()
            },
        )
        .map_err(|_| KillReason::ProtocolViolation)?;
        if let Some(version) = value.get("stellar_process") {
            if version.as_i64() != Some(STELLAR_PROCESS_VERSION)
                || value.get("type").and_then(J::as_str) != Some("heartbeat-ack")
            {
                return Err(KillReason::ProtocolViolation);
            }
            let seq = value.get("seq").and_then(J::as_i64);
            if let Some((pending, _)) = self.pending {
                if seq == i64::try_from(pending).ok() {
                    self.pending = None;
                }
            }
            return Ok(());
        }
        if let Some(session) = &self.session {
            session.submit(text);
        }
        Ok(())
    }

    /// 최대 `wait` 동안 메시지를 주고받고 수명을 확인합니다. 끝난 상태면 바로 돌아옵니다.
    pub fn pump(&mut self, wait: Duration) -> ProcessState {
        if self.state != ProcessState::Running {
            return self.state;
        }
        let deadline = Instant::now() + wait;
        loop {
            // Host 응답을 child에 넘깁니다.
            while let Some(reply) = self.session.as_ref().and_then(HostSession::try_recv) {
                if !self.send_line(&reply) {
                    return self.exited();
                }
            }
            if let Ok(Some(_)) = self.child.try_wait() {
                return self.exited();
            }
            let now = Instant::now();
            if let Some((_, sent)) = self.pending {
                if now.duration_since(sent) >= self.config.heartbeat_timeout {
                    return self.finish(ProcessState::Killed(KillReason::HeartbeatTimeout));
                }
            } else if now.duration_since(self.last_heartbeat) >= self.config.heartbeat_interval {
                let seq = self.next_seq;
                self.next_seq += 1;
                let message = write(&J::object(vec![
                    ("stellar_process", J::int(STELLAR_PROCESS_VERSION)),
                    ("type", J::string("heartbeat")),
                    ("seq", J::Number(JsonNumber::from_u64(seq))),
                ]));
                self.pending = Some((seq, now));
                self.last_heartbeat = now;
                if !self.send_line(&message) {
                    return self.exited();
                }
            }
            if now >= deadline {
                return self.state;
            }
            let slice = (deadline - now).min(Duration::from_millis(10));
            match self.lines.recv_timeout(slice) {
                Ok(Line::Text(text)) => {
                    if let Err(reason) = self.handle_line(&text) {
                        return self.finish(ProcessState::Killed(reason));
                    }
                }
                Ok(Line::TooLong) => {
                    return self.finish(ProcessState::Killed(KillReason::ProtocolViolation))
                }
                Ok(Line::Eof) | Err(RecvTimeoutError::Disconnected) => {
                    // stdout이 닫혔으면 끝날 때까지 잠깐 기다립니다.
                    std::thread::sleep(Duration::from_millis(5));
                    if let Ok(Some(_)) = self.child.try_wait() {
                        return self.exited();
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// 권한 철회: process를 끝내고 session을 닫습니다.
    pub fn revoke(&mut self) -> ProcessState {
        if self.state != ProcessState::Running {
            return self.state;
        }
        self.finish(ProcessState::Killed(KillReason::Revoked))
    }
}

impl Drop for ProcessPlugin {
    fn drop(&mut self) {
        if self.state == ProcessState::Running {
            self.finish(ProcessState::Killed(KillReason::Closed));
        }
    }
}
