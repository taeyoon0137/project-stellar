import AppKit

/// 입력 서비스의 응답자(ADR-0067 §6). 입력기에 즉시 답하도록 본문·선택·조합 범위의 사본을 두고, 조합 시작·
/// 갱신·확정·취소와 조합 없는 대체를 코어 편집 session에 넘깁니다. 글자·caret 위치는 코어 scene이 정합니다.
@MainActor
final class CanvasTextInputView: NSView, NSTextInputClient {
    private(set) var buffer: CanvasTextBuffer
    /// 조합 시작 직전 상태와 그 상태에서 조합이 대체하는 범위.
    private var beforeMark: CanvasTextBuffer?
    private var markRange: NSRange?
    /// 조합 시작(확정 본문 기준 범위, 확정 본문).
    var began: ((NSRange, String) -> Void)?
    /// 조합 본문 갱신.
    var marked: ((String) -> Void)?
    /// 조합 범위를 확정 본문으로 바꿈.
    var committed: ((String) -> Void)?
    /// 조합 취소.
    var cancelled: (() -> Void)?
    /// 조합 없는 대체(범위, 새 본문, 대체 전 본문).
    var replaced: ((NSRange, String, String) -> Void)?
    var changed: ((CanvasTextBuffer) -> Void)?
    /// Undo(false)·Redo(true). 편집 중에도 문서 history로 보냅니다.
    var history: ((Bool) -> Void)?
    var ended: (() -> Void)?
    var rectForRange: ((NSRange) -> CGRect?)?
    var indexAtPoint: ((CGPoint) -> Int?)?
    private var finished = false
    private let missing = NSRange(location: NSNotFound, length: 0)

    init(buffer: CanvasTextBuffer) {
        self.buffer = buffer
        super.init(frame: .zero)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var acceptsFirstResponder: Bool { true }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override func keyDown(with event: NSEvent) { interpretKeyEvents([event]) }
    override func resignFirstResponder() -> Bool {
        let removed = super.resignFirstResponder()
        if removed { finish() }
        return removed
    }

    /// 포커스를 잃으면 진행 중인 조합을 먼저 확정한 뒤 편집을 끝냅니다.
    func finish() {
        guard !finished else { return }
        if hasMarkedText() {
            unmarkText()
            inputContext?.discardMarkedText()
        }
        finished = true
        beforeMark = nil
        markRange = nil
        ended?()
    }

    /// Undo·Redo 뒤 코어가 돌려준 본문으로 사본을 바꿉니다. 조합 중에는 바꾸지 않습니다.
    func reload(_ next: CanvasTextBuffer) {
        guard beforeMark == nil else { return }
        buffer = next
        changed?(buffer)
    }

    func selectedRange() -> NSRange { buffer.selection }
    func markedRange() -> NSRange { buffer.marked ?? missing }
    func hasMarkedText() -> Bool { buffer.marked != nil }
    func validAttributesForMarkedText() -> [NSAttributedString.Key] { [] }

    func attributedSubstring(forProposedRange range: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? {
        guard let base = CanvasTextBuffer.bounds(range, in: buffer.contents) else { return nil }
        actualRange?.pointee = range
        return NSAttributedString(string: String(buffer.contents.unicodeScalars[base]))
    }

    func firstRect(forCharacterRange range: NSRange, actualRange: NSRangePointer?) -> NSRect {
        guard let window, let canvas = superview, let rect = rectForRange?(range) else { return .zero }
        actualRange?.pointee = range
        return window.convertToScreen(canvas.convert(rect, to: nil))
    }

    func characterIndex(for point: NSPoint) -> Int {
        guard let window, let canvas = superview else { return NSNotFound }
        return indexAtPoint?(canvas.convert(window.convertPoint(fromScreen: point), from: nil)) ?? NSNotFound
    }

    private func plain(_ value: Any) -> String? {
        if let text = value as? NSAttributedString { return text.string }
        return value as? String
    }

    func setMarkedText(_ value: Any, selectedRange: NSRange, replacementRange: NSRange) {
        guard !finished, let text = plain(value) else { return }
        if text.isEmpty {
            guard let previous = beforeMark else { return }
            buffer = previous
            beforeMark = nil
            markRange = nil
            cancelled?()
            changed?(buffer)
            return
        }
        let previous = buffer
        let range = buffer.target(replacementRange)
        guard buffer.replace(text, replacement: range, markedSelection: selectedRange) else { return }
        if beforeMark == nil {
            beforeMark = previous
            markRange = range
            began?(range, previous.contents)
        }
        marked?(text)
        changed?(buffer)
    }

    func insertText(_ value: Any, replacementRange: NSRange) {
        guard !finished, let text = plain(value) else { return }
        let original = beforeMark ?? buffer
        let composing = beforeMark != nil
        let range = buffer.target(replacementRange)
        guard buffer.replace(text, replacement: range, markedSelection: nil) else { return }
        let target = markRange ?? range
        beforeMark = nil
        markRange = nil
        if composing {
            // 조합 범위를 그대로 바꾼 확정이면 조합을 확정합니다. 입력기가 다른 범위까지 바꿨으면 조합을 버리고
            // 원래 본문에 대한 최소 대체를 보냅니다.
            var trial = original
            if trial.replace(text, replacement: target, markedSelection: nil),
               trial.contents.utf8.elementsEqual(buffer.contents.utf8) {
                committed?(text)
            } else {
                cancelled?()
                let change = Self.replacement(from: original.contents, to: buffer.contents, preferred: target, inserted: text)
                replaced?(change.0, change.1, original.contents)
            }
        } else if !original.contents.utf8.elementsEqual(buffer.contents.utf8) {
            replaced?(range, text, original.contents)
        }
        changed?(buffer)
    }

    static func replacement(from source: String, to result: String, preferred: NSRange,
                            inserted: String) -> (NSRange, String) {
        if var trial = CanvasTextBuffer(source, caret: 0),
           trial.replace(inserted, replacement: preferred, markedSelection: nil),
           trial.contents.utf8.elementsEqual(result.utf8) { return (preferred, inserted) }
        let a = Array(source.unicodeScalars), b = Array(result.unicodeScalars)
        var lo = 0, aEnd = a.count, bEnd = b.count
        while lo < aEnd, lo < bEnd, a[lo] == b[lo] { lo += 1 }
        while lo < aEnd, lo < bEnd, a[aEnd - 1] == b[bEnd - 1] { aEnd -= 1; bEnd -= 1 }
        let start = a[..<lo].reduce(0) { $0 + $1.utf16.count }
        let length = a[lo..<aEnd].reduce(0) { $0 + $1.utf16.count }
        return (NSRange(location: start, length: length), String(String.UnicodeScalarView(b[lo..<bEnd])))
    }

    func unmarkText() {
        guard let range = buffer.marked, let bounds = CanvasTextBuffer.bounds(range, in: buffer.contents) else { return }
        insertText(String(buffer.contents.unicodeScalars[bounds]), replacementRange: range)
    }

    @objc override func cancelOperation(_ sender: Any?) {
        if hasMarkedText() {
            setMarkedText("", selectedRange: NSRange(location: 0, length: 0), replacementRange: missing)
            inputContext?.discardMarkedText()
        } else { window?.makeFirstResponder(superview) }
    }
    @objc func undo(_ sender: Any?) { history?(false) }
    @objc func redo(_ sender: Any?) { history?(true) }
    @objc override func insertNewline(_ sender: Any?) { insertText("\n", replacementRange: missing) }
    @objc override func deleteBackward(_ sender: Any?) {
        var r = buffer.selection
        if r.length == 0, r.location > 0 {
            r = (buffer.contents as NSString).rangeOfComposedCharacterSequence(at: r.location - 1)
        }
        if r.length != 0 { insertText("", replacementRange: r) }
    }
    @objc override func deleteForward(_ sender: Any?) {
        var r = buffer.selection
        if r.length == 0, r.location < buffer.contents.utf16.count {
            r = (buffer.contents as NSString).rangeOfComposedCharacterSequence(at: r.location)
        }
        if r.length != 0 { insertText("", replacementRange: r) }
    }
    @objc override func selectAll(_ sender: Any?) {
        unmarkText()
        buffer.select(NSRange(location: 0, length: buffer.contents.utf16.count))
        changed?(buffer)
    }
    @objc override func moveLeft(_ sender: Any?) { move(forward: false) }
    @objc override func moveRight(_ sender: Any?) { move(forward: true) }
    private func move(forward: Bool) {
        unmarkText()
        let text = buffer.contents as NSString
        let range = buffer.selection
        var i = forward ? NSMaxRange(range) : range.location
        if range.length == 0 {
            if forward, i < text.length { i = NSMaxRange(text.rangeOfComposedCharacterSequence(at: i)) }
            if !forward, i > 0 { i = text.rangeOfComposedCharacterSequence(at: i - 1).location }
        }
        buffer.select(NSRange(location: i, length: 0))
        changed?(buffer)
    }
    @objc func copy(_ sender: Any?) {
        guard buffer.selection.length > 0, let text = attributedSubstring(forProposedRange: buffer.selection, actualRange: nil) else { return }
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text.string, forType: .string)
    }
    @objc func cut(_ sender: Any?) {
        guard buffer.selection.length > 0 else { return }
        copy(sender)
        insertText("", replacementRange: buffer.selection)
    }
    @objc func paste(_ sender: Any?) {
        if let text = NSPasteboard.general.string(forType: .string) { insertText(text, replacementRange: missing) }
    }
}
