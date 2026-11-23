import Foundation

// 사이드바(페이지·레이어) 영역이 코어와 주고받는 값 타입과 provider 프로토콜.
//
// 경계 원칙(아키텍처 §8.2, ADR-0009):
// - 코어 → 셸: 불변 값 snapshot(`SidebarSnapshot`)을 pull합니다. 셸은 snapshot을 해석해 그리기만 합니다.
// - 셸 → 코어: 의도를 담은 command(`SidebarCommand`)만 보냅니다. 결과는 다음 snapshot으로 돌아옵니다.
// - Rust 타입을 노출하지 않습니다. 모든 필드는 FFI POD로 옮길 수 있는 값(UUID, 정수, Bool, UTF-8 문자열)입니다.
// - 문서 의미(트리 불변식, 순서 키, 이름 규칙)는 코어가 소유합니다(디자인 가이드 1.2 원칙 4).

// MARK: - 식별자

/// 영속 StableId(UUIDv7, 128비트)의 셸 측 값 표현(아키텍처 §12.1).
///
/// 셸은 비교·해시·전달만 하고 내용을 해석하지 않습니다. FFI에서는 16바이트 POD로 오갑니다.
struct SidebarStableID: Hashable, CustomStringConvertible {
    let uuid: UUID

    init(_ uuid: UUID) {
        self.uuid = uuid
    }

    /// 대소문자 무관 UUID 문자열에서 복원합니다(드래그 pasteboard 왕복용).
    init?(string: String) {
        guard let uuid = UUID(uuidString: string) else { return nil }
        self.uuid = uuid
    }

    var stringValue: String { uuid.uuidString }
    var description: String { uuid.uuidString }
}

// MARK: - 노드 종류

/// scene node 종류.
///
/// 숫자 값은 `crates/document/src/node_kind.rs`의 `NodeKind`(golden test로 고정, VI.2.1)와 같습니다.
/// ComponentDefinition은 scene node가 아니므로 여기에 없습니다(정의는 문서 resource).
enum SidebarNodeKind: UInt16, CaseIterable {
    /// layout containing block과 좌표계를 만드는 컨테이너.
    case frame = 1
    /// 렌더·layout 필드가 없는 그룹 marker.
    case fragment = 2
    /// 파라메트릭 도형. 세부 종류는 `SidebarShapeKind`.
    case shape = 3
    /// 자유 곡선 경로.
    case path = 4
    /// 텍스트.
    case text = 5
    /// 이미지.
    case image = 6
    /// 컴포넌트 인스턴스.
    case componentInstance = 7
}

/// Shape 세부 종류.
///
/// 숫자 tag는 `crates/document/src/shape.rs`의 `ShapeKind`(golden test로 고정, VI.5)와 같습니다.
enum SidebarShapeKind: UInt8, CaseIterable {
    case rectangle = 1
    case ellipse = 2
    case line = 3
    case polygon = 4
}

// MARK: - Snapshot

/// 레이어 트리 한 행의 값.
struct SidebarLayerRow: Equatable {
    let id: SidebarStableID
    /// 부모 node. `nil`이면 active View의 root Frame 바로 아래(최상위 행)입니다.
    let parentID: SidebarStableID?
    let kind: SidebarNodeKind
    /// `kind == .shape`일 때만 값이 있습니다. 모르는 tag는 `nil`로 두고 일반 Shape 아이콘을 씁니다.
    let shapeKind: SidebarShapeKind?
    /// 표시 이름. 비어 있을 수 있습니다(VI.1.3). 셸은 비어 있으면 종류 이름을 placeholder로 보여 줍니다.
    let name: String
    let isVisible: Bool
    let isLocked: Bool
    /// 자식이 하나 이상 있는지. 자식 목록이 아직 로드되지 않았어도 펼침 표시를 위해 정확해야 합니다.
    let hasChildren: Bool
    /// 필터가 켜져 있을 때 이 행이 직접 일치했는지. 일치하지 않은 조상 행은 문맥으로만 보입니다.
    /// 필터가 꺼져 있으면 항상 `true`입니다.
    let isFilterMatch: Bool
    /// 표시를 바꿀 수 있는지. 잠금 규칙(D8)은 코어가 계산하고 셸은 토글 활성에만 씁니다.
    var canHide = true
    /// 잠금을 바꿀 수 있는지(조상이 잠겼으면 false). 코어 값입니다.
    var canLock = true
}

/// active View 하나의 레이어 트리(필터가 켜져 있으면 필터 결과 트리).
///
/// 대량 트리를 한 번에 보내지 않도록 자식 목록은 부모 단위로 나눠 받습니다(아키텍처 §6.3, §8.2).
/// `childIDs`에 없는 부모는 "아직 로드하지 않음"이며, 셸이 펼칠 때 `.loadChildren`을 보냅니다.
struct SidebarLayerTree: Equatable {
    /// 트리 구조(행 집합, 부모·자식 관계, 순서)가 바뀔 때마다 코어가 올리는 번호.
    /// 같은 번호면 셸은 전체 reload 대신 보이는 행만 갱신합니다.
    var structureRevision: UInt64
    /// 최상위 행(root Frame의 자식), 표시 순서(위→아래).
    var topLevelIDs: [SidebarStableID]
    /// 로드된 부모별 자식, 표시 순서(위→아래).
    var childIDs: [SidebarStableID: [SidebarStableID]]
    /// 더 읽을 직계 자식이 남은 부모. nil은 View root의 자식 목록입니다.
    var moreParents: Set<SidebarStableID?> = []
    /// 이 snapshot에 포함된 모든 행.
    var rows: [SidebarStableID: SidebarLayerRow]

    static let empty = SidebarLayerTree(structureRevision: 0, topLevelIDs: [], childIDs: [:], rows: [:])
}

/// 페이지에 속한 View 한 행.
struct SidebarViewRow: Equatable {
    let id: SidebarStableID
    let name: String
}

/// 페이지 한 행과 그 View들(표시 순서).
struct SidebarPageRow: Equatable {
    let id: SidebarStableID
    let name: String
    let views: [SidebarViewRow]
}

/// 레이어 필터 상태. 필터 질의 실행은 코어(또는 worker)가 하고 셸은 결과만 받습니다(아키텍처 §6.4, §9.2).
struct SidebarFilterState: Equatable {
    /// 적용된 질의(비어 있지 않음).
    let query: String
    /// 직접 일치한 행 수. 0이면 "결과 없음" 상태입니다.
    let matchCount: Int
}

/// 레이어 목록 로드 상태.
enum SidebarLoadState: Equatable {
    /// 코어가 아직 snapshot을 만들지 못함(문서 open, 대형 chunk materialize 등).
    case loading
    case ready
    /// 사용자에게 보여 줄 짧은 사유. 프로젝트 유래 문자열은 넣지 않습니다(아키텍처 §39).
    case failed(message: String)
}

/// 사이드바 전체의 불변 snapshot.
///
/// 코어는 변경이 생길 때마다 `generation`을 올린 새 snapshot을 냅니다. 셸은 이미 반영한 세대보다
/// 오래된 snapshot을 버립니다(아키텍처 §9.1 "세대가 현재와 다른 결과는 버린다").
struct SidebarSnapshot: Equatable {
    var generation: UInt64
    var loadState: SidebarLoadState
    var pages: [SidebarPageRow]
    var activePageID: SidebarStableID?
    var activeViewID: SidebarStableID?
    var layers: SidebarLayerTree
    /// 창별 선택(WindowViewState, VII.3.1). 캔버스 선택과 같은 상태입니다. 선택한 순서를 유지합니다.
    var selection: [SidebarStableID]
    var primarySelection: SidebarStableID?
    /// `nil`이면 필터가 꺼져 있습니다.
    var filter: SidebarFilterState?

    static let empty = SidebarSnapshot(
        generation: 0,
        loadState: .loading,
        pages: [],
        activePageID: nil,
        activeViewID: nil,
        layers: .empty,
        selection: [],
        primarySelection: nil,
        filter: nil
    )
}

// MARK: - Command

/// 드롭 위치: `parentID` 아래 표시 순서 `index` 자리(이동 대상 제거 전 기준, 0 = 맨 위).
///
/// 표시 순서를 OrderKey(ADR-0003)로 바꾸는 일은 코어가 합니다. `parentID == nil`은 최상위(root Frame 직속)입니다.
struct SidebarDropTarget: Equatable {
    let parentID: SidebarStableID?
    let index: Int
}

/// 셸이 코어에 보내는 의도. 코어는 일반 Command로 적용해 창별 Undo에 넣습니다(아키텍처 §23, 계획 VII.2).
enum SidebarCommand: Equatable {
    // 페이지·View(이름 변경·추가는 코어 Command와 함께 UX.12에서 더합니다)
    case activateView(SidebarStableID)

    // 레이어
    /// 선택 변경(VII.3.2 SetSelection). 순서는 선택한 순서, `primary`는 마지막으로 더한 행입니다.
    case setSelection([SidebarStableID], primary: SidebarStableID?)
    /// 이름 변경(VII.2.5 RenameNode). 값이 같으면 셸이 보내지 않습니다.
    case renameNode(SidebarStableID, name: String)
    /// 표시 토글(VII.2.5 SetVisibility). 방향은 코어가 정합니다(하나라도 꺼져 있으면 모두 켬, ADR-0028).
    /// Bool은 쓰지 않는 이전 인터페이스의 잔재이며 UI 재설계 때 지웁니다.
    case setVisible([SidebarStableID], Bool)
    /// 잠금 토글(VII.2.5 SetLocked). 방향 규칙은 `setVisible`과 같습니다.
    case setLocked([SidebarStableID], Bool)
    /// 복제(VII.2.8 DuplicateSubtree).
    case duplicate([SidebarStableID])
    /// 삭제(VII.2.2 DeleteSubtree).
    case delete([SidebarStableID])
    /// 묶기: 대상을 새 그룹 marker(Fragment) 아래로 옮깁니다.
    case group([SidebarStableID])
    /// 풀기: 그룹 marker를 지우고 자식을 그 자리로 옮깁니다.
    case ungroup([SidebarStableID])
    /// 재정렬·재부모화(VII.2.3 MoveNode). 대상 순서는 표시 순서입니다.
    case move([SidebarStableID], to: SidebarDropTarget)

    // 표시
    /// 레이어 필터 질의. 빈 문자열은 필터 해제입니다.
    case setFilter(String)
    /// 부모 하나의 자식 목록 요청(페이지 단위 로드).
    case loadChildren(SidebarStableID)
    /// 이미 표시한 자식 다음 페이지 요청. nil은 View root입니다.
    case loadMore(SidebarStableID?)
    /// 실패 상태에서 다시 시도.
    case reload
}

/// command 사전 검증 결과. 메뉴 활성화와 드롭 피드백에 씁니다.
///
/// 사전 검증은 표시용이며 최종 판정은 command 적용 시 코어가 다시 합니다.
enum SidebarCommandValidation: Equatable {
    case allowed
    case denied(SidebarDenialReason)

    var isAllowed: Bool { self == .allowed }
}

/// 거부 사유. 이동 관련 사유는 `crates/document/src/tree_ops.rs`의 `ReparentError`와 대응합니다.
enum SidebarDenialReason: Equatable {
    /// 대상이 비어 있습니다.
    case emptyTarget
    /// 대상 id를 찾을 수 없습니다(`ReparentError::StaleHandle`).
    case unknownNode
    /// 자기 자신을 부모로 지정했습니다(`ReparentError::SelfParent`).
    case selfParent
    /// 자기 자손을 부모로 지정했습니다(`ReparentError::DescendantParent`).
    case descendantParent
    /// 다른 View의 tree로 이동하려 합니다(`ReparentError::CrossViewMove`).
    case crossViewMove
    /// View root는 이동할 수 없습니다(`ReparentError::ViewRootMove`).
    case viewRootMove
    /// 대상 부모가 자식을 가질 수 없는 종류입니다.
    case notContainer
    /// 풀 수 있는 그룹 marker가 없습니다.
    case notGroup
    /// 부모가 서로 다른 행을 한 번에 묶으려 합니다.
    case mixedParents
    /// 필터가 켜져 있어 표시 순서가 실제 순서와 다릅니다.
    case filterActive
    /// 지금 상태에서 지원하지 않는 command입니다.
    case unsupported
}

// MARK: - Provider

/// 사이드바가 필요로 하는 데이터와 동작의 공급자.
///
/// 프로젝트 창은 실제 창 문서를 읽으며 UI 검사는 fixture provider를 씁니다.
/// provider는 새 값을 게시한 뒤 `snapshotDidChange`로 알립니다(ADR-0009).
/// 모든 메서드는 메인 스레드에서 호출되며 2ms 예산 안에서 반환해야 합니다(아키텍처 §6.3).
@MainActor
protocol SidebarDataProvider: AnyObject {
    /// 가장 최근 snapshot.
    var snapshot: SidebarSnapshot { get }

    /// snapshot이 바뀌면 호출됩니다. 셸은 이 호출을 다음 런 루프에서 한 번으로 모아 반영합니다.
    var snapshotDidChange: (() -> Void)? { get set }

    /// command를 지금 적용할 수 있는지 가볍게 확인합니다(메뉴 검증, 드롭 검증).
    func validate(_ command: SidebarCommand) -> SidebarCommandValidation

    /// command를 보냅니다. 결과는 다음 snapshot으로 반영됩니다.
    func perform(_ command: SidebarCommand)

    /// 빈 레이어 목록에 보일 다음 행동 안내 한 줄(UX 명세 7.2). 도구 키처럼 사이드바 밖의 배정을 담으므로 셸이 넣고,
    /// 사이드바는 받은 문자열만 보여 줍니다.
    var emptyLayersHint: String { get }
}
