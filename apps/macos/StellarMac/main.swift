import AppKit

// StellarMac 진입점.

// lifecycle 검사 모드: 코어 runtime create/destroy가 1:1인지 headless로 검증하고 종료(II.5.3).
if CommandLine.arguments.contains("--lifecycle-check") {
    exit(LifecycleCheck.run() ? 0 : 1)
}

// canvas 검사 모드: drawable pixel == point × scale 및 코어 round-trip을 검증하고 종료(II.5.4).
if CommandLine.arguments.contains("--canvas-check") {
    exit(CanvasCheck.run() ? 0 : 1)
}

// text input 검사 모드: 네이티브 UI 텍스트 컨트롤의 undo/IME smoke를 headless로 검증하고 종료(XII.5.1).
if CommandLine.arguments.contains("--text-input-check") {
    exit(TextInputCheck.run() ? 0 : 1)
}

// input 검사 모드: mouse·trackpad·keyboard NSEvent → 공통 InputEvent 변환을 headless로 검증하고 종료(XII.3.2).
if CommandLine.arguments.contains("--input-check") {
    exit(InputCheck.run() ? 0 : 1)
}

// update 검사 모드: 업데이트 구성, Stable/Beta 채널 격리, appcast 서명, 승인 전 다운로드 금지를 실제 Sparkle과
// loopback fixture로 headless 검증하고 종료(XVI.9.5, ADR-0017).
if CommandLine.arguments.contains("--update-check") {
    exit(UpdateCheck.run() ? 0 : 1)
}

// 영역별 UI 검사 모드: 각 UI 영역을 화면 밖 창으로 만들어 검사하고 종료합니다(docs/design/ui/*.md 검사 절).
// 인자 이름은 `--<영역>-ui-check`이고, scripts/macos/smoke.sh가 같은 순서로 모두 실행합니다.
let areaUIChecks: [(argument: String, run: () -> Bool)] = [
    ("--shell-frame-ui-check", ShellFrameUICheck.run),
    ("--tools-menus-ui-check", ToolsUICheck.run),
    ("--sidebar-ui-check", SidebarUICheck.run),
    ("--inspector-ui-check", InspectorUICheck.run),
    ("--git-panel-ui-check", GitPanelUICheck.run),
    ("--agent-panel-ui-check", AgentPanelUICheck.run),
    ("--welcome-ui-check", WelcomeUICheck.run),
]
for check in areaUIChecks where CommandLine.arguments.contains(check.argument) {
    exit(check.run() ? 0 : 1)
}

// 셸 통합 검사 모드: 실제 앱 구성(메뉴, 시작 창, 코어로 프로젝트 열기, 프로젝트 창 영역·도구, 복원·닫기)을 화면 밖에서
// 검사하고 종료합니다(docs/design/ui/shell_frame.md 통합 절).
if CommandLine.arguments.contains("--shell-integration-check") {
    exit(ShellIntegrationCheck.run() ? 0 : 1)
}

// Open Project 검사 모드: 폴더 선택 → 코어 열기(분류·안전 probe) → 창, 잘못된 폴더별 실패 코드 → 안내 대응을 실제 코어와
// 임시 fixture로 검사하고 종료합니다(XII.6.2).
if CommandLine.arguments.contains("--project-open-check") {
    exit(ShellProjectOpenCheck.run() ? 0 : 1)
}

// smoke 모드: GUI를 띄우지 않고 FFI로 코어 버전을 읽어 출력한 뒤 종료합니다(II.5.2).
if CommandLine.arguments.contains("--smoke") {
    let host = RuntimeHost()
    defer { host.shutdown() }
    if let version = host.version() {
        print("stellar core version: \(version)")
        exit(0)
    }
    FileHandle.standardError.write(Data("failed to read core version via FFI\n".utf8))
    exit(1)
}

// 일반 실행: 스토리보드 없이 AppKit 앱을 명시적으로 구성합니다(NSApplicationMain 미사용).
let application = NSApplication.shared
let delegate = AppDelegate()
application.delegate = delegate
application.setActivationPolicy(.regular)
application.run()
