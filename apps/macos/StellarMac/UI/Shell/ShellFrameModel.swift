import Foundation

// 메인 창 프레임(툴바·창 제목·빈/로딩/오류 상태)이 코어에서 받는 값과 코어로 보내는 명령.
//
// 코어 연결 전 단계이므로 Rust 타입을 쓰지 않습니다. 코어 연결 시에는 FFI pull 이벤트
// (`stellar_runtime_poll_event`)로 받은 메시지를 아래 값 snapshot으로 바꾸고, 셸 동작은
// `ShellFrameCommand`로 바꿔 보냅니다(아키텍처 §8.2 Snapshot·Command, ADR-0009).

/// 코어와 공유하는 명령 식별자(디자인 가이드 6.1 "command ID는 코어와 공유").
///
/// 셸은 값을 해석하지 않고 툴바·메뉴·빈 상태 버튼에서 그대로 되돌려 보냅니다.
struct ShellCommandID: RawRepresentable, Hashable, Sendable {
    let rawValue: String

    init(rawValue: String) {
        self.rawValue = rawValue
    }

    init(_ rawValue: String) {
        self.rawValue = rawValue
    }
}

/// 툴바 가운데 도구 영역의 도구 하나. 아이콘은 SF Symbol 이름으로만 받습니다(가이드 5.4).
struct ShellToolDescriptor: Equatable, Sendable {
    /// 도구 선택 시 코어로 보낼 명령 ID.
    var id: ShellCommandID
    /// 도구 이름. 접근성 레이블, 도움말 태그, overflow 메뉴 제목으로 씁니다(가이드 8.1, 8.3).
    var label: String
    /// SF Symbol 이름.
    var symbolName: String
}

/// 더 보기 메뉴, 빈 상태, 오류 복구 버튼이 실행하는 동작 하나.
struct ShellActionDescriptor: Equatable, Sendable {
    var id: ShellCommandID
    /// 메뉴·버튼 제목. 추가 입력이 필요한 명령은 말줄임표(…)로 끝냅니다(가이드 8.1).
    var title: String
    var isEnabled: Bool = true
    /// 한 문맥의 대표 동작. 한 문맥에는 하나만 둡니다(가이드 4.2 "강조", 5.1).
    var isPrimary: Bool = false
}

/// 창 전체 상태. 셸 프레임은 `ready`가 아니면 콘텐츠 슬롯 대신 상태 화면을 보여 줍니다.
enum ShellFrameStatus: Equatable, Sendable {
    /// 콘텐츠 슬롯(캔버스)을 표시합니다.
    case ready
    /// 열린 대상이 없습니다. 다음에 할 일과 그 동작 버튼을 함께 둡니다(가이드 7.1).
    case empty(title: String, message: String, actions: [ShellActionDescriptor])
    /// 프로젝트·문서를 여는 중입니다.
    case loading(message: String)
    /// 열기에 실패했습니다. 해결 방법(복구 동작)을 함께 둡니다(가이드 7.1).
    case failed(title: String, message: String, recoveryActions: [ShellActionDescriptor])
}

/// 툴바 내비게이션(뒤로·앞으로) 활성 상태.
struct ShellNavigationState: Equatable, Sendable {
    var canGoBack: Bool
    var canGoForward: Bool
}

/// 코어가 셸 프레임에 넘기는 불변 값 snapshot(아키텍처 §8.2).
struct ShellFrameSnapshot: Equatable, Sendable {
    /// 단조 증가 세대 번호. 이미 반영한 세대 이하의 snapshot은 버립니다(가이드 9장, 아키텍처 §9.2).
    var generation: UInt64
    /// 창 제목: 열린 프로젝트·문서 이름. 앱 이름을 반복하지 않습니다(가이드 2.2).
    var title: String
    /// 창 부제목(예: 현재 페이지·View). 빈 문자열이면 표시하지 않습니다.
    var subtitle: String
    var status: ShellFrameStatus
    var navigation: ShellNavigationState
    /// 툴바 가운데 도구 영역 항목. 상호작용 상태 기계 자체는 XII.3.6 범위입니다.
    var tools: [ShellToolDescriptor]
    var activeToolID: ShellCommandID?
    /// 툴바 뒤쪽 끝 더 보기 메뉴 항목.
    var moreActions: [ShellActionDescriptor]
    /// 프로젝트 전체 검색 필드의 placeholder.
    var searchPlaceholder: String
}

/// 셸 프레임이 코어로 보내는 명령(아키텍처 §8.2).
///
/// 사이드바·인스펙터·보조 패널 접기는 문서 의미가 아닌 창 상태이므로 명령으로 보내지 않고
/// AppKit split view 자동 저장이 맡습니다(가이드 2.6).
enum ShellFrameCommand: Equatable, Sendable {
    case navigateBack
    case navigateForward
    case selectTool(ShellCommandID)
    /// 검색 필드에서 Return으로 확정한 질의. 질의 실행은 코어가 메인 스레드 밖에서 합니다(가이드 9장).
    case submitSearch(query: String)
    /// 더 보기 메뉴, 빈 상태, 오류 복구 동작.
    case perform(ShellCommandID)
}

/// 셸 프레임의 데이터·동작 경계.
///
/// 구현체는 snapshot을 메인 스레드에서 `snapshotHandler`로 전달하고, `send(_:)`는 즉시 반환해야
/// 합니다. 무거운 일(검색, 프로젝트 열기)은 코어가 메인 스레드 밖에서 처리합니다(아키텍처 §6.3, §9.2).
@MainActor
protocol ShellFrameDataSource: AnyObject {
    /// 현재 snapshot. 창을 만들 때 한 번 읽습니다.
    var currentSnapshot: ShellFrameSnapshot { get }
    /// 새 snapshot 수신 handler. 셸 창 controller가 등록합니다.
    var snapshotHandler: ((ShellFrameSnapshot) -> Void)? { get set }
    /// 명령을 보냅니다.
    func send(_ command: ShellFrameCommand)
}
