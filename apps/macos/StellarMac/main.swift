import AppKit

// StellarMac 진입점.

// smoke 모드: GUI를 띄우지 않고 FFI로 코어 버전을 읽어 출력한 뒤 종료합니다.
// 앱이 Rust 코어와 정상 링크/호출되는지 CI에서 종료 코드로 검증하기 위한 경로입니다.
if CommandLine.arguments.contains("--smoke") {
    if let version = CoreRuntime.version() {
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
