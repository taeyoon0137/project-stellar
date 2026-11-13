import AppKit

/// 창 왼쪽의 최소 네이티브 필드 패널(XII.5.1).
///
/// 검색, 레이어 이름, 일반 값, 여러 줄 메모 입력을 AppKit 컨트롤로 배치합니다.
/// 상세 UX(목록·바인딩·검색 동작)는 이후 단계에서 연결하며, 이 패널은 코어를 호출하지 않습니다.
final class NativeFieldsPanel: NSView {
    let searchField = NativeTextControls.makeSearchField()
    let layerNameField = NativeTextControls.makeLayerNameField()
    let generalField = NativeTextControls.makeGeneralField(placeholder: "Value")
    let notesScrollView: NSScrollView
    let notesTextView: NSTextView

    override init(frame frameRect: NSRect) {
        let notes = NativeTextControls.makeMultilineTextView()
        notesScrollView = notes.scrollView
        notesTextView = notes.textView
        super.init(frame: frameRect)

        let stack = NSStackView(views: [searchField, layerNameField, generalField, notesScrollView])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 8
        stack.edgeInsets = NSEdgeInsets(top: 12, left: 12, bottom: 12, right: 12)
        stack.translatesAutoresizingMaskIntoConstraints = false
        addSubview(stack)

        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: topAnchor),
            stack.leadingAnchor.constraint(equalTo: leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor),
            searchField.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24),
            layerNameField.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24),
            generalField.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24),
            notesScrollView.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -24),
            notesScrollView.heightAnchor.constraint(equalToConstant: 120),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("NativeFieldsPanel은 코드로만 생성합니다")
    }
}
