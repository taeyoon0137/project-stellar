import SwiftUI

enum StellarColors {
    static let surface = Color(.displayP3, red: 0.96, green: 0.96, blue: 0.98, opacity: 1)
    static let accent = Color(.sRGB, red: 0, green: 0.48, blue: 1, opacity: 1)
    static let n00000000000000000000000000000001_4 = surface
    static let n00000000000000000000000000000002_5 = Color(.sRGB, red: 0.8, green: 0.1, blue: 0.1, opacity: 1)
    static let n0000000000000000000000000000000c_4 = accent
}

struct StellarAccessibility00000000000000000000000000000001: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .contain)
    }
}

struct StellarAccessibility00000000000000000000000000000002: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityAddTraits(.isStaticText)
            .accessibilitySortPriority(5)
    }
}

struct StellarAccessibility00000000000000000000000000000004: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityAddTraits(.isImage)
            .accessibilityLabel(Text(verbatim: "프로필 사진"))
            .accessibilitySortPriority(6)
    }
}

struct StellarAccessibility00000000000000000000000000000005: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityHidden(true)
    }
}

struct StellarAccessibility00000000000000000000000000000006: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text(verbatim: "크기"))
            .accessibilitySortPriority(4)
    }
}

struct StellarAccessibility00000000000000000000000000000008: ViewModifier {
    @Binding var selectedSize: String
    let onAction: (String) -> Void

    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityAddTraits(selectedSize == "00000000-0000-0000-0000-000000000008" ? .isSelected : [])
            .accessibilityAction {
                onAction("00000000-0000-0000-0000-00000000003d")
                selectedSize = "00000000-0000-0000-0000-000000000008"
            }
    }
}

struct StellarAccessibility00000000000000000000000000000009: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityAddTraits(.isStaticText)
    }
}

struct StellarAccessibility0000000000000000000000000000000a: ViewModifier {
    @Binding var selectedSize: String
    let onAction: (String) -> Void

    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityLabel(Text(verbatim: "크게"))
            .accessibilityAddTraits(selectedSize == "00000000-0000-0000-0000-00000000000a" ? .isSelected : [])
            .accessibilityAction {
                onAction("00000000-0000-0000-0000-00000000003e")
                selectedSize = "00000000-0000-0000-0000-00000000000a"
            }
    }
}

struct StellarAccessibility0000000000000000000000000000000b: ViewModifier {
    @Binding var memo: String

    func body(content: Content) -> some View {
        content
            .accessibilityLabel(Text(verbatim: "메모"))
            .accessibilityValue(Text(memo))
            .accessibilitySortPriority(3)
    }
}

struct StellarAccessibility0000000000000000000000000000000c: ViewModifier {
    @Binding var isSaving: Bool
    let onAction: (String) -> Void

    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityLabel(Text(verbatim: "저장"))
            .disabled(isSaving)
            .accessibilityAction {
                onAction("00000000-0000-0000-0000-00000000003c")
            }
            .accessibilitySortPriority(2)
    }
}

struct StellarAccessibility00000000000000000000000000000010: ViewModifier {
    let onAction: (String) -> Void

    func body(content: Content) -> some View {
        content
            .accessibilityElement(children: .contain)
            .accessibilityLabel(Text(verbatim: "알림"))
            .accessibilityAddTraits(.isModal)
            .accessibilityAction(.escape) {
                onAction("00000000-0000-0000-0000-00000000003f")
            }
            .accessibilitySortPriority(1)
    }
}

struct StellarAccessibility00000000000000000000000000000011: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityAddTraits(.isStaticText)
    }
}

struct StellarView: View {
    @State private var selectedSize: String = "00000000-0000-0000-0000-000000000008"
    @State private var memo: String = ""
    @State private var isSaving: Bool = false
    let onAction: (String) -> Void

    init(onAction: @escaping (String) -> Void = { _ in }) {
        self.onAction = onAction
    }

    var body: some View {
        node00000000000000000000000000000001
    }

    private var node00000000000000000000000000000001: some View {
        VStack(alignment: .leading, spacing: 16) {
            node00000000000000000000000000000002
            node00000000000000000000000000000004
            node00000000000000000000000000000005
            node00000000000000000000000000000006
            node0000000000000000000000000000000b
            node0000000000000000000000000000000c
            node00000000000000000000000000000010
        }
        .padding(EdgeInsets(top: 24, leading: 24, bottom: 24, trailing: 24))
        .frame(width: 390, height: 844, alignment: .topLeading)
        .modifier(StellarAccessibility00000000000000000000000000000001())
    }

    private var node00000000000000000000000000000002: some View {
        Text(verbatim: "설정")
        .modifier(StellarAccessibility00000000000000000000000000000002())
    }

    private var node00000000000000000000000000000004: some View {
        Image(systemName: "person.crop.circle")
        .resizable()
        .scaledToFit()
        .frame(width: 64, height: 64, alignment: .topLeading)
        .modifier(StellarAccessibility00000000000000000000000000000004())
    }

    private var node00000000000000000000000000000005: some View {
        Color.clear
        .frame(width: 0, height: 0)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .frame(height: 1, alignment: .topLeading)
        .modifier(StellarAccessibility00000000000000000000000000000005())
    }

    private var node00000000000000000000000000000006: some View {
        HStack(alignment: .center, spacing: 8) {
            node00000000000000000000000000000007
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .modifier(StellarAccessibility00000000000000000000000000000006())
    }

    private var node00000000000000000000000000000007: some View {
        HStack(alignment: .center, spacing: 8) {
            node00000000000000000000000000000008
            node0000000000000000000000000000000a
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var node00000000000000000000000000000008: some View {
        Button {
            onAction("00000000-0000-0000-0000-00000000003d")
            selectedSize = "00000000-0000-0000-0000-000000000008"
        } label: {
            HStack(alignment: .top, spacing: 0) {
                node00000000000000000000000000000009
            }
            .fixedSize(horizontal: false, vertical: true)
            .frame(maxWidth: .infinity, alignment: .top)
            .padding(EdgeInsets(top: 12, leading: 16, bottom: 12, trailing: 16))
        }
        .buttonStyle(.plain)
        .modifier(StellarAccessibility00000000000000000000000000000008(selectedSize: $selectedSize, onAction: onAction))
    }

    private var node00000000000000000000000000000009: some View {
        Text(verbatim: "작게")
        .frame(maxHeight: .infinity, alignment: .topLeading)
        .modifier(StellarAccessibility00000000000000000000000000000009())
    }

    private var node0000000000000000000000000000000a: some View {
        Button {
            onAction("00000000-0000-0000-0000-00000000003e")
            selectedSize = "00000000-0000-0000-0000-00000000000a"
        } label: {
            Color.clear
            .frame(width: 0, height: 0)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .padding(EdgeInsets(top: 12, leading: 16, bottom: 12, trailing: 16))
        }
        .buttonStyle(.plain)
        .modifier(StellarAccessibility0000000000000000000000000000000a(selectedSize: $selectedSize, onAction: onAction))
    }

    private var node0000000000000000000000000000000b: some View {
        TextField(text: $memo, prompt: Text(verbatim: "메모")) {
            Text(verbatim: "메모")
        }
        .textFieldStyle(.plain)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .padding(EdgeInsets(top: 8, leading: 12, bottom: 8, trailing: 12))
        .frame(height: 44, alignment: .topLeading)
        .modifier(StellarAccessibility0000000000000000000000000000000b(memo: $memo))
    }

    private var node0000000000000000000000000000000c: some View {
        Button {
            onAction("00000000-0000-0000-0000-00000000003c")
        } label: {
            Color.clear
            .frame(width: 0, height: 0)
            .frame(maxWidth: .infinity, alignment: .topLeading)
            .frame(height: 48, alignment: .topLeading)
            .frame(minWidth: 120, alignment: .topLeading)
        }
        .buttonStyle(.plain)
        .modifier(StellarAccessibility0000000000000000000000000000000c(isSaving: $isSaving, onAction: onAction))
    }

    private var node00000000000000000000000000000010: some View {
        Color.clear
        .frame(width: 0, height: 0)
        .frame(width: 320, height: 120, alignment: .topLeading)
        .overlay(alignment: .topLeading) {
            node00000000000000000000000000000011
        }
        .modifier(StellarAccessibility00000000000000000000000000000010(onAction: onAction))
    }

    private var node00000000000000000000000000000011: some View {
        Text(verbatim: "저장되었습니다")
        .offset(x: 16, y: 16)
        .modifier(StellarAccessibility00000000000000000000000000000011())
    }
}
