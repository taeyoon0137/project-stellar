import SwiftUI

/// 앱 설정 화면 placeholder(SwiftUI).
///
/// 캔버스는 AppKit view로 두고, 설정 화면만 SwiftUI placeholder로 둡니다.
/// 실제 설정 항목은 이후 단계에서 추가합니다.
struct SettingsPlaceholderView: View {
    var body: some View {
        Text("Stellar Settings")
            .padding()
    }
}
