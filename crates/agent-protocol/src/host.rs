//! attached host trait (ADR-0005 §8, ADR-0085, ADR-0124 결정 2).
//!
//! 앱에 붙은 bridge 연결(외부 MCP 클라이언트·앱이 시작한 내장 세션)의 attached `tools/list`·`tools/call` 판정과 요청 접수를
//! 앱 runtime이 이 trait로 구현합니다. 이 crate는 문서·command 타입에 닿지 않으므로 요청과 접수 결과는 구현이 정하는 연관
//! 타입이고, 거부는 ADR-0085 결과 종류([`ResultKind`])로 냅니다.
//!
//! 이 trait의 판정은 연결 I/O 스레드의 앞 단계 거부입니다(모드 상한, 노출하지 않는 tool). 첫 연결 승인·권한 확인 같은
//! 상태 판정은 앱 문서 스레드의 권한 저장소가 합니다(XIX.3).

use crate::catalog::tools::ToolDef;
use crate::outcome::ResultKind;
use crate::permission::Permission;

/// attached 호출 거부.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostRefusal {
    /// 결과 종류.
    pub kind: ResultKind,
    /// 고정 코드.
    pub code: &'static str,
}

impl HostRefusal {
    /// 거부 하나.
    pub const fn new(kind: ResultKind, code: &'static str) -> HostRefusal {
        HostRefusal { kind, code }
    }

    /// 이 연결에 노출하지 않는 tool.
    pub const UNKNOWN_TOOL: HostRefusal = HostRefusal::new(ResultKind::UnknownTool, "unknown-tool");
}

/// 앱 runtime이 구현하는 attached host.
pub trait AttachedHost {
    /// bridge 연결 handle.
    type Connection: Copy;
    /// 앞단이 해석한 요청.
    type Request;
    /// 접수 결과(처리 결과를 받을 handle 등).
    type Accepted;

    /// 이 연결의 attached `tools/list`(카탈로그 순서).
    fn listed_tools(&self, connection: Self::Connection) -> Vec<&'static ToolDef>;

    /// 이 연결이 `permission`이 필요한 호출을 할 수 있는지(모드 상한·내장 세션 제외 판정).
    fn permits(
        &self,
        connection: Self::Connection,
        permission: Permission,
    ) -> Result<(), HostRefusal>;

    /// 검증한 요청을 접수합니다. 기다리지 않습니다.
    fn submit(
        &self,
        connection: Self::Connection,
        request: Self::Request,
    ) -> Result<Self::Accepted, HostRefusal>;

    /// `tools/call` 판정: 이 연결 목록에 있는 tool이고 그 권한이 상한 안이면 정의를 돌려줍니다.
    fn callable_tool(
        &self,
        connection: Self::Connection,
        name: &str,
    ) -> Result<&'static ToolDef, HostRefusal> {
        let tool = self
            .listed_tools(connection)
            .into_iter()
            .find(|tool| tool.name == name)
            .ok_or(HostRefusal::UNKNOWN_TOOL)?;
        self.permits(connection, tool.grant)?;
        Ok(tool)
    }
}
