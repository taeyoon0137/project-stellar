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
        do {
            var report: [String] = []
            try checkSymbols(&report)
            try checkScenarios(&report)
            try checkInteractions(&report)
            print("inspector-ui ok: \(report.joined(separator: ", "))")
            return true
        } catch let failure as Failure {
            FileHandle.standardError.write(Data("inspector-ui 실패: \(failure.message)\n".utf8))
            return false
        } catch {
            FileHandle.standardError.write(Data("inspector-ui 실패: \(error)\n".utf8))
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

    private static func checkLayout(_ harness: Harness, label: String) throws {
        let views = visibleViews(in: harness.controller.view)
        let ambiguous = views.filter(\.hasAmbiguousLayout)
        try require(ambiguous.isEmpty, "\(label): 모호한 Auto Layout \(ambiguous.map(describe).joined(separator: ", "))")

        // 내용이 pane 폭을 밀어내지 않아야 합니다.
        let root = harness.controller.view
        try require(abs(root.bounds.width - InspectorMetrics.typicalWidth) < 0.5,
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
        try checkNumberRounding()
        try checkModifierStack()
        try checkMixedAndText()
        try checkLockFlow()
        try checkPageCache()
        try checkSelectionChangeCommitsEdit()
        report.append("interactions")
        report.append(try measureApply())
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
        _ = harness.window.makeFirstResponder(nil)
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

    /// 편집 중 선택이 바뀌면 입력을 이전 세대로 확정하고 편집을 끝냅니다(새 선택에 잘못 적용 방지).
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
        try require(width.currentEditor() == nil, "선택 변경 후에도 편집 중")
    }

    /// 페이지 재사용: 상한·제거 후 재생성·접힘 상태 공유.
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
        controller.evictOverflowPages()
        try require(controller.cachedPageCount <= InspectorViewController.maximumCachedPages,
                    "페이지 상한 초과 \(controller.cachedPageCount)")
        harness.source.select(.frame)
        harness.flush()
        try require(controller.sectionViews.map(\.sectionID) == [.layer, .frameLayout, .frameAppearance, .modifiers],
                    "제거된 페이지 재생성 실패")
        let width = try control(controller, "frame.size.width", as: NSTextField.self)
        try require(width.stringValue == "320", "재생성 페이지 값 '\(width.stringValue)'")
    }

    /// snapshot 반영(배치 포함)에 걸린 메인 스레드 시간(참고값, 판정 기준 아님).
    ///
    /// Stub의 snapshot 생성 시간은 빼고 `flush`(반영 + 배치)만 잽니다. 처음 보는 종류의 섹션을 만드는
    /// 비용(cold), 이미 만든 섹션을 재사용하는 선택 종류 전환, 값 하나 변경을 따로 보고합니다.
    /// 실제 판정은 XVI.1 계측이 맡습니다.
    private static func measureApply() throws -> String {
        let harness = Harness(scenario: .frame, appearance: .aqua)
        defer { harness.close() }
        func median(_ samples: [Double]) -> Double { samples.sorted()[samples.count / 2] }
        func measureFlush() -> Double {
            let start = DispatchTime.now().uptimeNanoseconds
            harness.flush()
            return Double(DispatchTime.now().uptimeNanoseconds - start) / 1_000_000
        }
        harness.source.select(.image)
        let cold = measureFlush()
        var switches: [Double] = []
        var updates: [Double] = []
        for iteration in 0..<12 {
            let even = iteration.isMultiple(of: 2)
            harness.source.select(even ? .text : .frame)
            let switchTime = measureFlush()
            let path = even ? "text.paragraph.letterSpacing" : "frame.size.width"
            harness.source.submit(.setValue(try propertyID(harness, path), .number(Double(iteration)), phase: .commit),
                                  basedOn: 0)
            let updateTime = measureFlush()
            if iteration > 1 {
                switches.append(switchTime)
                updates.append(updateTime)
            }
        }
        return String(format: "apply: new page %.2fms, cached page switch %.2fms, value change %.2fms (median)",
                      cold, median(switches), median(updates))
    }

    // MARK: - 도우미

    /// 화면 밖 창 하나에 인스펙터를 넣은 검사 환경.
    @MainActor
    private final class Harness {
        let source: InspectorStubDataSource
        let controller: InspectorViewController
        let window: NSWindow

        init(scenario: InspectorStubDataSource.Scenario, appearance: NSAppearance.Name) {
            source = InspectorStubDataSource(scenario: scenario)
            controller = InspectorViewController(dataSource: source)
            window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: InspectorMetrics.typicalWidth, height: 900),
                styleMask: [.titled, .resizable], backing: .buffered, defer: false
            )
            window.isReleasedWhenClosed = false
            window.appearance = NSAppearance(named: appearance)
            window.contentViewController = controller
            window.setContentSize(NSSize(width: InspectorMetrics.typicalWidth, height: 900))
            window.contentView?.layoutSubtreeIfNeeded()
        }

        /// 대기 snapshot을 반영하고 배치합니다.
        func flush() {
            controller.flushPendingSnapshot()
            window.contentView?.layoutSubtreeIfNeeded()
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

    /// 숨지 않은 하위 뷰 전체(스크롤러 등 시스템 내부 뷰 포함).
    private static func visibleViews(in root: NSView) -> [NSView] {
        var result: [NSView] = []
        func walk(_ view: NSView) {
            guard !view.isHidden else { return }
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
