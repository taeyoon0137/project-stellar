import AppKit

/// 목록 행의 인라인 이름 변경 처리(디자인 가이드 6.2).
///
/// - 확정: Return, 또는 포커스가 다른 곳으로 이동할 때.
/// - 취소: Escape. 필드 값을 snapshot 값으로 되돌리고 command를 만들지 않습니다.
/// - 값이 바뀌지 않았으면 command를 만들지 않습니다(비교는 `commit`을 받는 쪽이 snapshot 기준으로 합니다).
///
/// 입력·IME·Undo는 AppKit field editor가 처리합니다(아키텍처 §11.2, XII.5.1).
@MainActor
final class SidebarInlineRename: NSObject, NSTextFieldDelegate {
    /// 편집 확정: (편집한 필드, 새 값).
    var commit: ((NSTextField, String) -> Void)?
    /// 취소 시 필드에 되돌릴 원래 값.
    var originalValue: ((NSTextField) -> String?)?
    /// 편집이 끝난 뒤(확정·취소 모두). 미뤄 둔 snapshot 반영에 씁니다.
    var didEnd: (() -> Void)?
    /// Return으로 확정하거나 Escape로 취소한 뒤 포커스를 돌려줄 목록.
    weak var outline: NSOutlineView?

    private weak var cancelledField: NSTextField?

    /// 목록 안 필드를 편집 중인지.
    static func isEditing(in outline: NSOutlineView) -> Bool {
        guard let editor = outline.window?.firstResponder as? NSText,
              let field = editor.delegate as? NSTextField else {
            return false
        }
        return field.isDescendant(of: outline)
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard commandSelector == #selector(NSResponder.cancelOperation(_:)),
              let field = control as? NSTextField else {
            return false
        }
        cancelledField = field
        field.abortEditing()
        if let original = originalValue?(field) {
            field.stringValue = original
        }
        if let outline {
            outline.window?.makeFirstResponder(outline)
        }
        cancelledField = nil
        didEnd?()
        return true
    }

    func controlTextDidEndEditing(_ notification: Notification) {
        guard let field = notification.object as? NSTextField else { return }
        if field !== cancelledField {
            commit?(field, field.stringValue)
        }
        // Return으로 끝낸 경우에만 목록으로 포커스를 돌려줍니다. 클릭·Tab으로 다른 곳에 간 포커스는 건드리지 않습니다
        // (디자인 가이드 8.2 "사용자 조작 없이 포커스를 옮기지 않습니다").
        let movement = notification.userInfo?["NSTextMovement"] as? Int
        if movement == NSTextMovement.return.rawValue, let outline {
            outline.window?.makeFirstResponder(outline)
        }
        didEnd?()
    }
}
