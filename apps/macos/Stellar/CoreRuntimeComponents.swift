import Foundation

struct LibraryComponent: Equatable, Sendable {
    let id: UUID
    let root: UUID
    let nodeCount: UInt32
    let width: Float
    let height: Float
    let name: String
    let filename: String
    let hash: String
}

enum ComponentLibraryResult: Sendable {
    case loaded([LibraryComponent], legacyChunks: Int)
    case failed(String)
}

extension CoreRuntime {
    static func placeInstance(handle: UInt64, session: UInt64, view: UUID, revision: UInt64,
                              entry: LibraryComponent, x: Float, y: Float) -> Result<UInt64, CoreRuntimeCallError> {
        var request: UInt64 = 0
        let status = withUnsafeBytes(of: view.uuid) { viewBytes in
            withUnsafeBytes(of: entry.id.uuid) { componentBytes in
                Array(entry.hash.utf8).withUnsafeBufferPointer { hash in
                    stellar_window_place_component(handle, session, viewBytes.bindMemory(to: UInt8.self).baseAddress, revision,
                        componentBytes.bindMemory(to: UInt8.self).baseAddress, hash.baseAddress, hash.count, x, y, &request)
                }
            }
        }
        guard status == STELLAR_OK, request != 0 else { return .failure(CoreRuntimeCallError(status: status)) }
        return .success(request)
    }

    static func componentLibrary(handle: UInt64, session: UInt64) -> ComponentLibraryResult {
        precondition(!Thread.isMainThread)
        var buffer = [UInt8](repeating: 0, count: 16 * 1024)
        var length = 0
        // 두 호출 사이에 checkout으로 결과 크기가 바뀔 수 있습니다. 매 호출은 독립 snapshot입니다.
        for _ in 0..<3 {
            let status = buffer.withUnsafeMutableBufferPointer {
                stellar_window_component_library(handle, session, $0.baseAddress, $0.count, &length)
            }
            if status == STELLAR_BUFFER_TOO_SMALL, length > buffer.count, length <= 4 * 1024 * 1024 {
                buffer = [UInt8](repeating: 0, count: length)
                continue
            }
            guard status == STELLAR_OK, length <= buffer.count else { break }
            var reader = CoreRuntimeEvent.PayloadReader(bytes: buffer.prefix(length))
            guard let tag = reader.u8() else { break }
            if tag == 1, let message = reader.text(), reader.isAtEnd { return .failed(message) }
            guard tag == 0, let chunks = reader.u32(), let count = reader.u32(), count <= 4096 else { break }
            var rows: [LibraryComponent] = []
            for _ in 0..<count {
                guard let id = reader.uuid(), let root = reader.uuid(), let nodes = reader.u32(),
                      let width = reader.u32(), let height = reader.u32(), let name = reader.text(),
                      let filename = reader.text(), let hash = reader.text() else { break }
                rows.append(LibraryComponent(id: id, root: root, nodeCount: nodes, width: Float(bitPattern: width),
                                             height: Float(bitPattern: height), name: name, filename: filename, hash: hash))
            }
            if rows.count == Int(count), reader.isAtEnd { return .loaded(rows, legacyChunks: Int(chunks)) }
            break
        }
        return .failed(String(localized: "Couldn’t read the component library."))
    }
}

extension CoreRuntime {
    /// writer 결과는 단일 호출에서 수신합니다. 읽기처럼 작은 출력 버퍼로 쓰기를 재시도하지 않습니다.
    static func resizeComponent(handle: UInt64, session: UInt64, entry: LibraryComponent, width: Float, height: Float) -> String? {
        precondition(!Thread.isMainThread)
        var id = entry.id.uuid, length = 0
        var bytes = [UInt8](repeating: 0, count: Int(STELLAR_EVENT_TEXT_MAX))
        let status = withUnsafeBytes(of: &id) { id in
            Array(entry.hash.utf8).withUnsafeBufferPointer { hash in
                bytes.withUnsafeMutableBufferPointer { error in
                    stellar_window_component_resize(handle, session, id.bindMemory(to: UInt8.self).baseAddress,
                                                    hash.baseAddress, hash.count, width, height, error.baseAddress, error.count, &length)
                }
            }
        }
        guard status != STELLAR_OK else { return nil }
        return length > 0 && length <= bytes.count ? String(decoding: bytes.prefix(length), as: UTF8.self)
            : String(localized: "Couldn’t save the component size.")
    }
}
