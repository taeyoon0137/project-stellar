import AppKit

// Stellar 진입점. UI는 재설계 전까지 빈 창 하나만 있는 껍데기입니다(ADR-0098).

// lifecycle 검사 모드: 코어 runtime create/destroy가 1:1인지 headless로 검증하고 종료(II.5.3).
if CommandLine.arguments.contains("--lifecycle-check") {
    exit(LifecycleCheck.run() ? 0 : 1)
}

// update 검사 모드: 업데이트 구성, Stable/Beta 채널 격리, appcast 서명, 승인 전 다운로드 금지를 실제 Sparkle과
// loopback fixture로 headless 검증하고 종료(XVI.9.5, ADR-0017).
if CommandLine.arguments.contains("--update-check") {
    exit(UpdateCheck.run() ? 0 : 1)
}

// 명령 설치 검사 모드: `stellar` 명령 설치기의 위치 결정·설치·제거·상태 확인을 메모리·임시 폴더 fixture로 검증하고
// 종료(XXIV.5.3~XXIV.5.5).
if CommandLine.arguments.contains("--command-install-check") {
    exit(CommandInstallCheck.run() ? 0 : 1)
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
