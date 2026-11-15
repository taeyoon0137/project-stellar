import Foundation

// 내장 에이전트 패널이 코어와 주고받는 값 타입과 프로토콜입니다.
//
// 경계 원칙(아키텍처 §8.2, ADR-0009):
// - 셸은 코어가 만든 값 스냅샷과 이벤트를 받고(pull), 사용자의 의도를 명령으로 보냅니다.
// - 이 파일의 타입은 Rust 타입을 노출하지 않습니다. 식별자는 불투명 정수, 문자열은 표시용 값입니다.
// - 코어→셸 콜백은 두지 않습니다. 셸이 `pollEvent()`로 꺼냅니다(ADR-0009 "callback 등록 API 없음").
// - 비용·사용량 필드는 두지 않습니다(ADR-0005 결정 4.1).

// MARK: - 식별자

/// 대화 기록 항목 식별자. 코어가 발급하는 불투명 정수입니다.
struct AgentItemID: Hashable {
    let rawValue: UInt64
}

/// 승인 요청 식별자. 첫 응답 하나만 유효합니다(ADR-0005 결정 4.5).
struct AgentApprovalID: Hashable {
    let rawValue: UInt64
}

/// 에이전트가 적용한 변경 하나의 식별자. 개별 되돌리기 명령의 대상입니다.
struct AgentChangeID: Hashable {
    let rawValue: UInt64
}

/// 설치된 CLI(launch profile) 하나의 식별자.
struct AgentCLIID: Hashable {
    let rawValue: UInt64
}

// MARK: - 헤더(CLI·세션·연결)

/// 설치된 CLI의 실행 가능 상태(ADR-0005 결정 4.1의 세 값).
enum AgentCLIAvailability: Equatable {
    /// 실행 가능.
    case ready
    /// 검증되지 않은 버전이라 실행하지 않음.
    case unsupportedVersion
    /// CLI가 인증 필요를 보고함. Stellar는 인증 흐름에 참여하지 않습니다.
    case authenticationRequired
}

/// 설치된 CLI 하나. 표시 이름과 버전은 launch profile·CLI 보고 값입니다.
struct AgentCLIOption: Equatable {
    let id: AgentCLIID
    var displayName: String
    var version: String?
    var availability: AgentCLIAvailability
}

/// 앱이 시작한 세션의 연결 상태.
enum AgentConnectionState: Equatable {
    case noSession
    case starting
    case connected
    /// 재연결 유예 중. 유예를 넘기면 `ended`가 됩니다.
    case reconnecting
    case ended
}

/// 에이전트 모드. Stellar 권한의 상한이며 사용자 UI에서만 바꿉니다(ADR-0005 결정 4.5).
enum AgentPermissionMode: CaseIterable, Equatable {
    case readOnly
    case reviewBeforeApplying
    case applyWithinApprovedScope
}

/// 패널 헤더에 필요한 세션 요약.
struct AgentSessionHeader: Equatable {
    /// 시작 확인 화면에만 쓰는 프로젝트 표시 이름. 알림·진단에는 넣지 않습니다(아키텍처 §39).
    var projectDisplayName: String
    /// 신뢰된 프로젝트에서만 세션을 시작합니다(ADR-0005 결정 4.5).
    var projectTrusted: Bool
    var installedCLIs: [AgentCLIOption]
    var selectedCLIID: AgentCLIID?
    /// CLI가 보고한 모델 이름을 그대로 표시합니다(ADR-0005 결정 4.1).
    var reportedModelName: String?
    var connection: AgentConnectionState
    var mode: AgentPermissionMode
    /// 에이전트가 현재 차례를 처리 중인지(코어 기록 기준).
    var isAgentWorking: Bool

    var selectedCLI: AgentCLIOption? {
        installedCLIs.first { $0.id == selectedCLIID }
    }

    /// 중지할 수 있는 세션이 있는지.
    var isSessionActive: Bool {
        switch connection {
        case .starting, .connected, .reconnecting: return true
        case .noSession, .ended: return false
        }
    }
}

// MARK: - 패널 단계

/// 진행 중 작업의 종류. 문구는 셸이 정합니다.
enum AgentLoadingReason: Equatable {
    case detectingCLIs
    case startingSession
}

/// 실패 종류. 문구는 셸이 정하고 프로젝트 데이터를 담지 않습니다.
enum AgentFailureReason: Equatable {
    case sessionEndedUnexpectedly
    case cliLaunchRejected
    case connectionLost
}

enum AgentPanelPhase: Equatable {
    /// 이 창의 내장 패널에 세션 제공자가 연결되어 있지 않습니다.
    case unavailable
    case loading(AgentLoadingReason)
    case ready
    case failed(AgentFailureReason)
}

// MARK: - 대화 기록 항목

/// Stellar가 자체 tool 호출 기록으로 만든 단계의 상태.
enum AgentToolStepStatus: Equatable {
    case running
    case succeeded
    case failed
    case rejected
}

/// 도구 단계. CLI 출력을 해석한 것이 아니라 Stellar의 tool 호출 기록입니다.
struct AgentToolStep: Equatable {
    /// Stellar tool 카탈로그의 표시 이름.
    var title: String
    /// Stellar 기록 요약(예: 읽은 노드 수).
    var detail: String?
    var status: AgentToolStepStatus
}

enum AgentApprovalKind: Equatable {
    /// 문서 편집 batch(Propose).
    case editBatch
    /// 민감 확인: Git commit.
    case gitCommit
    /// 민감 확인: Git push.
    case gitPush

    /// 파괴적 동작 확인 패턴이 필요한지(ADR-0005 결정 4.5 "민감 확인").
    var requiresSensitiveConfirmation: Bool {
        switch self {
        case .editBatch: return false
        case .gitCommit, .gitPush: return true
        }
    }
}

enum AgentApprovalStatus: Equatable {
    case pending
    case approved
    case declined
    /// 응답 없음·창 닫힘·세션 종료로 거부 처리됨.
    case expired
}

/// 코어 semantic diff 요약 한 줄.
struct AgentDiffLine: Equatable {
    enum Change: Equatable {
        case added
        case removed
        case modified
    }

    var change: Change
    /// 대상 경로(코어 생성, 예: "Sign In / Button / Label").
    var target: String
    /// 속성 변화 요약(코어 생성).
    var detail: String?
}

/// 코어 validation 결과 요약.
struct AgentValidationSummary: Equatable {
    var errorCount: Int
    var warningCount: Int
    /// 코어 진단 메시지(표시용 일부).
    var messages: [String]
}

/// 승인 카드. 주 내용은 코어가 만든 값이고, 에이전트 설명은 출처를 표시한 보조 정보입니다.
struct AgentApprovalRequest: Equatable {
    let approvalID: AgentApprovalID
    var kind: AgentApprovalKind
    /// 코어가 만든 한 줄 요약.
    var summary: String
    /// 코어 semantic diff 요약(앞부분 페이지).
    var diffLines: [AgentDiffLine]
    /// 이 카드에 싣지 않은 diff 줄 수. 전체는 코어 diff 화면에서 봅니다.
    var omittedDiffLineCount: Int
    var validation: AgentValidationSummary
    /// 코어가 계산한 잠금(lease) 범위 요약.
    var lockScope: String
    /// 에이전트가 붙인 작업 설명. Stellar가 검증하지 않은 보조 정보입니다.
    var agentExplanation: String?
    var status: AgentApprovalStatus
}

enum AgentChangeState: Equatable {
    case applied
    case reverted
    /// 이후 편집과 겹쳐 자동으로 되돌릴 수 없음(코어 판단).
    case revertUnavailable
}

/// 에이전트가 적용한 변경. 출처 표시를 유지하고 사용자가 되돌릴 수 있습니다.
struct AgentAppliedChange: Equatable {
    let changeID: AgentChangeID
    /// 코어가 만든 변경 요약.
    var summary: String
    var detail: String?
    var state: AgentChangeState
}

/// Stellar가 만든 세션 알림 항목. 문구는 셸이 정합니다.
enum AgentNotice: Equatable {
    case sessionStarted(cliName: String, mode: AgentPermissionMode)
    case modeChanged(AgentPermissionMode)
    /// 세션 종료. 자동 롤백 없이 변경을 유지했음을 알립니다.
    case sessionEnded(keptChangeCount: Int)
    case leasesReleased
}

struct AgentConversationItem: Equatable {
    enum Content: Equatable {
        /// 사용자가 보낸 문장(그대로 CLI에 전달됨).
        case userMessage(String)
        /// 에이전트 응답(표시 전용). 스트리밍 중이면 `isStreaming`.
        case agentMessage(String, isStreaming: Bool)
        case toolStep(AgentToolStep)
        case approval(AgentApprovalRequest)
        case appliedChange(AgentAppliedChange)
        case notice(AgentNotice)
    }

    let id: AgentItemID
    var content: Content
}

/// 세션 전체의 에이전트 변경 집계(세션 단위 되돌리기용).
struct AgentSessionChangeSummary: Equatable {
    var appliedCount: Int
    var revertableCount: Int

    static let none = AgentSessionChangeSummary(appliedCount: 0, revertableCount: 0)
}

// MARK: - 스냅샷·이벤트·명령

/// 패널 전체 상태 스냅샷. 처음 붙일 때와 세션 교체(reset) 때 씁니다.
struct AgentPanelSnapshot: Equatable {
    /// 세대 번호. 현재와 다른 세대의 이벤트는 버립니다(아키텍처 §9.2).
    var generation: UInt64
    var header: AgentSessionHeader
    var phase: AgentPanelPhase
    var items: [AgentConversationItem]
    /// CLI 출력 원문(텍스트 보기). 해석하지 않은 텍스트입니다.
    var rawText: String
    var changeSummary: AgentSessionChangeSummary
}

/// OS 알림·주의 요청 종류. 연관값이 없으므로 프로젝트 데이터를 실을 수 없습니다(ADR-0005 결정 4.5).
enum AgentAttentionKind: CaseIterable, Equatable {
    case approvalRequested
    case sessionEnded
    case sessionFailed
}

/// 코어→셸 이벤트(pull). 같은 세대의 증분 변경만 적용합니다.
struct AgentPanelEvent: Equatable {
    enum Change: Equatable {
        /// 전체 교체. 스냅샷의 세대가 새 기준이 됩니다.
        case reset(AgentPanelSnapshot)
        case headerChanged(AgentSessionHeader)
        case phaseChanged(AgentPanelPhase)
        case itemsAppended([AgentConversationItem])
        case itemUpdated(AgentConversationItem)
        case rawTextAppended(String)
        case changeSummaryChanged(AgentSessionChangeSummary)
        case attentionRequested(AgentAttentionKind)
    }

    let generation: UInt64
    let change: Change
}

enum AgentApprovalDecision: Equatable {
    case approve
    case decline
}

/// 셸→코어 명령. 사용자 의도만 담고 결과는 이벤트로 돌아옵니다.
enum AgentPanelCommand: Equatable {
    case refreshInstalledCLIs
    case selectCLI(AgentCLIID)
    /// 인증 필요 시 공식 절차용 로그인 터미널 열기(ADR-0005 결정 4.1).
    case openSignInTerminal(AgentCLIID)
    case reviewProjectTrust
    /// 시작 확인을 받은 뒤에만 보냅니다(첫 연결 승인으로 기록).
    case startSession(cli: AgentCLIID, mode: AgentPermissionMode)
    case stopSession
    /// 이 프로젝트에서 앱과 연결된 모든 에이전트 lease 해제.
    case releaseAllProjectLeases
    case setMode(AgentPermissionMode)
    /// 사용자 문장을 그대로 전달합니다. 시스템 프롬프트 템플릿은 없습니다.
    case sendMessage(String)
    case respondToApproval(AgentApprovalID, AgentApprovalDecision)
    /// 코어 transient preview 경로로 렌더 프리뷰 요청.
    case previewApproval(AgentApprovalID)
    case revertChange(AgentChangeID)
    case revertAllSessionChanges
}

/// 에이전트 패널의 데이터 원천. 향후 Rust 코어(FFI pull 이벤트·스냅샷)가 구현합니다.
///
/// 모든 메서드는 메인 스레드에서 호출하며 즉시 반환해야 합니다(아키텍처 §6.3 단일 점유 2ms).
/// 무거운 작업(CLI 프로세스, IPC, diff 계산)은 구현체가 다른 스레드에서 처리하고 결과를 이벤트로 넣습니다.
protocol AgentPanelSource: AnyObject {
    /// 현재 전체 상태.
    func currentSnapshot() -> AgentPanelSnapshot
    /// 대기 중인 이벤트 하나를 꺼냅니다. 없으면 nil.
    func pollEvent() -> AgentPanelEvent?
    /// 사용자 명령을 전달합니다.
    func perform(_ command: AgentPanelCommand)
}

/// 세션 제공자가 없는 창에서는 CLI 탐지 결과나 세션 이벤트를 만들지 않습니다.
final class DisconnectedAgentPanelSource: AgentPanelSource {
    func currentSnapshot() -> AgentPanelSnapshot {
        AgentPanelSnapshot(
            generation: 0,
            header: AgentSessionHeader(
                projectDisplayName: "", projectTrusted: false, installedCLIs: [], selectedCLIID: nil,
                reportedModelName: nil, connection: .noSession, mode: .readOnly, isAgentWorking: false),
            phase: .unavailable, items: [], rawText: "", changeSummary: .none)
    }

    func pollEvent() -> AgentPanelEvent? { nil }
    func perform(_ command: AgentPanelCommand) {}
}
