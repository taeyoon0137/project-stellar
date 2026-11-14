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

/// PNG pixels are decoded on a serial worker; drawing only consults the per-window cache.
final class CanvasPNGCache {
    private let images = NSCache<NSData, CGImage>()
    private let queue = DispatchQueue(label: "xyz.taeyoon.stellar.canvas.png", qos: .userInitiated)
    private var hashes: [UUID: Data] = [:]
    private var queued = Set<UUID>()
    private var serial: UInt64 = 0
    init() { images.totalCostLimit = 64 * 1024 * 1024 }

    func reset() {
        serial &+= 1
        hashes.removeAll()
        queued.removeAll()
    }

    func image(_ node: UUID) -> CGImage? {
        hashes[node].flatMap { images.object(forKey: $0 as NSData) }
    }

    func request(handle: UInt64, session: UInt64, revision: UInt64, row: Int, node: UUID, redraw: @escaping () -> Void) {
        guard image(node) == nil, queued.insert(node).inserted else { return }
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
                guard let self, self.serial == generation else { return }
                if let key {
                    self.hashes[node] = key
                    self.queued.remove(node)
                    redraw()
                }
            }
        }
    }
}
