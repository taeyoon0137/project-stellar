import Foundation

/// AppKit은 선택을 UTF-16으로 전달합니다. 문서에 보낼 범위만 scalar 인덱스로 변환합니다.
struct CanvasTextBuffer {
    private(set) var contents: String
    private(set) var selection: NSRange
    private(set) var marked: NSRange?

    init?(_ text: String, caret: UInt32) {
        guard let offset = text.unicodeScalars.index(text.unicodeScalars.startIndex,
                                                      offsetBy: Int(caret), limitedBy: text.unicodeScalars.endIndex),
              let unit = offset.samePosition(in: text.utf16) else { return nil }
        contents = text
        selection = NSRange(location: text.utf16.distance(from: text.utf16.startIndex, to: unit), length: 0)
    }

    /// Surrogate pair 경계만 허용합니다. 조합 중의 scalar 경계는 grapheme 내부일 수 있습니다.
    static func bounds(_ range: NSRange, in text: String) -> Range<String.Index>? {
        let units = text.utf16
        guard range.location >= 0, range.length >= 0, range.location <= units.count,
              range.length <= units.count - range.location else { return nil }
        let a = units.index(units.startIndex, offsetBy: range.location)
        let b = units.index(a, offsetBy: range.length)
        guard let start = a.samePosition(in: text.unicodeScalars),
              let end = b.samePosition(in: text.unicodeScalars) else { return nil }
        return start..<end
    }

    func scalarRange(_ range: NSRange) -> (UInt32, UInt32)? {
        guard let bounds = Self.bounds(range, in: contents) else { return nil }
        let scalars = contents.unicodeScalars
        guard let start = UInt32(exactly: scalars.distance(from: scalars.startIndex, to: bounds.lowerBound)),
              let end = UInt32(exactly: scalars.distance(from: scalars.startIndex, to: bounds.upperBound)) else { return nil }
        return (start, end)
    }

    func target(_ replacement: NSRange) -> NSRange {
        replacement.location == NSNotFound ? (marked ?? selection) : replacement
    }

    mutating func replace(_ value: String, replacement: NSRange, markedSelection: NSRange?) -> Bool {
        let range = target(replacement)
        guard let bounds = Self.bounds(range, in: contents) else { return false }
        if let markedSelection, Self.bounds(markedSelection, in: value) == nil { return false }
        contents.unicodeScalars.replaceSubrange(bounds, with: value.unicodeScalars)
        if let markedSelection {
            marked = value.isEmpty ? nil : NSRange(location: range.location, length: value.utf16.count)
            selection = NSRange(location: range.location + markedSelection.location, length: markedSelection.length)
        } else {
            marked = nil
            selection = NSRange(location: range.location + value.utf16.count, length: 0)
        }
        return true
    }

    mutating func select(_ range: NSRange) {
        guard Self.bounds(range, in: contents) != nil else { return }
        selection = range
    }
}
