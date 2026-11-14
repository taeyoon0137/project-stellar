import AppKit

/// 입력 서비스의 응답자입니다. 문서 글자와 caret 도형은 canvas의 공통 레이아웃이 제공합니다.
@MainActor
final class CanvasTextInputView: NSView, NSTextInputClient {
    typealias Replacement = (NSRange, String, String) -> Void
    private(set) var buffer: CanvasTextBuffer
    private var beforeMark: CanvasTextBuffer?
    private var originalRange: NSRange?
    var changed: ((CanvasTextBuffer) -> Void)?
    var confirmed: Replacement?
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

    func finish() {
        guard !finished else { return }
        finished = true
        if let original = beforeMark { buffer = original }
        originalRange = nil
        beforeMark = nil
        ended?()
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
        if text.isEmpty, let previous = beforeMark {
            buffer = previous
            beforeMark = nil
            originalRange = nil
            changed?(buffer)
            return
        }
        let previous = buffer
        let range = buffer.target(replacementRange)
        guard buffer.replace(text, replacement: range, markedSelection: selectedRange) else { return }
        if beforeMark == nil {
            beforeMark = previous
            originalRange = range
        }
        changed?(buffer)
    }

    func insertText(_ value: Any, replacementRange: NSRange) {
        guard !finished, let text = plain(value) else { return }
        let original = beforeMark ?? buffer
        let range = buffer.target(replacementRange)
        guard buffer.replace(text, replacement: range, markedSelection: nil) else { return }
        let target = originalRange ?? range
        beforeMark = nil
        originalRange = nil
        if original.contents.utf8.elementsEqual(buffer.contents.utf8) {
            changed?(buffer)
            return
        }
        // 입력기가 기존 조합 범위 외부를 재변환하면 원래 문서에 대한 최소 대체를 구합니다.
        let replacement = Self.replacement(from: original.contents, to: buffer.contents, preferred: target, inserted: text)
        confirmed?(replacement.0, replacement.1, original.contents)
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
