import SwiftUI

enum StellarColors {
}

struct StellarAccessibility00000000000000000000000000000002: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityHidden(true)
    }
}

struct StellarAccessibility00000000000000000000000000000005: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityHidden(true)
    }
}

struct StellarAccessibility00000000000000000000000000000008: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityHidden(true)
    }
}

struct StellarAccessibility0000000000000000000000000000000b: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityHidden(true)
    }
}

struct StellarAccessibility0000000000000000000000000000000c: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityHidden(true)
    }
}

struct StellarAccessibility0000000000000000000000000000000e: ViewModifier {
    func body(content: Content) -> some View {
        content
            .accessibilityHidden(true)
    }
}

struct StellarView: View {
    let onAction: (String) -> Void

    init(onAction: @escaping (String) -> Void = { _ in }) {
        self.onAction = onAction
    }

    var body: some View {
        node00000000000000000000000000000001
    }

    private var node00000000000000000000000000000001: some View {
        VStack(alignment: .leading, spacing: 8) {
            node00000000000000000000000000000002
            Spacer(minLength: 0)
            node00000000000000000000000000000005
            Spacer(minLength: 0)
            node00000000000000000000000000000008
            Spacer(minLength: 0)
            node0000000000000000000000000000000b
            Spacer(minLength: 0)
            node0000000000000000000000000000000e
        }
        .frame(width: 360, height: 640, alignment: .topLeading)
        .padding(EdgeInsets(top: 16, leading: 16, bottom: 16, trailing: 16))
    }

    private var node00000000000000000000000000000002: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            node00000000000000000000000000000003
            node00000000000000000000000000000004
        }
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .modifier(StellarAccessibility00000000000000000000000000000002())
    }

    private var node00000000000000000000000000000003: some View {
        Text(verbatim: "가")
    }

    private var node00000000000000000000000000000004: some View {
        Text(verbatim: "나")
        .fixedSize(horizontal: true, vertical: false)
    }

    private var node00000000000000000000000000000005: some View {
        HStack(alignment: .top, spacing: 0) {
            node00000000000000000000000000000007
            node00000000000000000000000000000006
        }
        .frame(maxWidth: .infinity, alignment: .topTrailing)
        .frame(height: 40, alignment: .topTrailing)
        .modifier(StellarAccessibility00000000000000000000000000000005())
    }

    private var node00000000000000000000000000000006: some View {
        Color.clear
        .frame(width: 0, height: 0)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var node00000000000000000000000000000007: some View {
        Color.clear
        .frame(width: 0, height: 0)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var node00000000000000000000000000000008: some View {
        Grid(alignment: .topLeading, horizontalSpacing: 8, verticalSpacing: 0) {
            GridRow {
                node00000000000000000000000000000009
                .frame(width: 100, height: 40, alignment: .topLeading)
                node0000000000000000000000000000000a
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .frame(height: 40, alignment: .topLeading)
                Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .frame(maxWidth: .infinity, alignment: .topLeading)
        .modifier(StellarAccessibility00000000000000000000000000000008())
    }

    private var node00000000000000000000000000000009: some View {
        Text(verbatim: "넓은 칸")
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var node0000000000000000000000000000000a: some View {
        Text(verbatim: "자동")
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private var node0000000000000000000000000000000b: some View {
        Color.clear
        .frame(width: 0, height: 0)
        .frame(width: 160, alignment: .topLeading)
        .aspectRatio(16 / 9, contentMode: .fit)
        .transformEffect(CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: 0, ty: 0))
        .offset(x: 8, y: 4)
        .modifier(StellarAccessibility0000000000000000000000000000000b())
    }

    private var node0000000000000000000000000000000e: some View {
        Color.clear
        .frame(width: 0, height: 0)
        .frame(width: 200, height: 100, alignment: .topLeading)
        .overlay(alignment: .bottomTrailing) {
            node0000000000000000000000000000000f
        }
        .overlay(alignment: .topLeading) {
            node00000000000000000000000000000010
        }
        .modifier(StellarAccessibility0000000000000000000000000000000e())
    }

    private var node0000000000000000000000000000000f: some View {
        Color.clear
        .frame(width: 0, height: 0)
        .frame(width: 20, height: 20, alignment: .topLeading)
        .offset(x: -8, y: -8)
    }

    private var node00000000000000000000000000000010: some View {
        Color.clear
        .frame(width: 0, height: 0)
        .frame(width: 10, height: 10, alignment: .topLeading)
    }
}
