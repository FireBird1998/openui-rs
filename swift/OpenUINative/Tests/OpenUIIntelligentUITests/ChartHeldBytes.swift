// Test-only: the bytes chart values hold in their heap buffers. It walks
// every stored property with Mirror, private ones included, so a buffer added
// to the chart data, its visibility or its geometry is counted without
// changing this file. A copy-on-write buffer reachable from two places, such
// as a row in both rows and firstRowsByCategory, is counted once, in both
// figures.
//
// Two figures come from the same walk:
// - used: what the chart stores, which depends only on its input.
//   Array and ContiguousArray: count × MemoryLayout<Element>.stride.
//   Dictionary: count (occupied entries) × (key stride + value stride).
//   Set: count (occupied entries) × element stride.
//   String: its UTF-8 length when over 15 bytes, else 0 (stored inline).
//   No bucket bitmaps, empty buckets, spare capacity or empty tables.
// - capacity: what the buffers reserve, which follows malloc's block sizes.
//   Arrays: capacity × stride. Hash tables: the power-of-two bucket count
//   for their capacity × entry stride plus one occupancy bit per bucket, or
//   malloc_size of the table where that is larger (an empty table that
//   mapValues made is a 96-byte heap object reporting capacity 0). Strings
//   count as in used.
//
// Neither counts array headers or malloc rounding of array buffers. Mirror
// can't see into NSObject or CF types, closures, unsafe pointers,
// ManagedBuffer contents, superclass properties or slice and substring bases.
import Darwin

struct ChartHeldBytes {
    struct Held {
        var used = 0
        var capacity = 0

        static func + (a: Held, b: Held) -> Held {
            Held(used: a.used + b.used, capacity: a.capacity + b.capacity)
        }
    }

    private var seen = Set<UnsafeRawPointer>()

    static func of(_ values: Any...) -> Held {
        var walk = ChartHeldBytes()
        return values.reduce(Held()) { $0 + walk.bytes($1) }
    }

    fileprivate mutating func bytes(_ value: Any) -> Held {
        if let buffer = value as? HeldBuffer { return buffer.heldBytes(&self) }
        if let string = value as? String {
            let length = string.utf8.count > 15 ? string.utf8.count : 0
            return Held(used: length, capacity: length)
        }
        return Mirror(reflecting: value).children.reduce(Held()) { $0 + bytes($1.value) }
    }

    // False when this walk has already counted the buffer at this address.
    fileprivate mutating func firstVisit(_ address: UnsafeRawPointer?) -> Bool {
        address.map { seen.insert($0).inserted } ?? true
    }

    fileprivate mutating func arrayBytes<C: Collection>(_ elements: C, capacity: Int,
                                                         address: UnsafeRawPointer?) -> Held {
        guard capacity > 0, firstVisit(address) else { return Held() }
        let stride = MemoryLayout<C.Element>.stride
        return Held(used: elements.count * stride, capacity: capacity * stride) + elementBytes(elements)
    }

    // Native hash tables keep a power-of-two bucket count at a maximum load
    // factor of 3/4, with one occupancy bit per bucket. Read the table's real
    // allocation where the storage pointer is a malloc block; the shared
    // empty table is static and reads 0.
    // Nil when this walk has already counted the table.
    fileprivate mutating func hashTableBytes<Table>(_ table: Table, count: Int, capacity: Int,
                                                    entryStride: Int) -> Held? {
        var buckets = capacity > 0 ? 1 : 0
        while buckets * 3 / 4 < capacity { buckets <<= 1 }
        let layout = buckets * entryStride + (buckets + 63) / 64 * 8
        guard MemoryLayout<Table>.size == MemoryLayout<UnsafeRawPointer>.size,
              let storage = withUnsafeBytes(of: table, { $0.load(as: UnsafeRawPointer?.self) })
        else { return Held(used: count * entryStride, capacity: layout) }
        guard firstVisit(storage) else { return nil }
        return Held(used: count * entryStride, capacity: max(layout, malloc_size(storage)))
    }

    fileprivate mutating func elementBytes<S: Sequence>(_ elements: S) -> Held {
        _isPOD(S.Element.self) ? Held() : elements.reduce(Held()) { $0 + bytes($1) }
    }
}

private protocol HeldBuffer {
    func heldBytes(_ walk: inout ChartHeldBytes) -> ChartHeldBytes.Held
}

extension Array: HeldBuffer {
    fileprivate func heldBytes(_ walk: inout ChartHeldBytes) -> ChartHeldBytes.Held {
        walk.arrayBytes(self, capacity: capacity, address: withUnsafeBufferPointer { UnsafeRawPointer($0.baseAddress) })
    }
}

extension ContiguousArray: HeldBuffer {
    fileprivate func heldBytes(_ walk: inout ChartHeldBytes) -> ChartHeldBytes.Held {
        walk.arrayBytes(self, capacity: capacity, address: withUnsafeBufferPointer { UnsafeRawPointer($0.baseAddress) })
    }
}

extension Dictionary: HeldBuffer {
    fileprivate func heldBytes(_ walk: inout ChartHeldBytes) -> ChartHeldBytes.Held {
        guard let table = walk.hashTableBytes(self, count: count, capacity: capacity,
                                              entryStride: MemoryLayout<Key>.stride + MemoryLayout<Value>.stride)
        else { return ChartHeldBytes.Held() }
        return table + walk.elementBytes(keys) + walk.elementBytes(values)
    }
}

extension Set: HeldBuffer {
    fileprivate func heldBytes(_ walk: inout ChartHeldBytes) -> ChartHeldBytes.Held {
        guard let table = walk.hashTableBytes(self, count: count, capacity: capacity,
                                              entryStride: MemoryLayout<Element>.stride)
        else { return ChartHeldBytes.Held() }
        return table + walk.elementBytes(self)
    }
}
