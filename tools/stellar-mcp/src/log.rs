//! stderr 전용 로그 (XIX.1.1).
//!
//! MCP stdio transport에서 stdout은 protocol 메시지 전용이므로 모든 로그는 이 logger로 stderr에
//! 씁니다. 한 로그는 한 줄이며, 메시지 안의 제어 문자(개행 포함)는 이스케이프해 줄 위조를
//! 막습니다. 로그에는 메시지 본문(프로젝트 내용 가능)을 넣지 않고 method 이름·오류 코드 같은
//! protocol 수준 정보만 넣습니다.

use std::io::Write;

/// 로그 수준.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 정보.
    Info,
    /// 경고.
    Warn,
    /// 오류.
    Error,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// stderr(또는 테스트 버퍼)로 쓰는 logger.
pub struct Logger<W: Write> {
    sink: W,
}

impl<W: Write> Logger<W> {
    /// 대상에 쓰는 logger를 만듭니다.
    pub fn new(sink: W) -> Logger<W> {
        Logger { sink }
    }

    /// 대상을 돌려받습니다.
    pub fn into_inner(self) -> W {
        self.sink
    }

    /// 한 줄을 씁니다. 기록 실패는 무시합니다(로그 실패로 protocol을 중단하지 않음).
    pub fn log(&mut self, level: Level, message: &str) {
        let line = format!("stellar-mcp: {}: {}\n", level.name(), sanitize(message));
        let _ = self.sink.write_all(line.as_bytes());
        let _ = self.sink.flush();
    }

    /// 정보 로그.
    pub fn info(&mut self, message: &str) {
        self.log(Level::Info, message);
    }

    /// 경고 로그.
    pub fn warn(&mut self, message: &str) {
        self.log(Level::Warn, message);
    }

    /// 오류 로그.
    pub fn error(&mut self, message: &str) {
        self.log(Level::Error, message);
    }
}

/// 제어 문자를 이스케이프합니다.
fn sanitize(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    for c in message.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_log_is_one_sanitized_line() {
        let mut logger = Logger::new(Vec::new());
        logger.info("method: a\nstellar-mcp: error: 위조");
        logger.warn("탭\t끝");
        let text = String::from_utf8(logger.into_inner()).unwrap();
        assert_eq!(
            text,
            "stellar-mcp: info: method: a\\nstellar-mcp: error: 위조\nstellar-mcp: warn: 탭\\t끝\n"
        );
    }
}
