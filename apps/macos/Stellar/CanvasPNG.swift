import AppKit
import CryptoKit
import ImageIO
import UniformTypeIdentifiers

enum CanvasPNG {
    static let maxBytes = 32 * 1024 * 1024
    static let maxPixels = 16 * 1024 * 1024

    static func decode(_ data: Data) -> CGImage? {
        guard !data.isEmpty, data.count <= maxBytes,
              let source = CGImageSourceCreateWithData(data as CFData, nil),
              CGImageSourceGetType(source) == UTType.png.identifier as CFString,
              CGImageSourceGetCount(source) == 1, CGImageSourceGetStatus(source) == .statusComplete,
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int, let height = properties[kCGImagePropertyPixelHeight] as? Int,
              (1...16384).contains(width), (1...16384).contains(height), width * height <= maxPixels,
              let image = CGImageSourceCreateImageAtIndex(source, 0, [kCGImageSourceShouldCacheImmediately: true] as CFDictionary),
              image.width == width, image.height == height else { return nil }
        return image
    }

    static func picked(_ url: URL) -> Data? {
        guard let values = try? url.resourceValues(forKeys: [.isRegularFileKey, .isSymbolicLinkKey, .fileSizeKey]),
              values.isRegularFile == true, values.isSymbolicLink != true, (1...maxBytes).contains(values.fileSize ?? 0),
              let file = try? FileHandle(forReadingFrom: url) else { return nil }
        defer { try? file.close() }
        guard let encoded = try? file.read(upToCount: maxBytes + 1), decode(encoded) != nil else { return nil }
        return Data(SHA256.hash(data: encoded))
    }
}

/// PNG 픽셀은 직렬 worker에서 디코드하고, 그리기는 창별 캐시만 봅니다.
///
/// node별 마지막 내용 hash를 revision이 바뀌어도 유지하고, 새 revision에서 그 행을 한 번 다시 확인합니다.
/// 확인이 끝날 때까지 이전 픽셀을 그리므로 다른 node 편집으로 revision이 바뀌어도 깜박이지 않습니다. 내용 hash가
/// 바뀐 node만 새 픽셀로 바꿉니다.
final class CanvasPNGCache {
    private let images = NSCache<NSData, CGImage>()
    private let queue = DispatchQueue(label: "xyz.taeyoon.stellar.canvas.png", qos: .userInitiated)
    private var hashes: [UUID: Data] = [:]
    /// node별로 확인을 요청한 revision(같은 revision에서는 다시 요청하지 않음).
    private var checked: [UUID: UInt64] = [:]
    private var serial: UInt64 = 0
    init() { images.totalCostLimit = 64 * 1024 * 1024 }

    /// 창·View가 바뀌면 모두 버립니다.
    func reset() {
        serial &+= 1
        hashes.removeAll()
        checked.removeAll()
    }

    /// 새 표시 목록의 이미지 행만 남깁니다.
    func retain(_ nodes: Set<UUID>) {
        hashes = hashes.filter { nodes.contains($0.key) }
        checked = checked.filter { nodes.contains($0.key) }
    }

    func image(_ node: UUID) -> CGImage? {
        hashes[node].flatMap { images.object(forKey: $0 as NSData) }
    }

    func request(handle: UInt64, session: UInt64, revision: UInt64, row: Int, node: UUID, redraw: @escaping () -> Void) {
        guard checked[node] != revision else { return }
        checked[node] = revision
        let generation = serial
        let cache = images
        queue.async { [weak self] in
            var hash = [UInt8](repeating: 0, count: 32), count = 0
            var id = node.uuid
            let key: Data? = withUnsafeBytes(of: &id) { id in
                let pointer = id.bindMemory(to: UInt8.self).baseAddress
                guard stellar_window_canvas_png(handle, session, revision, UInt32(row), pointer, &hash, nil, 0, &count) == STELLAR_OK,
                      (1...CanvasPNG.maxBytes).contains(count) else { return nil }
                let key = Data(hash)
                if cache.object(forKey: key as NSData) == nil {
                    var bytes = Data(count: count)
                    let status = bytes.withUnsafeMutableBytes { buffer in
                        stellar_window_canvas_png(handle, session, revision, UInt32(row), pointer, &hash,
                            buffer.bindMemory(to: UInt8.self).baseAddress, buffer.count, &count)
                    }
                    guard status == STELLAR_OK, count == bytes.count, key == Data(hash),
                          let raster = CanvasPNG.decode(bytes) else { return nil }
                    cache.setObject(raster, forKey: key as NSData, cost: raster.bytesPerRow * raster.height)
                }
                return key
            }
            DispatchQueue.main.async { [weak self] in
                // 확인하지 못하면(행·revision이 그사이 바뀜 등) 이전 픽셀을 두고 다음 revision에서 다시 확인합니다.
                guard let self, self.serial == generation, self.checked[node] == revision,
                      let key, self.hashes[node] != key || self.image(node) == nil else { return }
                self.hashes[node] = key
                redraw()
            }
        }
    }
}
