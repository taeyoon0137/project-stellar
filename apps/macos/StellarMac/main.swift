import AppKit

// StellarMac 진입점.
// 스토리보드 없이 AppKit 앱을 명시적으로 구성합니다(NSApplicationMain 미사용).
let application = NSApplication.shared
let delegate = AppDelegate()
application.delegate = delegate
application.setActivationPolicy(.regular)
application.run()
