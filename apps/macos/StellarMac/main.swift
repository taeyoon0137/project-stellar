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
