import AppKit

/// 앱 UI 텍스트용 AppKit 네이티브 컨트롤 팩토리(XII.5.1).
///
/// 레이어명·일반 필드는 `NSTextField`, 검색은 `NSSearchField`(`NSTextField` 하위 클래스),
/// 여러 줄 입력은 `NSTextView`를 사용합니다. 입력·선택·undo·IME(marked text)·받아쓰기는
/// AppKit 텍스트 시스템이 처리하며, 이 파일은 코어 텍스트 엔진(FFI)을 호출하지 않습니다.
/// 캔버스 Text node 편집은 이 경로가 아니라 공통 editing session(XII.5.3)을 사용합니다.
enum NativeTextControls {
    /// 레이어 이름 입력: 한 줄, 서식 없는 plain text.
    static func makeLayerNameField(_ name: String = "") -> NSTextField {
        let field = NSTextField(string: name)
        configureSingleLine(field, placeholder: "Layer name")
        return field
    }

    /// 검색 입력: 시스템 검색 필드(취소 버튼·최근 검색 등 native 동작 유지).
    static func makeSearchField() -> NSSearchField {
        let field = NSSearchField()
        configureSingleLine(field, placeholder: "Search")
        return field
    }

    /// 일반 한 줄 값 입력(속성 패널·다이얼로그 등).
    static func makeGeneralField(placeholder: String) -> NSTextField {
        let field = NSTextField(string: "")
        configureSingleLine(field, placeholder: placeholder)
        return field
    }

    /// 여러 줄 plain text 입력(설명·커밋 메시지 등). scroll view와 text view를 함께 반환합니다.
    static func makeMultilineTextView() -> (scrollView: NSScrollView, textView: NSTextView) {
        let scrollView = NSTextView.scrollableTextView()
        scrollView.hasVerticalScroller = true
        scrollView.borderType = .bezelBorder
        // scrollableTextView()는 항상 NSTextView를 documentView로 만듭니다.
        let textView = (scrollView.documentView as? NSTextView) ?? NSTextView()
        textView.isRichText = false
        textView.importsGraphics = false
        textView.isEditable = true
        textView.isSelectable = true
        textView.allowsUndo = true
        textView.font = NSFont.systemFont(ofSize: NSFont.systemFontSize)
        return (scrollView, textView)
    }

    private static func configureSingleLine(_ field: NSTextField, placeholder: String) {
        field.isEditable = true
        field.isSelectable = true
        field.isBezeled = true
        field.usesSingleLineMode = true
        field.allowsEditingTextAttributes = false
        field.importsGraphics = false
        field.lineBreakMode = .byTruncatingTail
        field.cell?.isScrollable = true
        field.cell?.wraps = false
        field.placeholderString = placeholder
    }
}
