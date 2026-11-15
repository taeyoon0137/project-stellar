import AppKit

/// 사이드바가 쓰는 SF Symbols 이름(디자인 가이드 5.4). 이미지 asset은 쓰지 않습니다.
///
/// 모든 이름은 `SidebarUICheck`가 실행 OS에서 실제로 해석되는지 확인합니다.
enum SidebarSymbols {
    // 노드 종류
    static let frame = "rectangle.dashed"
    static let fragment = "square.on.square.dashed"
    static let shape = "square.on.circle"
    static let rectangle = "rectangle"
    static let ellipse = "circle"
    static let line = "line.diagonal"
    static let polygon = "pentagon"
    static let path = "point.topleft.down.to.point.bottomright.curvepath"
    static let text = "textformat"
    static let image = "photo"
    static let componentInstance = "diamond"

    // 행 토글
    static let visible = "eye"
    static let hidden = "eye.slash"
    static let locked = "lock.fill"
    static let unlocked = "lock.open"

    // 상태 화면
    static let noResults = "magnifyingglass"
    static let failed = "exclamationmark.triangle"
    static let noView = "sidebar.left"

    /// 노드 종류(Shape는 세부 종류)별 symbol 이름.
    static func name(for kind: SidebarNodeKind, shapeKind: SidebarShapeKind?) -> String {
        switch kind {
        case .frame: return frame
        case .fragment: return fragment
        case .shape:
            switch shapeKind {
            case .rectangle: return rectangle
            case .ellipse: return ellipse
            case .line: return line
            case .polygon: return polygon
            case nil: return shape
            }
        case .path: return path
        case .text: return text
        case .image: return image
        case .componentInstance: return componentInstance
        }
    }

    /// 검사용: 이 파일이 쓰는 모든 이름.
    static var allNames: [String] {
        [
            frame, fragment, shape, rectangle, ellipse, line, polygon, path, text, image, componentInstance,
            visible, hidden, locked, unlocked, noResults, failed, noView,
        ]
    }

    /// symbol 이미지. 장식용(접근성 설명 없음) 이미지는 `label`을 `nil`로 둡니다.
    static func image(_ name: String, label: String? = nil) -> NSImage? {
        NSImage(systemSymbolName: name, accessibilityDescription: label)
    }
}

/// 사이드바 UI 문자열. 개발 지역은 en이며 지역화 대상입니다(SWIFT_EMIT_LOC_STRINGS).
enum SidebarStrings {
    static let layers = String(localized: "Layers", comment: "사이드바 레이어 구획 제목")
    static let filterPlaceholder = String(localized: "Filter", comment: "레이어 필터 필드 placeholder")
    static let filterLabel = String(localized: "Filter Layers", comment: "레이어 필터 필드 접근성 레이블")

    // 메뉴·동작
    static let rename = String(localized: "Rename", comment: "이름 변경 메뉴")
    static let duplicate = String(localized: "Duplicate", comment: "복제 메뉴")
    static let delete = String(localized: "Delete", comment: "삭제 메뉴")
    static let group = String(localized: "Group Selection", comment: "선택 묶기 메뉴")
    static let ungroup = String(localized: "Ungroup", comment: "그룹 풀기 메뉴")
    static let show = String(localized: "Show", comment: "레이어 보이기 메뉴")
    static let hide = String(localized: "Hide", comment: "레이어 숨기기 메뉴")
    static let lock = String(localized: "Lock", comment: "레이어 잠그기 메뉴")
    static let unlock = String(localized: "Unlock", comment: "레이어 잠금 해제 메뉴")
    static let lockedLayerCannotHide = String(
        localized: "Locked layers can’t be hidden.", comment: "잠긴 레이어의 비활성 표시 토글 도움말"
    )
    static let parentLayerLocked = String(
        localized: "A parent layer is locked.", comment: "조상이 잠긴 레이어의 비활성 토글 도움말"
    )

    // 상태 화면
    static let loading = String(localized: "Loading Layers…", comment: "레이어 로드 중")
    static let failedTitle = String(localized: "Couldn’t Load Layers", comment: "레이어 로드 실패 제목")
    static let retry = String(localized: "Try Again", comment: "다시 시도 버튼")
    static let noViewTitle = String(localized: "No View Selected", comment: "선택된 View 없음 제목")
    static let noViewMessage = String(
        localized: "Select a page or view to see its layers.",
        comment: "선택된 View 없음 설명"
    )
    static let noResultsTitle = String(localized: "No Results", comment: "필터 결과 없음 제목")
    static let clearFilter = String(localized: "Clear Filter", comment: "필터 지우기 버튼")

    static func noResultsMessage(_ query: String) -> String {
        String(localized: "No layers match “\(query)”.", comment: "필터 결과 없음 설명")
    }

    // 접근성
    static let hiddenState = String(localized: "Hidden", comment: "레이어 숨김 상태(접근성)")
    static let lockedState = String(localized: "Locked", comment: "레이어 잠김 상태(접근성)")
    static let contextOnly = String(
        localized: "Contains matches",
        comment: "필터에 직접 일치하지 않고 일치한 자손을 담은 행(접근성)"
    )
    static let pagePath = String(localized: "Page and View", comment: "사이드바 머리 줄 Page › View 팝업 접근성 레이블")
    static let layersOutline = String(localized: "Layers", comment: "레이어 트리 접근성 레이블")

    static func showLayer(_ name: String) -> String {
        String(localized: "Show \(name)", comment: "레이어 보이기 버튼 접근성 레이블")
    }

    static func hideLayer(_ name: String) -> String {
        String(localized: "Hide \(name)", comment: "레이어 숨기기 버튼 접근성 레이블")
    }

    static func lockLayer(_ name: String) -> String {
        String(localized: "Lock \(name)", comment: "레이어 잠그기 버튼 접근성 레이블")
    }

    static func unlockLayer(_ name: String) -> String {
        String(localized: "Unlock \(name)", comment: "레이어 잠금 해제 버튼 접근성 레이블")
    }

    /// 노드 종류 표시 이름. 이름이 빈 행의 placeholder와 접근성 레이블에 씁니다.
    static func kindName(_ kind: SidebarNodeKind, shapeKind: SidebarShapeKind?) -> String {
        switch kind {
        case .frame: return String(localized: "Frame", comment: "노드 종류")
        case .fragment: return String(localized: "Fragment", comment: "노드 종류(그룹 marker)")
        case .shape:
            switch shapeKind {
            case .rectangle: return String(localized: "Rectangle", comment: "Shape 종류")
            case .ellipse: return String(localized: "Ellipse", comment: "Shape 종류")
            case .line: return String(localized: "Line", comment: "Shape 종류")
            case .polygon: return String(localized: "Polygon", comment: "Shape 종류")
            case nil: return String(localized: "Shape", comment: "노드 종류")
            }
        case .path: return String(localized: "Path", comment: "노드 종류")
        case .text: return String(localized: "Text", comment: "노드 종류")
        case .image: return String(localized: "Image", comment: "노드 종류")
        case .componentInstance: return String(localized: "Component Instance", comment: "노드 종류")
        }
    }

    static let pageKind = String(localized: "Page", comment: "페이지 종류(접근성)")
    static let viewKind = String(localized: "View", comment: "View 종류(접근성)")
}
