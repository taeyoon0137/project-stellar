import AppKit

/// 인스펙터 UI를 Stub 데이터로 화면 밖에서 만들어 검사합니다.
///
/// - 모든 Stub 장면을 Light·Dark 외형에서 대표 폭으로 배치하고 Auto Layout 모호성, 폭 넘침,
///   0 크기 컨트롤, 접근성 레이블 누락, 장면별 섹션 구성, key view 순서를 확인합니다.
/// - 수치 입력·단위 배율·stepper·switch·popup·segmented·색·disclosure·modifier 추가/활성/이동/삭제,
///   혼합 값 표시·상대 조정, 잠금·읽기 전용, 오래된 세대 무시, Escape 되돌리기를 확인합니다.
/// - 사용한 SF Symbols 이름이 모두 시스템에 있는지 확인합니다.
/// 대비 높이기·투명도 줄이기 설정 자체는 화면 밖에서 강제할 수 없어 검사하지 않습니다.
/// 통합 시 scripts/macos/smoke.sh의 `--inspector-ui-check` 경로에서 호출합니다.
@MainActor
enum InspectorUICheck {
    private struct Failure: Error {
        let message: String
    }

    /// `main.swift` 최상위 코드(동기, actor 미지정)에서 부를 수 있는 진입점. 메인 스레드에서만 호출합니다.
    nonisolated static func run() -> Bool {
        MainActor.assumeIsolated { runOnMainActor() }
    }

    private static func runOnMainActor() -> Bool {
        let application = NSApplication.shared
        application.setActivationPolicy(.accessory)
        var report: [String] = []
        do {
            try checkSymbols(&report)
            // 시간 측정은 다른 검사가 창을 많이 만들기 전에 합니다.
            try checkTiming(&report)
            try checkScenarios(&report)
            try checkInteractions(&report)
            print("inspector-ui ok: \(report.joined(separator: ", "))")
            return true
        } catch {
            let message = (error as? Failure)?.message ?? "\(error)"
            // 실패해도 그때까지 잰 값은 남깁니다(측정 보고).
            FileHandle.standardError.write(Data("inspector-ui 실패: \(message)\n  측정: \(report.joined(separator: ", "))\n".utf8))
            return false
        }
    }

    // MARK: - 검사 단계

    private static func checkSymbols(_ report: inout [String]) throws {
        let missing = InspectorSymbols.all.filter { NSImage(systemSymbolName: $0, accessibilityDescription: nil) == nil }
        try require(missing.isEmpty, "SF Symbol 없음: \(missing.joined(separator: ", "))")
        report.append("\(InspectorSymbols.all.count) symbols")
    }

    private static func checkScenarios(_ report: inout [String]) throws {
        var checkedControls = 0
        var destructiveFound: Set<String> = []
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            for scenario in InspectorStubDataSource.Scenario.allCases {
                let harness = Harness(scenario: scenario, appearance: appearance)
                defer { harness.close() }
                let label = "\(scenario.rawValue)/\(appearance == .aqua ? "light" : "dark")"
                try checkLayout(harness, label: label)
                checkedControls += try checkAccessibility(harness, label: label)
                try checkStructure(harness, scenario: scenario, label: label)
                destructiveFound.formUnion(try checkDestructiveButtons(harness, label: label))
                try checkKeyViewChain(harness, label: label)
            }
        }
        try require(destructiveFound == destructiveButtonIDs,
                    "파괴 동작 버튼 누락: \(destructiveButtonIDs.subtracting(destructiveFound).sorted())")
        report.append("\(InspectorStubDataSource.Scenario.allCases.count) scenarios × light/dark")
        report.append("\(checkedControls) labeled controls")
        report.append("\(destructiveFound.count) destructive buttons")
    }

    private static func checkLayout(_ harness: Harness, label: String,
                                    width: CGFloat = InspectorMetrics.typicalWidth) throws {
        let views = visibleViews(in: harness.controller.view)
        let ambiguous = views.filter(\.hasAmbiguousLayout)
        try require(ambiguous.isEmpty, "\(label): 모호한 Auto Layout \(ambiguous.map(describe).joined(separator: ", "))")

        // 내용이 pane 폭을 밀어내지 않아야 합니다.
        let root = harness.controller.view
        try require(abs(root.bounds.width - width) < 0.5,
                    "\(label): 내용이 인스펙터 폭을 \(root.bounds.width)로 넓힘")
        // 패널 여백 안에 있어야 합니다(스크롤러 같은 시스템 뷰는 제외).
        let inset = InspectorMetrics.panelInset
        for view in views where view is InspectorRowView || (view is NSControl && !(view is NSScroller)) {
            // Auto Layout이 배치하는 정렬 사각형 기준(텍스트 필드의 그리기 여백 제외).
            guard let superview = view.superview else { continue }
            let frame = superview.convert(view.alignmentRect(forFrame: view.frame), to: root)
            try require(frame.width >= 1 && frame.height >= 1, "\(label): 크기 0 \(describe(view))")
            try require(frame.minX >= inset - 0.5 && frame.maxX <= root.bounds.width - inset + 0.5,
                        "\(label): 패널 여백 밖으로 넘침 \(describe(view)) \(frame)")
        }
    }

    /// 조작 가능한 컨트롤의 접근성 레이블을 확인하고 개수를 돌려줍니다.
    private static func checkAccessibility(_ harness: Harness, label: String) throws -> Int {
        try require(harness.controller.view.accessibilityLabel() == "Inspector", "\(label): 인스펙터 레이블 없음")
        var count = 0
        for view in visibleViews(in: harness.controller.view) {
            guard let control = view as? NSControl, isInteractive(control) else { continue }
            let accessibility = control.accessibilityLabel() ?? ""
            let title = (control as? NSButton)?.title ?? ""
            try require(!accessibility.isEmpty || !title.isEmpty, "\(label): 접근성 레이블 없음 \(describe(control))")
            count += 1
        }
        for section in harness.controller.sectionViews {
            let disclosure = section.disclosureButton.accessibilityLabel() ?? ""
            try require(!disclosure.isEmpty, "\(label): 섹션 \(section.sectionID.rawValue) disclosure 레이블 없음")
        }
        return count
    }

    private static func checkStructure(
        _ harness: Harness, scenario: InspectorStubDataSource.Scenario, label: String
    ) throws {
        let controller = harness.controller
        let ids = controller.sectionViews.map(\.sectionID)
        let expected: [InspectorSectionID]
        switch scenario {
        case .noSelection, .loading, .failed: expected = []
        case .frame, .locked: expected = [.layer, .frameLayout, .frameAppearance, .modifiers]
        case .rectangle, .ellipse, .line, .polygon, .readOnly: expected = [.layer, .shape, .modifiers]
        case .path: expected = [.layer, .path, .modifiers]
        case .text: expected = [.layer, .textContent, .textStyle, .paragraph, .modifiers]
        case .image: expected = [.layer, .image, .modifiers]
        case .componentInstance: expected = [.layer, .component, .componentProperties, .componentSlots, .modifiers]
        case .fragment: expected = [.layer, .fragment]
        case .multipleFrames: expected = [.layer, .frameLayout, .frameAppearance, .modifiers]
        case .mixedKinds: expected = [.layer, .modifiers]
        }
        try require(ids == expected, "\(label): 섹션 \(ids.map(\.rawValue)) ≠ \(expected.map(\.rawValue))")

        switch scenario {
        case .noSelection, .loading, .failed:
            let retry = controller.control(identifier: "state.retry") as? NSButton
            try require((retry?.isHiddenOrHasHiddenAncestor ?? true) == (scenario != .failed),
                        "\(label): 다시 시도 버튼 표시 상태 불일치")
        case .multipleFrames:
            let width = controller.control(identifier: "frame.size.width") as? NSTextField
            try require(width?.stringValue == "" && width?.placeholderString == InspectorFieldController.mixedText,
                        "\(label): 혼합 너비가 Mixed로 표시되지 않음")
            let name = controller.control(identifier: InspectorPropertyID.layerName.path) as? NSTextField
            try require(name?.placeholderString == InspectorFieldController.mixedText, "\(label): 혼합 이름 표시 없음")
            let color = controller.control(identifier: "frame.background.color")
            try require(color?.accessibilityValueDescription() == InspectorFieldController.mixedText,
                        "\(label): 혼합 색 접근성 값 없음")
            let height = controller.control(identifier: "frame.size.height") as? NSTextField
            try require(height?.stringValue == "200", "\(label): 공통 높이 값이 보이지 않음")
        case .locked:
            try require(!isEnabled(controller, "frame.size.width"), "\(label): 잠금인데 너비 편집 가능")
            try require(isEnabled(controller, InspectorPropertyID.layerLocked.path), "\(label): 잠금 해제 불가")
            let unlock = controller.control(identifier: "banner.unlock")
            try require(unlock?.isHiddenOrHasHiddenAncestor == false, "\(label): 잠금 안내 없음")
        case .readOnly:
            for view in visibleViews(in: controller.view) {
                guard let control = view as? NSControl, let id = control.identifier?.rawValue,
                      id.contains("."), !id.hasPrefix("section."), id != "modifiers.table" else { continue }
                try require(!control.isEnabled, "\(label): 읽기 전용인데 \(id) 편집 가능")
            }
        case .image:
            let issue = visibleViews(in: controller.view).contains {
                ($0.accessibilityLabel() ?? "").hasPrefix("Issue:")
            }
            try require(issue, "\(label): 누락 asset 진단 표시 없음")
        default:
            break
        }
    }

    /// 파괴 동작 버튼(Path로 변환, Detach, modifier 삭제)만 `hasDestructiveAction`이고 기본 버튼이 아님(가이드 5.1).
    static let destructiveButtonIDs: Set<String> = ["shape.convertToPath", "component.detach", "modifiers.remove"]

    /// 찾은 파괴 동작 버튼 id를 돌려줍니다(모든 장면을 합쳐 빠진 것이 없는지 확인).
    private static func checkDestructiveButtons(_ harness: Harness, label: String) throws -> Set<String> {
        var found: Set<String> = []
        for view in visibleViews(in: harness.controller.view) {
            guard let button = view as? NSButton, let id = button.identifier?.rawValue else { continue }
            try require(button.hasDestructiveAction == destructiveButtonIDs.contains(id),
                        "\(label): \(id) 파괴 동작 표시 \(button.hasDestructiveAction)")
            if button.hasDestructiveAction {
                try require(button.keyEquivalent != "\r", "\(label): 파괴 동작 \(id)이 기본 버튼임")
                found.insert(id)
            }
        }
        return found
    }

    private static func checkKeyViewChain(_ harness: Harness, label: String) throws {
        let chain = harness.controller.keyViewChain
        if harness.controller.appliedSnapshot?.state == .ready {
            try require(!chain.isEmpty, "\(label): key view 순서 비어 있음")
        }
        for (current, next) in zip(chain, chain.dropFirst()) {
            try require(current.nextKeyView === next, "\(label): key view 연결 끊김 \(describe(current))")
            try require(!current.isHiddenOrHasHiddenAncestor, "\(label): 숨은 뷰가 key view 순서에 있음")
        }
    }

    private static func checkInteractions(_ report: inout [String]) throws {
        try checkFrameEditing()
        try checkContinuousGestures()
        try checkNumberRounding()
        try checkModifierStack()
        try checkMixedAndText()
        try checkLockFlow()
        try checkPageCache()
        try checkResize()
        try checkSelectionChangeCommitsEdit()
        report.append("interactions")
        try checkPrewarmAndStandby(&report)
    }

    /// 수치·switch·popup·disclosure·Escape·세대 순서.
    private static func checkFrameEditing() throws {
        let harness = Harness(scenario: .frame, appearance: .aqua)
        defer { harness.close() }
        let controller = harness.controller
        let source = harness.source

        let width = try control(controller, "frame.size.width", as: NSTextField.self)
        try require(width.stringValue == "320", "초기 너비 '\(width.stringValue)'")
        width.stringValue = "480"
        width.sendAction(width.action, to: width.target)
        try require(source.receivedCommands.last == .setValue(InspectorPropertyID("frame.size.width", coreKey: 0x0101),
                                                              .number(480), phase: .commit),
                    "너비 확정 명령 불일치: \(String(describing: source.receivedCommands.last))")
        harness.flush()
        try require(width.stringValue == "480", "너비 갱신 실패 '\(width.stringValue)'")

        // 음수 크기는 규격 하한으로 clamp(코어 Size2 비음수 정책과 같은 방향).
        width.stringValue = "-5"
        width.sendAction(width.action, to: width.target)
        harness.flush()
        try require(width.stringValue == "0", "음수 너비가 0으로 clamp되지 않음 '\(width.stringValue)'")

        // Escape: 편집 중 입력을 되돌리고 명령을 보내지 않습니다.
        let commandCount = source.receivedCommands.count
        try require(harness.window.makeFirstResponder(width), "너비 first responder 실패")
        guard let editor = width.currentEditor() as? NSTextView else { throw Failure(message: "field editor 없음") }
        editor.string = "999"
        editor.doCommand(by: #selector(NSResponder.cancelOperation(_:)))
        try require(editor.string == "0", "Escape 되돌리기 실패 '\(editor.string)'")
        // 화살표 키: 한 단계 증가(연속 입력).
        editor.doCommand(by: #selector(NSResponder.moveUp(_:)))
        try require(source.receivedCommands.count == commandCount + 1 &&
                    source.receivedCommands.last == .setValue(try propertyID(harness, "frame.size.width"), .number(1), phase: .continuous),
                    "화살표 키 증가 명령 불일치: \(String(describing: source.receivedCommands.last))")
        // 편집이 끝나면 값이 그대로여도 연속 조작 끝(`.ended`)을 한 번 보냅니다(별도 commit 없음).
        _ = harness.window.makeFirstResponder(nil)
        try require(source.receivedCommands.count == commandCount + 2 &&
                    source.receivedCommands.last == .setValue(try propertyID(harness, "frame.size.width"), .number(1), phase: .ended),
                    "화살표 키 조정 종료 명령 불일치: \(String(describing: source.receivedCommands.last))")
        harness.flush()
        try require(width.stringValue == "1", "화살표 키 후 너비 '\(width.stringValue)'")

        // stepper(회전).
        let stepper = try control(controller, "frame.transform.rotation.stepper", as: NSStepper.self)
        stepper.doubleValue += stepper.increment
        stepper.sendAction(stepper.action, to: stepper.target)
        harness.flush()
        let rotation = try control(controller, "frame.transform.rotation", as: NSTextField.self)
        try require(rotation.stringValue == "1", "stepper 회전 '\(rotation.stringValue)'")

        // switch → 의존 색 필드 비활성.
        let background = try control(controller, "frame.background.enabled", as: NSSwitch.self)
        background.state = .off
        background.sendAction(background.action, to: background.target)
        harness.flush()
        try require(!isEnabled(controller, "frame.background.color"), "배경을 끄면 색 필드가 비활성이어야 함")

        // popup.
        let clip = try control(controller, "frame.clipping", as: NSPopUpButton.self)
        clip.selectItem(withTitle: "None")
        clip.sendAction(clip.action, to: clip.target)
        harness.flush()
        try require(source.snapshot.field(InspectorPropertyID("frame.clipping", coreKey: 0x0103))?.value == .choice("none"),
                    "clip popup 반영 실패")
        let layout = try control(controller, "frame.layoutMode", as: NSPopUpButton.self)
        try require(layout.itemTitles == ["None", "Vertical Stack"], "layout 동적 선택지 '\(layout.itemTitles)'")

        // 색 well(문서 색).
        let border = try control(controller, "frame.border.color", as: NSColorWell.self)
        border.color = NSColor(srgbRed: 1, green: 0, blue: 0, alpha: 1)
        border.sendAction(border.action, to: border.target)
        guard case .setValue(_, .color(let color), .continuous)? = source.receivedCommands.last,
              color.space == .srgb, abs(color.red - 1) < 0.001, abs(color.green) < 0.001 else {
            throw Failure(message: "색 명령 불일치: \(String(describing: source.receivedCommands.last))")
        }

        // disclosure: 접으면 본문과 key view 순서에서 빠집니다.
        guard let appearance = controller.sectionViews.first(where: { $0.sectionID == .frameAppearance }) else {
            throw Failure(message: "Appearance 섹션 없음")
        }
        appearance.disclosureButton.performClick(nil)
        try require(!appearance.isExpanded, "섹션 접기 실패")
        try require(border.isHiddenOrHasHiddenAncestor, "접은 섹션 본문이 보임")
        try require(!controller.keyViewChain.contains { $0 === border }, "접은 섹션 컨트롤이 key view 순서에 있음")
        appearance.disclosureButton.performClick(nil)
        try require(appearance.isExpanded && !border.isHiddenOrHasHiddenAncestor, "섹션 펼치기 실패")

        // 오래된 세대는 무시합니다.
        var stale = source.snapshot
        stale.generation = 1
        stale.sections = []
        controller.receive(stale)
        harness.flush()
        try require(controller.sectionViews.count == 4, "오래된 세대 snapshot이 반영됨")

        // 선택 변경: 구성이 다른 snapshot은 섹션을 다시 만듭니다.
        source.select(.text)
        harness.flush()
        try require(controller.sectionViews.map(\.sectionID) == [.layer, .textContent, .textStyle, .paragraph, .modifiers],
                    "선택 변경 후 섹션 불일치")
        let alignment = try control(controller, "text.paragraph.alignment", as: NSSegmentedControl.self)
        alignment.selectedSegment = 1
        alignment.sendAction(alignment.action, to: alignment.target)
        harness.flush()
        try require(source.snapshot.field(InspectorPropertyID("text.paragraph.alignment"))?.value == .choice("center"),
                    "segmented 반영 실패")
        let lineMode = try control(controller, "text.paragraph.lineHeight.mode", as: NSPopUpButton.self)
        lineMode.selectItem(withTitle: "Auto")
        lineMode.sendAction(lineMode.action, to: lineMode.target)
        harness.flush()
        try require(!isEnabled(controller, "text.paragraph.lineHeight.value"), "Auto 줄 높이 값이 편집 가능")
    }

    /// 연속 조작은 `.continuous` 뒤에 같은 속성의 `.ended` 하나로 끝납니다(아키텍처 §23.3).
    /// 화살표 키는 key up·편집 끝·Return, stepper·색은 마우스 추적이 끝난 다음 런 루프에서 끝납니다.
    private static func checkContinuousGestures() throws {
        let harness = Harness(scenario: .frame, appearance: .aqua)
        defer { harness.close() }
        let source = harness.source
        let widthID = try propertyID(harness, "frame.size.width")
        let width = try control(harness.controller, "frame.size.width", as: NSTextField.self)
        func commands(since start: Int) -> [InspectorCommand] { Array(source.receivedCommands[start...]) }

        // 화살표 키 두 번 → key up에서 끝(값 322).
        var start = source.receivedCommands.count
        try require(harness.window.makeFirstResponder(width), "너비 first responder 실패")
        guard let editor = width.currentEditor() as? NSTextView else { throw Failure(message: "field editor 없음") }
        editor.doCommand(by: #selector(NSResponder.moveUp(_:)))
        editor.doCommand(by: #selector(NSResponder.moveUp(_:)))
        try sendArrowKeyUp(harness)
        try require(commands(since: start) == [
            .setValue(widthID, .number(321), phase: .continuous),
            .setValue(widthID, .number(322), phase: .continuous),
            .setValue(widthID, .number(322), phase: .ended),
        ], "화살표 키 key up 종료 불일치: \(commands(since: start))")

        // 다시 조정한 뒤 값을 입력하고 확정 → 확정 값이 같은 조작의 최종 값(`.ended`)이고 별도 commit은 없음.
        start = source.receivedCommands.count
        editor.doCommand(by: #selector(NSResponder.moveDown(_:)))
        editor.string = "400"
        _ = harness.window.makeFirstResponder(nil)
        try require(commands(since: start) == [
            .setValue(widthID, .number(321), phase: .continuous),
            .setValue(widthID, .number(400), phase: .ended),
        ], "화살표 키 뒤 확정 종료 불일치: \(commands(since: start))")
        harness.flush()

        // stepper 자동 반복 두 번 → 추적이 끝난 다음 런 루프에서 `.ended`.
        let rotationID = try propertyID(harness, "frame.transform.rotation")
        let stepper = try control(harness.controller, "frame.transform.rotation.stepper", as: NSStepper.self)
        start = source.receivedCommands.count
        for _ in 0..<2 {
            stepper.doubleValue += stepper.increment
            stepper.sendAction(stepper.action, to: stepper.target)
        }
        try require(commands(since: start).allSatisfy {
            if case .setValue(_, _, .continuous) = $0 { return true } else { return false }
        }, "stepper 반복 중 종료 명령이 먼저 나감: \(commands(since: start))")
        try pumpUntil("stepper 종료") { source.receivedCommands.count == start + 3 }
        try require(commands(since: start) == [
            .setValue(rotationID, .number(1), phase: .continuous),
            .setValue(rotationID, .number(2), phase: .continuous),
            .setValue(rotationID, .number(2), phase: .ended),
        ], "stepper 종료 불일치: \(commands(since: start))")
        harness.flush()

        // 색 well(색 패널 변경) → 다음 런 루프에서 `.ended`.
        let borderID = try propertyID(harness, "frame.border.color")
        let border = try control(harness.controller, "frame.border.color", as: NSColorWell.self)
        start = source.receivedCommands.count
        border.color = NSColor(srgbRed: 0, green: 0, blue: 1, alpha: 1)
        border.sendAction(border.action, to: border.target)
        try pumpUntil("색 종료") { source.receivedCommands.count == start + 2 }
        guard case .setValue(borderID, .color(let first), .continuous) = source.receivedCommands[start],
              source.receivedCommands[start + 1] == .setValue(borderID, .color(first), phase: .ended) else {
            throw Failure(message: "색 종료 불일치: \(commands(since: start))")
        }
        harness.flush()

        // 선택이 바뀌면 진행 중인 stepper 조작을 이전 세대로 끝냅니다.
        start = source.receivedCommands.count
        let previousGeneration = source.snapshot.generation
        stepper.doubleValue += stepper.increment
        stepper.sendAction(stepper.action, to: stepper.target)
        harness.flush()
        let continuousGeneration = harness.controller.appliedSnapshot?.generation ?? 0
        source.select(.text)
        harness.flush()
        try require(source.receivedCommands.count >= start + 2
                    && source.receivedCommands[start + 1] == .setValue(rotationID, .number(3), phase: .ended),
                    "선택 변경 시 stepper 조작이 끝나지 않음: \(commands(since: start))")
        try require(source.receivedGenerations[start + 1] == continuousGeneration
                    && continuousGeneration > previousGeneration,
                    "조작 종료가 이전 선택 세대가 아님: \(source.receivedGenerations[start + 1])")

        // 혼합 값 상대 조정 → key up에서 `delta: 0` 종료.
        let mixed = Harness(scenario: .multipleFrames, appearance: .aqua)
        defer { mixed.close() }
        let mixedWidth = try control(mixed.controller, "frame.size.width", as: NSTextField.self)
        try require(mixed.window.makeFirstResponder(mixedWidth), "혼합 너비 first responder 실패")
        guard let mixedEditor = mixedWidth.currentEditor() as? NSTextView else {
            throw Failure(message: "혼합 field editor 없음")
        }
        start = mixed.source.receivedCommands.count
        mixedEditor.doCommand(by: #selector(NSResponder.moveDown(_:)))
        try sendArrowKeyUp(mixed)
        try require(Array(mixed.source.receivedCommands[start...]) == [
            .adjustNumber(widthID, delta: -1, phase: .continuous),
            .adjustNumber(widthID, delta: 0, phase: .ended),
        ], "혼합 상대 조정 종료 불일치: \(Array(mixed.source.receivedCommands[start...]))")
    }

    /// 화살표 키 key up을 앱 이벤트 경로(`sendEvent`)로 보냅니다.
    private static func sendArrowKeyUp(_ harness: Harness) throws {
        let up = String(UnicodeScalar(NSUpArrowFunctionKey)!)
        guard let event = NSEvent.keyEvent(
            with: .keyUp, location: .zero, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
            windowNumber: harness.window.windowNumber, context: nil, characters: up,
            charactersIgnoringModifiers: up, isARepeat: false, keyCode: 126
        ) else { throw Failure(message: "key up 이벤트 생성 실패") }
        NSApplication.shared.sendEvent(event)
    }

    /// 기본 모드 런 루프를 조건이 맞을 때까지(최대 1초) 돌립니다.
    private static func pumpUntil(_ label: String, _ condition: () -> Bool) throws {
        let deadline = Date().addingTimeInterval(1)
        while !condition(), Date() < deadline {
            _ = RunLoop.current.run(mode: .default, before: Date().addingTimeInterval(0.01))
        }
        try require(condition(), "\(label): 런 루프 대기 시간 초과")
    }

    /// 입력값을 표시 자릿수로 반올림해 보냅니다. 정수 필드(변 수)는 정수만, 배율 필드는 표시 자릿수 기준.
    private static func checkNumberRounding() throws {
        let harness = Harness(scenario: .polygon, appearance: .aqua)
        defer { harness.close() }
        let source = harness.source
        let sides = try control(harness.controller, "shape.polygon.sides", as: NSTextField.self)
        let sidesID = try propertyID(harness, "shape.polygon.sides")
        for (typed, expected, shown) in [("6.7", 7.0, "7"), ("4.5", 5.0, "5"), ("2.2", 3.0, "3")] {
            sides.stringValue = typed
            sides.sendAction(sides.action, to: sides.target)
            try require(source.receivedCommands.last == .setValue(sidesID, .number(expected), phase: .commit),
                        "변 수 '\(typed)' 반올림 명령 불일치: \(String(describing: source.receivedCommands.last))")
            harness.flush()
            try require(sides.stringValue == shown, "변 수 '\(typed)' 표시 '\(sides.stringValue)' ≠ '\(shown)'")
        }

        source.select(.image)
        harness.flush()
        let cropX = try control(harness.controller, "image.crop.x", as: NSTextField.self)
        cropX.stringValue = "12.34"
        cropX.sendAction(cropX.action, to: cropX.target)
        guard case .setValue(_, .number(let crop), .commit)? = source.receivedCommands.last, abs(crop - 0.123) < 1e-9
        else { throw Failure(message: "crop 반올림 명령 불일치: \(String(describing: source.receivedCommands.last))") }
        harness.flush()
        try require(cropX.stringValue == "12.3", "crop 표시 '\(cropX.stringValue)'")
    }

    /// modifier 목록: 선택·단위 배율·추가·활성·이동·삭제.
    private static func checkModifierStack() throws {
        let harness = Harness(scenario: .frame, appearance: .darkAqua)
        defer { harness.close() }
        let controller = harness.controller
        let source = harness.source
        guard let stack = controller.modifierStackController else { throw Failure(message: "modifier 섹션 없음") }
        try require(stack.tableView.numberOfRows == 2, "modifier 행 수 \(stack.tableView.numberOfRows)")

        // 불투명도 선택 → 상세 행(0.95 → 95%).
        stack.tableView.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
        guard let opacityID = stack.selectedID else { throw Failure(message: "modifier 선택 실패") }
        // 처음 보는 파라미터 구성은 화면 갱신을 막지 않도록 나눠 만든 뒤 보입니다.
        try require(stack.visibleDetail == nil, "처음 보는 파라미터 구성을 선택 경로에서 바로 만듦")
        harness.flush()
        let opacity = try control(controller, "modifier.\(opacityID).opacity.value", as: NSTextField.self)
        try require(opacity.stringValue == "95", "불투명도 표시 '\(opacity.stringValue)'")
        opacity.stringValue = "50"
        opacity.sendAction(opacity.action, to: opacity.target)
        try require(source.receivedCommands.last == .setValue(try propertyID(harness, "modifier.\(opacityID).opacity.value"), .number(0.5), phase: .commit),
                    "불투명도 배율 변환 실패: \(String(describing: source.receivedCommands.last))")
        harness.flush()

        // 추가.
        guard let menu = stack.addButton.menu,
              let fillIndex = menu.items.firstIndex(where: { $0.title == InspectorModifierKind.fill.title }) else {
            throw Failure(message: "추가 메뉴에 Fill 없음")
        }
        menu.performActionForItem(at: fillIndex)
        harness.flush()
        try require(stack.tableView.numberOfRows == 3, "modifier 추가 실패")

        // 활성 전환(행 체크박스).
        guard let cell = stack.tableView.view(atColumn: 0, row: 2, makeIfNecessary: true) as? InspectorModifierCellView
        else { throw Failure(message: "modifier 행 뷰 없음") }
        try require(!(cell.checkbox.accessibilityLabel() ?? "").isEmpty, "체크박스 레이블 없음")
        cell.checkbox.state = .off
        cell.checkbox.sendAction(cell.checkbox.action, to: cell.checkbox.target)
        harness.flush()
        try require(stack.stack.items[2].isEnabled == false, "modifier 비활성 전환 실패")

        // 이동(Command-Option-위).
        stack.tableView.selectRowIndexes(IndexSet(integer: 2), byExtendingSelection: false)
        let movedID = stack.stack.items[2].id
        guard let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [.command, .option], timestamp: 0,
            windowNumber: harness.window.windowNumber, context: nil,
            characters: String(UnicodeScalar(NSUpArrowFunctionKey)!), charactersIgnoringModifiers:
                String(UnicodeScalar(NSUpArrowFunctionKey)!), isARepeat: false, keyCode: 126
        ) else { throw Failure(message: "키 이벤트 생성 실패") }
        stack.tableView.keyDown(with: event)
        harness.flush()
        try require(stack.stack.items[1].id == movedID, "modifier 이동 실패")

        // 삭제.
        stack.tableView.selectRowIndexes(IndexSet(integer: 1), byExtendingSelection: false)
        stack.removeButton.performClick(nil)
        harness.flush()
        try require(stack.tableView.numberOfRows == 2 && !stack.stack.items.contains { $0.id == movedID },
                    "modifier 삭제 실패")
    }

    /// 혼합 값 상대 조정, 다른 종류 다중 선택, Fragment·변환 명령.
    private static func checkMixedAndText() throws {
        let harness = Harness(scenario: .multipleFrames, appearance: .aqua)
        defer { harness.close() }
        let width = try control(harness.controller, "frame.size.width", as: NSTextField.self)
        try require(harness.window.makeFirstResponder(width), "혼합 너비 first responder 실패")
        guard let editor = width.currentEditor() as? NSTextView else { throw Failure(message: "field editor 없음") }
        editor.doCommand(by: #selector(NSResponder.moveUpAndModifySelection(_:)))
        try require(harness.source.receivedCommands.last ==
                        .adjustNumber(try propertyID(harness, "frame.size.width"), delta: 10, phase: .continuous),
                    "혼합 값 Shift-화살표 상대 조정 불일치: \(String(describing: harness.source.receivedCommands.last))")
        _ = harness.window.makeFirstResponder(nil)
        try require(harness.source.receivedCommands.last ==
                        .adjustNumber(try propertyID(harness, "frame.size.width"), delta: 0, phase: .ended),
                    "혼합 값 상대 조정 종료 불일치: \(String(describing: harness.source.receivedCommands.last))")
        harness.flush()

        // Stub 한계(계획 1.2 규칙 6): 혼합 값 상대 조정은 node별 값이 없어 적용하지 않고,
        // Value Binding 요청은 snapshot을 바꾸지 않습니다.
        try require(harness.source.snapshot.field(try propertyID(harness, "frame.size.width"))?.value == .mixed,
                    "Stub이 혼합 값 상대 조정을 적용함(한계와 다름)")
        let generation = harness.source.snapshot.generation
        harness.source.submit(.showBinding(try propertyID(harness, "frame.background.color")), basedOn: generation)
        try require(harness.source.snapshot.generation == generation, "Stub이 Value Binding 요청에 snapshot을 발행함")

        harness.source.select(.fragment)
        harness.flush()
        let convert = try control(harness.controller, "fragment.convertToFrame", as: NSButton.self)
        convert.performClick(nil)
        harness.flush()
        try require(harness.controller.sectionViews.map(\.sectionID).contains(.frameLayout), "Frame으로 변환 실패")
    }

    /// 잠금 → 안내 → 잠금 해제.
    private static func checkLockFlow() throws {
        let harness = Harness(scenario: .frame, appearance: .aqua)
        defer { harness.close() }
        let lock = try control(harness.controller, InspectorPropertyID.layerLocked.path, as: NSSwitch.self)
        lock.state = .on
        lock.sendAction(lock.action, to: lock.target)
        harness.flush()
        try require(harness.controller.appliedSnapshot?.editability == .locked, "잠금 상태 반영 실패")
        try require(!isEnabled(harness.controller, "frame.size.width"), "잠금 후 너비 편집 가능")
        try require(harness.controller.modifierStackController?.removeButton.isEnabled == false,
                    "잠금 후 modifier 삭제 가능")
        let unlock = try control(harness.controller, "banner.unlock", as: NSButton.self)
        unlock.performClick(nil)
        harness.flush()
        try require(harness.controller.appliedSnapshot?.editability == .editable, "잠금 해제 실패")
        try require(isEnabled(harness.controller, "frame.size.width"), "잠금 해제 후 너비 편집 불가")
    }

    /// 편집 중 선택이 바뀌면 입력을 이전 세대로 확정합니다(새 선택에 잘못 적용 방지). 사용자 조작 없이 바뀐 선택이므로
    /// 포커스는 옮기지 않고(가이드 8.2), 같은 필드에 새 선택의 값을 보입니다.
    private static func checkSelectionChangeCommitsEdit() throws {
        let harness = Harness(scenario: .frame, appearance: .aqua)
        defer { harness.close() }
        let source = harness.source
        let width = try control(harness.controller, "frame.size.width", as: NSTextField.self)
        let widthID = try propertyID(harness, "frame.size.width")
        try require(harness.window.makeFirstResponder(width), "너비 first responder 실패")
        guard let editor = width.currentEditor() as? NSTextView else { throw Failure(message: "field editor 없음") }
        editor.string = "777"
        let previousGeneration = harness.controller.appliedSnapshot?.generation ?? 0
        let previousToken = source.selectionToken
        source.select(.frame) // 같은 구성의 다른 선택(선택 토큰 변경)
        harness.flush()
        guard let index = source.receivedCommands.lastIndex(where: {
            if case .setValue(let id, _, _) = $0 { return id == widthID }
            return false
        }) else { throw Failure(message: "선택 변경 시 진행 중 편집이 확정되지 않음") }
        try require(source.receivedCommands[index] == .setValue(widthID, .number(777), phase: .commit),
                    "확정 값 불일치: \(source.receivedCommands[index])")
        try require(source.receivedGenerations[index] == previousGeneration,
                    "확정 명령이 이전 세대가 아님: \(source.receivedGenerations[index]) ≠ \(previousGeneration)")
        // 프로토콜 7.2: 이전 세대 명령은 이전 선택에 적용되고, 새 선택 값은 바뀌지 않습니다.
        try require(source.appliedSelectionTokens[index] == previousToken,
                    "확정 명령이 이전 선택에 적용되지 않음: \(String(describing: source.appliedSelectionTokens[index]))")
        try require(source.storedValue("frame.size.width", selectionToken: previousToken) == .number(777),
                    "이전 선택 너비가 777이 아님")
        try require(source.snapshot.field(widthID)?.value == .number(320),
                    "이전 세대 명령이 새 선택에 적용됨: \(String(describing: source.snapshot.field(widthID)?.value))")
        harness.flush()
        guard let kept = width.currentEditor() as? NSTextView else {
            throw Failure(message: "선택 변경으로 포커스·편집이 끝남(사용자 조작 없이 포커스를 옮기면 안 됨)")
        }
        try require(kept.string == "320", "편집 중인 필드에 새 선택 너비가 보이지 않음 '\(kept.string)'")
        // 이어서 편집을 끝내도 새 선택에 명령을 보내지 않습니다(값이 그대로).
        let afterSelection = source.receivedCommands.count
        _ = harness.window.makeFirstResponder(nil)
        try require(source.receivedCommands.count == afterSelection,
                    "새 선택 값 그대로 편집을 끝냈는데 명령이 나감: \(source.receivedCommands.suffix(1))")
        try require(width.stringValue == "320", "새 선택 너비 표시 '\(width.stringValue)'")

        // 알 수 없는(만료된) 세대 명령은 거부하고 현재 snapshot을 다시 보냅니다.
        let generation = source.snapshot.generation
        source.submit(.setValue(widthID, .number(5), phase: .commit), basedOn: 0)
        try require(source.appliedSelectionTokens.last == .some(nil), "만료 세대 명령이 거부되지 않음")
        try require(source.snapshot.generation == generation + 1, "거부 후 현재 snapshot을 다시 보내지 않음")
        try require(source.snapshot.field(widthID)?.value == .number(320), "거부한 명령이 적용됨")
        harness.flush()

        // 다른 구성으로 바뀌는 선택: 같은 속성 컨트롤(레이어 이름)이 새 페이지에 있으면 포커스가 그 컨트롤로 이어집니다.
        let nameID = try propertyID(harness, InspectorPropertyID.layerName.path)
        let frameName = try control(harness.controller, InspectorPropertyID.layerName.path, as: NSTextField.self)
        try require(harness.window.makeFirstResponder(frameName), "이름 first responder 실패")
        guard let nameEditor = frameName.currentEditor() as? NSTextView else { throw Failure(message: "이름 편집 없음") }
        nameEditor.string = "Renamed"
        let frameGeneration = harness.controller.appliedSnapshot?.generation ?? 0
        source.select(.text)
        harness.flush()
        guard let renamed = source.receivedCommands.lastIndex(where: { $0 == .setValue(nameID, .text("Renamed"), phase: .commit) })
        else { throw Failure(message: "구성이 바뀌는 선택 변경 시 이름 편집이 확정되지 않음") }
        try require(source.receivedGenerations[renamed] == frameGeneration, "이름 확정이 이전 세대가 아님")
        let textName = try control(harness.controller, InspectorPropertyID.layerName.path, as: NSTextField.self)
        try require(textName !== frameName, "Text 페이지가 아닌 이전 페이지의 이름 필드가 보임")
        guard let textEditor = textName.currentEditor() as? NSTextView else {
            throw Failure(message: "새 페이지의 같은 속성(이름) 필드로 포커스가 이어지지 않음")
        }
        try require(textEditor.string == "Headline", "새 선택 이름이 보이지 않음 '\(textEditor.string)'")

        // 새 페이지에 같은 속성이 없으면(너비 → Text) 보이지 않는 컨트롤에 포커스가 남지 않아야 합니다.
        source.select(.frame)
        harness.flush()
        let frameWidth = try control(harness.controller, "frame.size.width", as: NSTextField.self)
        try require(harness.window.makeFirstResponder(frameWidth), "너비 first responder 실패")
        source.select(.text)
        harness.flush()
        if let responder = harness.window.firstResponder as? NSView {
            let owner = ((responder as? NSText)?.delegate as? NSView) ?? responder
            try require(!(owner.isDescendant(of: harness.controller.view) && !owner.isDescendant(of: harness.controller.activePage ?? owner)),
                        "대기 페이지 컨트롤에 포커스가 남음: \(describe(owner))")
        }

        // 캐시 미스(로딩 상태)를 거쳐도 페이지가 보이면 같은 속성 컨트롤로 포커스가 돌아옵니다.
        let textNameAgain = try control(harness.controller, InspectorPropertyID.layerName.path, as: NSTextField.self)
        try require(harness.window.makeFirstResponder(textNameAgain), "이름 first responder 실패")
        source.select(.componentInstance)
        harness.controller.flushPendingSnapshot()
        try require(harness.controller.isShowingState, "Component 구성이 이미 있음(캐시 미스 경로를 검사하지 못함)")
        harness.flush()
        let componentName = try control(harness.controller, InspectorPropertyID.layerName.path, as: NSTextField.self)
        try require(componentName.currentEditor() != nil, "로딩 뒤 같은 속성 필드로 포커스가 돌아오지 않음")
    }

    /// 페이지 재사용: 접힘 상태 공유, 상한 초과 시 유휴 시간 제거, 제거 뒤 재생성.
    private static func checkPageCache() throws {
        let harness = Harness(scenario: .frame, appearance: .aqua)
        defer { harness.close() }
        let controller = harness.controller
        guard let layer = controller.sectionViews.first(where: { $0.sectionID == .layer }) else {
            throw Failure(message: "Layer 섹션 없음")
        }
        layer.disclosureButton.performClick(nil)
        let scenarios: [InspectorStubDataSource.Scenario] = [.rectangle, .path, .text, .image, .componentInstance]
        for scenario in scenarios {
            harness.source.select(scenario)
            harness.flush()
            let shared = controller.sectionViews.first { $0.sectionID == .layer }
            try require(shared?.isExpanded == false, "\(scenario.rawValue): 접힌 Layer 섹션 상태가 페이지 사이에 공유되지 않음")
        }
        layer.disclosureButton.performClick(nil)

        // 상한을 넘기면 가장 오래 안 쓴 대기 페이지를 유휴 시간(배경 작업)에 제거합니다.
        controller.prewarm((1...InspectorViewController.maximumCachedPages + 2).map(componentSnapshot(propertyCount:)))
        harness.runAllWork()
        try require(controller.cachedPageCount <= InspectorViewController.maximumCachedPages,
                    "페이지 상한 초과 \(controller.cachedPageCount)")
        let attached = controller.view.subviews.filter { $0 is InspectorPageView }.count
        try require(attached == controller.cachedPageCount, "제거한 페이지가 창에 남음 \(attached) ≠ \(controller.cachedPageCount)")
        try require(controller.activePage?.superview != nil, "활성 페이지가 제거됨")

        harness.source.select(.frame)
        harness.flush()
        try require(controller.sectionViews.map(\.sectionID) == [.layer, .frameLayout, .frameAppearance, .modifiers],
                    "제거된 페이지 재생성 실패")
        let width = try control(controller, "frame.size.width", as: NSTextField.self)
        try require(width.stringValue == "320", "재생성 페이지 값 '\(width.stringValue)'")
    }

    /// 폭 변경: 활성 페이지는 바로, 대기 페이지는 유휴 시간에 새 폭으로 다시 배치합니다. 이어서 대기 페이지를
    /// 보이면 컨트롤이 새 폭의 패널 여백 안에 있어야 합니다.
    private static func checkResize() throws {
        let harness = Harness(scenario: .frame, appearance: .aqua, prewarms: true)
        defer { harness.close() }
        harness.runAllWork()
        for width: CGFloat in [360, 260] {
            harness.window.setContentSize(NSSize(width: width, height: 900))
            harness.window.contentView?.layoutSubtreeIfNeeded()
            try checkLayout(harness, label: "resize \(Int(width))/active", width: width)
            harness.runAllWork() // 대기 페이지 재배치(배경 작업)
            for scenario in [InspectorStubDataSource.Scenario.text, .rectangle, .image, .frame] {
                harness.source.select(scenario)
                harness.flush()
                try require(!harness.controller.isShowingState, "resize \(Int(width))/\(scenario.rawValue): 캐시 미스")
                try checkLayout(harness, label: "resize \(Int(width))/\(scenario.rawValue)", width: width)
            }
        }
    }

    /// 공개 Property가 `propertyCount`개인 Component Instance 구성(구성 수를 늘리는 검사용).
    private static func componentSnapshot(propertyCount: Int) -> InspectorSnapshot {
        let resolve: InspectorFieldResolver = { _ in InspectorFieldState(value: .none) }
        let properties = (0..<propertyCount).map { (id: "p\($0)", name: "Property \($0)",
                                                    kind: InspectorPropertyCatalog.ComponentValueKind.string) }
        let sections = [InspectorPropertyCatalog.layerSection.make(resolve)]
            + InspectorPropertyCatalog.kindSections(.componentInstance, shape: nil).map { $0.make(resolve) }
            + [InspectorPropertyCatalog.componentPropertiesSection(properties).make(resolve),
               InspectorPropertyCatalog.componentSlotsSection([]).make(resolve)]
        return InspectorSnapshot(
            generation: 0, selectionToken: 0, state: .ready,
            selection: InspectorSelectionSummary(count: 1, nodeKinds: [.componentInstance], shapeKind: nil,
                                                 displayName: nil),
            editability: .editable, sections: sections,
            modifiers: InspectorModifierStack(availability: .available, items: [], addableKinds: [], isEditable: true)
        )
    }

    /// 예열: 자주 쓰는 카탈로그 구성 페이지가 유휴 작업으로 만들어지고, 그 구성의 Stub 장면은 캐시 미스 없이
    /// 바로 보입니다. 나머지 구성은 로딩 상태를 먼저 보인 뒤 나눠 만들어 보입니다. 대기 페이지는 보이지 않고
    /// (alpha 0) hit test·접근성 트리에서 빠집니다.
    private static func checkPrewarmAndStandby(_ report: inout [String]) throws {
        let harness = Harness(scenario: .noSelection, appearance: .aqua, prewarms: true)
        defer { harness.close() }
        let controller = harness.controller
        let prewarm = Array(InspectorPropertyCatalog.prewarmSnapshots()
            .prefix(InspectorViewController.prewarmedCatalogPageCount))
        harness.runAllWork()
        let prewarmKeys = Set(prewarm.map(InspectorPageView.structureKey(of:)))
        for snapshot in prewarm {
            try require(controller.hasCompletePage(for: snapshot),
                        "예열 페이지 없음: \(InspectorPageView.structureKey(of: snapshot).prefix(60))")
        }
        // 예열 구성 장면을 먼저 봅니다(미스로 만든 페이지가 상한을 넘겨 예열 페이지를 밀어내기 전에).
        let scenarios = InspectorStubDataSource.Scenario.allCases.filter {
            InspectorStubDataSource(scenario: $0).snapshot.state == .ready
        }
        let expectsHit = { (scenario: InspectorStubDataSource.Scenario) in
            prewarmKeys.contains(InspectorPageView.structureKey(of: InspectorStubDataSource(scenario: scenario).snapshot))
        }
        var hits = 0
        var misses = 0
        for scenario in scenarios.filter(expectsHit) + scenarios.filter({ !expectsHit($0) }) {
            harness.source.select(scenario)
            controller.flushPendingSnapshot() // 대기열을 돌리지 않음: 예열 페이지가 있으면 바로 보여야 함
            harness.window.contentView?.layoutSubtreeIfNeeded()
            let hit = expectsHit(scenario)
            try require(controller.isShowingState == !hit,
                        "\(scenario.rawValue): \(hit ? "예열 페이지가 바로 보이지 않음" : "예열 밖 구성인데 로딩 상태가 아님")")
            if hit { hits += 1 } else { misses += 1 }
            if !hit {
                // 로딩 문구·spinner는 잠시 기다렸다 보입니다(짧은 로딩에 깜박이지 않게).
                try require(controller.isLoadingRevealPending && !controller.isLoadingSpinnerRunning,
                            "\(scenario.rawValue): 로딩 문구를 바로 보임")
                if misses == 0 {
                    try pumpUntil("로딩 문구 표시") { controller.isLoadingSpinnerRunning }
                }
            }
            harness.flush()
            try require(!controller.isShowingState, "\(scenario.rawValue): 나눠 만든 페이지가 보이지 않음")
            try require(!controller.isLoadingSpinnerRunning, "\(scenario.rawValue): 페이지를 보인 뒤에도 로딩 spinner가 돎")
            try checkStandby(harness, label: scenario.rawValue)
        }
        report.append("prewarm \(prewarm.count) layouts, \(hits) scenarios without miss, \(misses) via loading")
    }

    /// 대기 페이지 격리: 보이지 않고, hit test·접근성 트리·key view 순서에 없습니다.
    private static func checkStandby(_ harness: Harness, label: String) throws {
        let controller = harness.controller
        guard let active = controller.activePage else { throw Failure(message: "\(label): 활성 페이지 없음") }
        try require(active.alphaValue == 1 && active.isActive, "\(label): 활성 페이지가 보이지 않음")
        let center = NSPoint(x: active.bounds.midX, y: active.bounds.midY)
        for page in controller.allPages where page !== active {
            try require(page.alphaValue == 0 && !page.isActive, "\(label): 대기 페이지가 보임")
            try require(page.hitTest(page.convert(center, to: page.superview)) == nil, "\(label): 대기 페이지가 클릭을 받음")
            try require((page.accessibilityChildren() ?? []).isEmpty, "\(label): 대기 페이지가 접근성 트리에 있음")
        }
        // 접근성 트리를 루트부터 따라가도 활성 페이지 밖의 뷰가 나오지 않아야 합니다.
        var pending: [Any] = controller.view.accessibilityChildren() ?? []
        var visited = 0
        while let element = pending.popLast(), visited < 5000 {
            visited += 1
            var children: [Any]?
            if let view = element as? NSView {
                try require(view.isDescendant(of: active), "\(label): 접근성 트리에 대기 페이지 요소 \(describe(view))")
                children = view.accessibilityChildren()
            } else if let node = element as? NSAccessibilityElement {
                children = node.accessibilityChildren()
            }
            pending += children ?? []
        }
        if let window = harness.window.contentView,
           let hit = window.hitTest(window.convert(center, from: active)) {
            try require(hit.isDescendant(of: active), "\(label): 클릭이 활성 페이지가 아닌 \(describe(hit))로 감")
        }
        for view in controller.keyViewChain {
            try require(view.isDescendant(of: active), "\(label): key view 순서에 대기 페이지 컨트롤")
        }
    }

    /// 메인 스레드 점유 시간(아키텍처 §6.3 단일 점유 2ms, 가이드 9장). 판정과 측정값 보고를 함께 합니다.
    /// 다른 검사가 창을 많이 만들기 전에(프로세스 첫 단계로) 돌립니다.
    ///
    /// - 나눠 만들기(예열·캐시 미스·보강): slice마다 그 slice가 붙인 컨트롤의 배치와 화면 그리기
    ///   (`displayIfNeeded`)까지 잽니다. 2ms 상한을 다 지키지 못해 분포를 보고하고 회귀 경계(중앙값 3ms, p95 8ms)만 봅니다.
    /// - 선택 경로: 값 하나 변경과 캐시 미스(로딩 표시)는 반영 + 배치 + 그리기 합이 2ms 이하여야 합니다.
    /// - 예열 페이지 전환: 값이 같으면 alpha 전환뿐이고, 값이 모두 바뀌면(처음 표시 포함) AppKit 컨트롤 갱신
    ///   (popup·segmented 하나당 0.5~0.8ms 재배치)이 들어 2ms를 넘습니다. 측정값만 보고하고 회귀 경계(25ms)만 봅니다.
    /// Stub snapshot 생성 시간은 뺍니다. 실제 판정은 Release·통합 창에서 XVI.1 계측이 맡습니다.
    private static func checkTiming(_ report: inout [String]) throws {
        let budget = 2.0
        let harness = Harness(scenario: .noSelection, appearance: .aqua, prewarms: true)
        defer { harness.close() }
        let controller = harness.controller
        let queue = controller.buildQueue
        queue.sliceEpilogue = { harness.display() }

        // 1) 예열.
        let footprintBefore = memoryFootprintMB()
        queue.resetMeasurements()
        harness.runAllWork()
        let prewarmSlices = queue.sliceDurations
        let footprintAfter = memoryFootprintMB()
        harness.display()

        // 2) 예열 페이지 처음 표시(값이 모두 없음 → 실제 값). 이어서 보강 작업(처음 보는 modifier 파라미터 구성).
        let kinds: [InspectorStubDataSource.Scenario] = [.frame, .text, .rectangle, .image]
        var refineSlices: [Double] = []
        func finishRefinement() {
            queue.resetMeasurements()
            queue.finishActiveWork()
            refineSlices += queue.sliceDurations
        }
        var firstShow: [Double] = []
        for scenario in kinds {
            harness.source.select(scenario)
            firstShow.append(harness.measureFlush().total)
            try require(!controller.isShowingState, "\(scenario.rawValue): 예열 페이지 전환이 캐시 미스")
            finishRefinement()
        }

        // 3) 예열 페이지 사이 전환: 같은 값(같은 Stub 장면)으로 돌아올 때.
        var sameValueSwitch: [Double] = []
        for _ in 0..<3 {
            for scenario in kinds {
                harness.source.select(scenario)
                sameValueSwitch.append(harness.measureFlush().total)
                finishRefinement()
            }
        }

        // 4) 같은 종류의 다른 layer로 전환: 모든 필드 값이 바뀜(현실적인 선택 변경).
        var allValuesSwitch: [Double] = []
        for scenario in [InspectorStubDataSource.Scenario.frame, .text, .image] {
            harness.source.select(scenario)
            _ = harness.measureFlush()
            finishRefinement()
            let base = harness.source.snapshot
            for round in 1...6 {
                var snapshot = base.withAllValuesChanged(seed: round)
                snapshot.generation = (controller.appliedSnapshot?.generation ?? 0) + 1
                snapshot.selectionToken = UInt64.max - UInt64(round)
                controller.receive(snapshot)
                allValuesSwitch.append(harness.measureFlush().total)
                finishRefinement()
            }
            // Stub 세대로 돌아가도록 Stub 쪽 세대를 앞으로 당깁니다.
            while harness.source.snapshot.generation <= (controller.appliedSnapshot?.generation ?? 0) {
                harness.source.submit(.reload, basedOn: harness.source.snapshot.generation)
            }
            harness.flush()
        }

        // 5) 값 하나 변경(같은 페이지).
        harness.source.select(.frame)
        _ = harness.measureFlush()
        finishRefinement()
        let widthID = try propertyID(harness, "frame.size.width")
        var updates: [Double] = []
        for iteration in 0..<16 {
            harness.source.submit(.setValue(widthID, .number(Double(100 + iteration)), phase: .commit),
                                  basedOn: harness.source.snapshot.generation)
            updates.append(harness.measureFlush().total)
        }

        // 6) 캐시 미스(예열 밖 구성): 로딩 표시까지(반영 + 배치 + 그리기), 이어지는 구성 slice.
        var missToLoading: [Double] = []
        var missSlices: [Double] = []
        for count in 1...6 {
            harness.source.select(.frame)
            harness.flush()
            // 선택 사이의 유휴 시간: 넘친 페이지 제거 같은 배경 작업을 마칩니다.
            harness.runAllWork()
            var snapshot = componentSnapshot(propertyCount: 10 + count)
            // 다음 Stub 세대와 같은 번호를 쓰면 이후 Stub snapshot이 오래된 세대로 버려지지 않습니다.
            snapshot.generation = harness.source.snapshot.generation + 1
            snapshot.selectionToken = UInt64.max - 100 - UInt64(count)
            controller.receive(snapshot)
            missToLoading.append(harness.measureFlush().total)
            try require(controller.isShowingState, "예열 밖 구성인데 로딩 상태가 아님")
            queue.resetMeasurements()
            queue.finishActiveWork()
            missSlices += queue.sliceDurations
            try require(!controller.isShowingState && controller.activePage != nil, "캐시 미스 페이지가 보이지 않음")
        }

        func percentile(_ values: [Double], _ p: Double) -> Double {
            guard !values.isEmpty else { return 0 }
            let sorted = values.sorted()
            return sorted[min(sorted.count - 1, Int((Double(sorted.count - 1) * p).rounded()))]
        }
        func describeSlices(_ values: [Double]) -> String {
            String(format: "%d slices p50 %.2f p95 %.2f max %.2f ms, %d over 2ms", values.count,
                   percentile(values, 0.5), percentile(values, 0.95), values.max() ?? 0,
                   values.filter { $0 > budget }.count)
        }
        func describe(_ values: [Double]) -> String {
            String(format: "p50 %.2f p95 %.2f max %.2f ms", percentile(values, 0.5), percentile(values, 0.95),
                   values.max() ?? 0)
        }
        report.append("timing[prewarm \(InspectorViewController.prewarmedCatalogPageCount) pages: \(describeSlices(prewarmSlices)), "
            + String(format: "footprint +%.1fMB]", footprintAfter - footprintBefore))
        report.append("timing[first show \(describe(firstShow)); same-value switch \(describe(sameValueSwitch)); "
            + "all-values switch \(describe(allValuesSwitch))]")
        report.append("timing[value change \(describe(updates)); miss to loading \(describe(missToLoading))]")
        report.append("timing[miss build \(describeSlices(missSlices)); refinement \(describeSlices(refineSlices))]")

        // 나눠 만들기 slice: 2ms 상한을 다 지키지 못하므로(컨트롤 하나의 첫 그리기와 런 루프마다의 CA commit이 slice
        // 하나에 들어감, 명세 8.3절 한계) 값을 보고하고 회귀 경계만 판정합니다: 중앙값 3ms, p95 8ms.
        // 이전 구현은 페이지 하나를 한 번에 만들어 단일 점유가 70~90ms였습니다.
        let sliceMedianGuard = 3.0
        let sliceGuard = 8.0
        for (name, values) in [("예열", prewarmSlices), ("미스 구성", missSlices), ("보강", refineSlices)] {
            try require(percentile(values, 0.5) <= sliceMedianGuard,
                        "\(name) slice 중앙값이 회귀 경계 3ms 초과: \(describeSlices(values))")
            try require(percentile(values, 0.95) <= sliceGuard, "\(name) slice p95가 회귀 경계 8ms 초과: \(describeSlices(values))")
        }
        try require(percentile(updates, 0.5) <= budget, "값 변경(반영+배치+그리기) 중앙값이 2ms 초과: \(describe(updates))")
        try require(percentile(missToLoading, 0.5) <= budget,
                    "캐시 미스 로딩 표시 중앙값이 2ms 초과: \(describe(missToLoading))")
        try require(percentile(sameValueSwitch, 0.5) <= budget,
                    "같은 값 예열 페이지 전환 중앙값이 2ms 초과: \(describe(sameValueSwitch))")
        let switchGuard = 25.0
        try require(percentile(firstShow, 0.95) <= switchGuard, "예열 페이지 첫 표시 p95가 25ms 초과: \(describe(firstShow))")
        try require(percentile(allValuesSwitch, 0.95) <= switchGuard,
                    "값이 모두 바뀌는 전환 p95가 25ms 초과: \(describe(allValuesSwitch))")
    }

    /// 이 프로세스의 물리 메모리 사용량(MB, phys_footprint).
    private static func memoryFootprintMB() -> Double {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
        let result = withUnsafeMutablePointer(to: &info) { pointer in
            pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? Double(info.phys_footprint) / 1_048_576 : -1
    }

    // MARK: - 도우미

    /// 화면 밖 창 하나에 인스펙터를 넣은 검사 환경. 대기열은 런 루프에 예약하지 않고 검사가 직접 돌립니다.
    /// 통합 창(`NSSplitViewItem`)처럼 인스펙터 뷰를 제약으로 담아, 창이 Auto Layout 엔진을 가진 상태에서 잽니다
    /// (제약이 하나도 없는 창은 배치마다 뷰 트리 전체에서 제약 사용 뷰를 찾아 실제보다 느립니다).
    @MainActor
    private final class Harness {
        let source: InspectorStubDataSource
        let controller: InspectorViewController
        let window: NSWindow

        init(scenario: InspectorStubDataSource.Scenario, appearance: NSAppearance.Name, prewarms: Bool = false) {
            source = InspectorStubDataSource(scenario: scenario)
            controller = InspectorViewController(dataSource: source, prewarmsCatalogPages: prewarms)
            controller.buildQueue.schedulesAutomatically = false
            window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: InspectorMetrics.typicalWidth, height: 900),
                styleMask: [.titled, .resizable], backing: .buffered, defer: false
            )
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: appearance)
            let container = NSView(frame: NSRect(x: 0, y: 0, width: InspectorMetrics.typicalWidth, height: 900))
            window.contentView = container
            let inspector = controller.view
            inspector.translatesAutoresizingMaskIntoConstraints = false
            container.addSubview(inspector)
            NSLayoutConstraint.activate([
                inspector.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                inspector.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                inspector.topAnchor.constraint(equalTo: container.topAnchor),
                inspector.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            ])
            window.setContentSize(NSSize(width: InspectorMetrics.typicalWidth, height: 900))
            flush()
        }

        /// 대기 snapshot을 반영하고, 지금 필요한 페이지 구성(급한 작업)을 끝까지 실행한 뒤 배치합니다.
        func flush() {
            controller.flushPendingSnapshot()
            controller.buildQueue.finishActiveWork()
            window.contentView?.layoutSubtreeIfNeeded()
        }

        /// 예열·제거를 포함해 대기열을 비웁니다.
        func runAllWork() {
            controller.buildQueue.runSlices { true }
            window.contentView?.layoutSubtreeIfNeeded()
        }

        /// 창을 화면에 내놓지 않은 채 그리기를 강제합니다(그리기 비용 측정).
        func display() {
            window.displayIfNeeded()
            CATransaction.flush()
        }

        /// 반영 + 배치(ms)와 이어지는 그리기(ms).
        struct Sample {
            let apply: Double
            let display: Double
            var total: Double { apply + display }
        }

        /// 대기 snapshot 반영 + 배치와 이어지는 그리기를 잽니다(대기열은 돌리지 않음).
        func measureFlush() -> Sample {
            let start = DispatchTime.now().uptimeNanoseconds
            controller.flushPendingSnapshot()
            window.contentView?.layoutSubtreeIfNeeded()
            let applied = DispatchTime.now().uptimeNanoseconds
            display()
            let drawn = DispatchTime.now().uptimeNanoseconds
            return Sample(apply: Double(applied - start) / 1_000_000, display: Double(drawn - applied) / 1_000_000)
        }

        func close() {
            window.close()
        }
    }

    /// 적용된 snapshot에서 경로로 속성 id(coreKey 포함)를 찾습니다.
    private static func propertyID(_ harness: Harness, _ path: String) throws -> InspectorPropertyID {
        guard let snapshot = harness.controller.appliedSnapshot else { throw Failure(message: "snapshot 없음") }
        let rows = snapshot.sections.flatMap(\.rows) + (snapshot.modifiers?.items.flatMap(\.parameters) ?? [])
        guard let field = rows.flatMap(\.fields).first(where: { $0.id.path == path }) else {
            throw Failure(message: "\(path) 필드 없음")
        }
        return field.id
    }

    private static func require(_ condition: Bool, _ message: @autoclosure () -> String) throws {
        if !condition { throw Failure(message: message()) }
    }

    private static func control<T: NSView>(
        _ controller: InspectorViewController, _ identifier: String, as type: T.Type
    ) throws -> T {
        guard let view = controller.control(identifier: identifier) as? T else {
            throw Failure(message: "\(identifier) \(type) 없음")
        }
        return view
    }

    private static func isEnabled(_ controller: InspectorViewController, _ identifier: String) -> Bool {
        (controller.control(identifier: identifier) as? NSControl)?.isEnabled ?? false
    }

    private static func isInteractive(_ control: NSControl) -> Bool {
        switch control {
        case let field as NSTextField: return field.isEditable
        case is NSButton, is NSPopUpButton, is NSSegmentedControl, is NSColorWell, is NSStepper, is NSSwitch,
             is NSTableView:
            return true
        default: return false
        }
    }

    /// 숨지 않은 하위 뷰 전체(스크롤러 등 시스템 내부 뷰 포함). 대기 페이지(alpha 0)는 뺍니다.
    private static func visibleViews(in root: NSView) -> [NSView] {
        var result: [NSView] = []
        func walk(_ view: NSView) {
            guard !view.isHidden else { return }
            if let page = view as? InspectorPageView, !page.isActive { return }
            result.append(view)
            view.subviews.forEach(walk)
        }
        walk(root)
        return result
    }

    /// 실패 메시지용 설명: 뷰 종류·식별자와 가장 가까운 식별 가능한 상위 뷰.
    private static func describe(_ view: NSView) -> String {
        func name(_ view: NSView) -> String {
            "\(type(of: view))[\(view.identifier?.rawValue ?? view.accessibilityLabel() ?? "-")]"
        }
        var ancestor = view.superview
        while let current = ancestor, current.identifier == nil, current.accessibilityLabel() == nil {
            ancestor = current.superview
        }
        return name(view) + (ancestor.map { " in \(name($0))" } ?? "")
    }
}

// MARK: - 검사용 snapshot 변형

private extension InspectorSnapshot {
    /// 모든 필드 값(과 modifier 요약·이름)을 다른 값으로 바꾼 snapshot. 같은 구성의 다른 layer 선택을 흉내 냅니다.
    func withAllValuesChanged(seed: Int) -> InspectorSnapshot {
        var copy = self
        copy.sections = sections.map { section in
            var changed = section
            changed.rows = section.rows.map { $0.withValuesChanged(seed: seed) }
            return changed
        }
        if var stack = modifiers {
            stack.items = stack.items.map { item in
                var changed = item
                changed.summary += " \(seed)"
                changed.parameters = item.parameters.map { $0.withValuesChanged(seed: seed) }
                return changed
            }
            copy.modifiers = stack
        }
        copy.selection.displayName = (selection.displayName ?? "Layer") + " \(seed)"
        return copy
    }
}

private extension InspectorRow {
    func withValuesChanged(seed: Int) -> InspectorRow {
        var copy = self
        copy.fields = fields.map { field in
            var changed = field
            changed.value = Self.changed(field.value, control: field.control, seed: seed)
            return changed
        }
        return copy
    }

    static func changed(_ value: InspectorValue, control: InspectorControl, seed: Int) -> InspectorValue {
        switch value {
        case .number(let number):
            return .number(number + Double(seed))
        case .bool(let on):
            return .bool(seed.isMultiple(of: 2) ? on : !on)
        case .choice(let id):
            let choices: [InspectorChoice]
            switch control {
            case .popup(let list), .segmented(let list): choices = list
            default: choices = []
            }
            guard let index = choices.firstIndex(where: { $0.id == id }), choices.count > 1 else { return value }
            return .choice(choices[(index + seed) % choices.count].id)
        case .color(var color):
            color.red = (color.red + 0.1 * Double(seed)).truncatingRemainder(dividingBy: 1)
            return .color(color)
        case .text(let text):
            return .text(text + " \(seed)")
        case .none, .mixed:
            return value
        }
    }
}
