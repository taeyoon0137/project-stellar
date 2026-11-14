import AppKit

/// 네이티브 UI 텍스트 컨트롤의 undo/IME smoke를 headless로 검증합니다(XII.5.1).
///
/// 검사용 창(`CanvasWindowController`)을 화면에 표시하지 않고 만든 뒤 `NativeFieldsPanel`의 각 컨트롤에서
/// - IME 조합: `NSTextInputClient`의 `setMarkedText`로 한글 조합 중간("ㅎ"→"하"→"한")을 만들고
///   `insertText`로 확정, 빈 marked text로 조합 취소
/// - native undo/redo: 창 undo manager로 확정 입력을 되돌리고 다시 적용
/// 을 확인합니다. 한 줄 필드는 AppKit field editor(`NSTextView`)를 통해 같은 경로를 검사합니다.
/// 실제 입력 소스(시스템 한글 IME) 전환과 후보 창 표시는 검사하지 않습니다.
enum TextInputCheck {
    private static let noReplacement = NSRange(location: NSNotFound, length: 0)

    /// 검사 실패 메시지.
    private struct CheckFailure: Error {
        let message: String
    }

    static func run() -> Bool {
        func fail(_ message: String) -> Bool {
            FileHandle.standardError.write(Data("text-input 실패: \(message)\n".utf8))
            return false
        }

        // 창·field editor를 쓰려면 NSApplication이 필요합니다. Dock 아이콘은 띄우지 않습니다.
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)

        // 네이티브 필드 fixture 창(CanvasWindowController)을 화면에 표시하지 않고 사용합니다. 셸 프레임 창의 텍스트 편집
        // 중 Undo 경로는 `--tools-menus-ui-check`가, 실제 창 구성은 `--shell-integration-check`가 검사합니다.
        let controller = CanvasWindowController(runtimeHandle: nil)
        guard let window = controller.window, let panel = controller.fieldsPanel else {
            return fail("창에 NativeFieldsPanel이 없음")
        }
        window.isReleasedWhenClosed = false
        window.contentView?.layoutSubtreeIfNeeded()
        defer { window.close() }

        // 구조: 앱 UI 텍스트는 AppKit 네이티브 컨트롤입니다.
        guard type(of: panel.searchField) == NSSearchField.self else {
            return fail("검색 필드가 NSSearchField가 아님")
        }
        guard panel.layerNameField.isEditable, panel.generalField.isEditable else {
            return fail("레이어명·일반 필드가 편집 불가")
        }
        guard panel.notesTextView.allowsUndo, !panel.notesTextView.isRichText else {
            return fail("여러 줄 NSTextView가 plain text + undo 설정이 아님")
        }

        // 1) 여러 줄 NSTextView.
        guard window.makeFirstResponder(panel.notesTextView) else {
            return fail("NSTextView first responder 실패")
        }
        var report: [String] = []
        switch exercise(panel.notesTextView, label: "NSTextView") {
        case .success(let undone): report.append("NSTextView undo→'\(undone)'")
        case .failure(let failure): return fail(failure.message)
        }

        // 2) 한 줄 필드: field editor 경유.
        let fields: [(String, NSTextField)] = [
            ("레이어명 NSTextField", panel.layerNameField),
            ("검색 NSSearchField", panel.searchField),
            ("일반 NSTextField", panel.generalField),
        ]
        for (label, field) in fields {
            guard window.makeFirstResponder(field) else {
                return fail("\(label) first responder 실패")
            }
            guard let editor = field.currentEditor() as? NSTextView else {
                return fail("\(label) field editor 없음")
            }
            switch exercise(editor, label: label) {
            case .success(let undone): report.append("\(type(of: field)) undo→'\(undone)'")
            case .failure(let failure): return fail(failure.message)
            }
            // 편집 종료 시 field editor 내용이 필드 값으로 반영됩니다.
            guard window.makeFirstResponder(nil) else {
                return fail("\(label) 편집 종료 실패")
            }
            guard field.stringValue == "한글" else {
                return fail("\(label) 편집 종료 후 값이 '한글'이 아님: '\(field.stringValue)'")
            }
        }

        print("text-input ok: native undo/redo + IME marked text [\(report.joined(separator: ", "))]")
        return true
    }

    /// 한 text view에서 IME 조합·확정·취소와 undo/redo를 검사합니다. 성공 시 undo 직후 text를 반환합니다.
    private static func exercise(_ textView: NSTextView, label: String) -> Result<String, CheckFailure> {
        func failure(_ message: String) -> Result<String, CheckFailure> {
            .failure(CheckFailure(message: message))
        }

        textView.string = ""
        guard let undoManager = textView.undoManager else {
            return failure("\(label) undo manager 없음")
        }
        undoManager.removeAllActions()

        // 한글 조합: ㅎ → 하 → 한 (marked text, caret은 조합 끝).
        for (step, syllable) in ["ㅎ", "하", "한"].enumerated() {
            textView.setMarkedText(
                syllable,
                selectedRange: NSRange(location: (syllable as NSString).length, length: 0),
                replacementRange: noReplacement
            )
            guard textView.hasMarkedText() else {
                return failure("\(label) 조합 \(step) marked text 없음")
            }
            guard textView.markedRange() == NSRange(location: 0, length: 1) else {
                return failure("\(label) 조합 \(step) marked range 불일치: \(textView.markedRange())")
            }
        }
        textView.insertText("한", replacementRange: noReplacement)
        guard !textView.hasMarkedText(), textView.string == "한" else {
            return failure("\(label) 첫 음절 확정 실패: '\(textView.string)'")
        }

        // 두 번째 음절: ㄱ → 그 → 글 → 확정.
        for syllable in ["ㄱ", "그", "글"] {
            textView.setMarkedText(
                syllable,
                selectedRange: NSRange(location: 1, length: 0),
                replacementRange: noReplacement
            )
        }
        guard textView.markedRange() == NSRange(location: 1, length: 1) else {
            return failure("\(label) 둘째 음절 marked range 불일치: \(textView.markedRange())")
        }
        textView.insertText("글", replacementRange: noReplacement)
        guard !textView.hasMarkedText(), textView.string == "한글" else {
            return failure("\(label) 둘째 음절 확정 실패: '\(textView.string)'")
        }

        // 조합 취소: 빈 marked text는 조합을 지우고 확정 text를 바꾸지 않습니다.
        textView.setMarkedText(
            "ㅇ",
            selectedRange: NSRange(location: 1, length: 0),
            replacementRange: noReplacement
        )
        guard textView.hasMarkedText(), textView.string == "한글ㅇ" else {
            return failure("\(label) 취소 전 조합 상태 불일치: '\(textView.string)'")
        }
        textView.setMarkedText("", selectedRange: NSRange(location: 0, length: 0), replacementRange: noReplacement)
        guard !textView.hasMarkedText(), textView.string == "한글" else {
            return failure("\(label) 조합 취소 실패: '\(textView.string)'")
        }

        // native undo/redo: 확정 입력을 되돌리고 다시 적용합니다.
        textView.breakUndoCoalescing()
        guard undoManager.canUndo else {
            return failure("\(label) 확정 입력이 undo에 기록되지 않음")
        }
        undoManager.undo()
        let undone = textView.string
        guard undone != "한글", "한글".hasPrefix(undone) else {
            return failure("\(label) undo 결과 불일치: '\(undone)'")
        }
        guard undoManager.canRedo else {
            return failure("\(label) redo 불가")
        }
        undoManager.redo()
        guard textView.string == "한글" else {
            return failure("\(label) redo 결과 불일치: '\(textView.string)'")
        }
        return .success(undone)
    }
}
