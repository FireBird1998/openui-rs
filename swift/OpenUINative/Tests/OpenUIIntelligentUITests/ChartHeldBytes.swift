// Test-only: the bytes chart values hold in their heap buffers, summed as
// capacity × MemoryLayout.stride. It walks every stored property with Mirror,
// private ones included, so a buffer added to the chart data, its visibility
// or its geometry is counted without changing this file. A copy-on-write
// buffer reachable from two places, such as a row in both rows and
// firstRowsByCategory, is counted once.
//
// Not counted: array headers and malloc rounding, and strings of up to 15
// UTF-8 bytes, which are stored inline.
import Darwin

struct ChartHeldBytes {
    private var seen = Set<UnsafeRawPointer>()

    static func of(_ values: Any...) -> Int {
        var walk = ChartHeldBytes()
        return values.reduce(0) { $0 + walk.bytes($1) }
    }

    fileprivate mutating func bytes(_ value: Any) -> Int {
        if let buffer = value as? HeldBuffer { return buffer.heldBytes(&self) }
        if let string = value as? String { return string.utf8.count > 15 ? string.utf8.count : 0 }
        return Mirror(reflecting: value).children.reduce(0) { $0 + bytes($1.value) }
    }

    // False when this walk has already counted the buffer at this address.
    fileprivate mutating func firstVisit(_ address: UnsafeRawPointer?) -> Bool {
        address.map { seen.insert($0).inserted } ?? true
    }

    fileprivate mutating func arrayBytes<C: Collection>(_ elements: C, capacity: Int,
                                                         address: UnsafeRawPointer?) -> Int {
        guard capacity > 0, firstVisit(address) else { return 0 }
        return capacity * MemoryLayout<C.Element>.stride + elementBytes(elements)
    }

    // Native hash tables keep a power-of-two bucket count at a maximum load
    // factor of 3/4, with one occupancy bit per bucket. An empty table that
    // mapValues made is still a heap object, 96 bytes, that reports capacity
    // 0; each sparse row holds one. So read the table's real allocation where
    // the storage pointer is a malloc block. The shared empty table is static
    // and reads 0.
    // Nil when this walk has already counted the table.
    fileprivate mutating func hashTableBytes<Table>(_ table: Table, capacity: Int, entryStride: Int) -> Int? {
        var buckets = capacity > 0 ? 1 : 0
        while buckets * 3 / 4 < capacity { buckets <<= 1 }
        let layout = buckets * entryStride + (buckets + 63) / 64 * 8
        guard MemoryLayout<Table>.size == MemoryLayout<UnsafeRawPointer>.size,
              let storage = withUnsafeBytes(of: table, { $0.load(as: UnsafeRawPointer?.self) }) else { return layout }
        return firstVisit(storage) ? max(layout, malloc_size(storage)) : nil
    }

    fileprivate mutating func elementBytes<S: Sequence>(_ elements: S) -> Int {
        _isPOD(S.Element.self) ? 0 : elements.reduce(0) { $0 + bytes($1) }
    }
}

private protocol HeldBuffer {
    func heldBytes(_ walk: inout ChartHeldBytes) -> Int
}

extension Array: HeldBuffer {
    fileprivate func heldBytes(_ walk: inout ChartHeldBytes) -> Int {
        walk.arrayBytes(self, capacity: capacity, address: withUnsafeBufferPointer { UnsafeRawPointer($0.baseAddress) })
    }
}

extension ContiguousArray: HeldBuffer {
    fileprivate func heldBytes(_ walk: inout ChartHeldBytes) -> Int {
        walk.arrayBytes(self, capacity: capacity, address: withUnsafeBufferPointer { UnsafeRawPointer($0.baseAddress) })
    }
}

extension Dictionary: HeldBuffer {
    fileprivate func heldBytes(_ walk: inout ChartHeldBytes) -> Int {
        guard let table = walk.hashTableBytes(self, capacity: capacity,
                                              entryStride: MemoryLayout<Key>.stride + MemoryLayout<Value>.stride)
        else { return 0 }
        return table + walk.elementBytes(keys) + walk.elementBytes(values)
    }
}

extension Set: HeldBuffer {
    fileprivate func heldBytes(_ walk: inout ChartHeldBytes) -> Int {
        guard let table = walk.hashTableBytes(self, capacity: capacity, entryStride: MemoryLayout<Element>.stride)
        else { return 0 }
        return table + walk.elementBytes(self)
    }
}
