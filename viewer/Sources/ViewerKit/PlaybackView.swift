import AppKit
import SwiftUI

/// The playback controls, where each update's time went, the drawing, and
/// the program's text with what's been received so far.
public struct PlaybackView: View {
    @Bindable var playback: Playback
    @Binding var chunking: Chunking
    let chunkings: [Chunking]
    let program: String
    @Environment(ElementSelection.self) private var selection: ElementSelection?
    /// The drawing's width in points; 0 fits the window.
    @AppStorage("drawingWidth") private var drawingWidth = 0.0

    static let widths: [(label: String, points: Double)] = [("Window", 0), ("Phone", 390), ("Tablet", 820)]

    static let speeds: [Double] = [1, 2, 5, 10, 30, 60, 120, 300, 1000, 10000]

    public init(playback: Playback, chunking: Binding<Chunking>, chunkings: [Chunking], program: String) {
        self.playback = playback
        _chunking = chunking
        self.chunkings = chunkings
        self.program = program
    }

    public var body: some View {
        VStack(spacing: 0) {
            controls.padding(.horizontal).padding(.vertical, 8)
            Divider()
            VSplitView {
                drawing.frame(minHeight: 120)
                ProgramTextView(
                    text: program,
                    received: playback.textOffsets[playback.position],
                    previous: playback.textOffsets[max(playback.position - 1, 0)]
                )
                .frame(minHeight: 60, idealHeight: 140)
            }
        }
    }

    private var controls: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                Button("Step back", systemImage: "backward.frame") { playback.stepBack() }
                    .disabled(playback.position == 0)
                Button(playback.isPlaying ? "Pause" : "Play", systemImage: playback.isPlaying ? "pause.fill" : "play.fill") {
                    playback.isPlaying ? playback.pause() : playback.play()
                }
                .disabled(playback.chunks.isEmpty)
                Button("Step forward", systemImage: "forward.frame") { playback.stepForward() }
                    .disabled(playback.position >= playback.chunks.count)
                Slider(
                    value: Binding(
                        get: { Double(playback.position) },
                        set: { playback.seek(to: Int($0.rounded())) }
                    ),
                    in: 0...Double(max(playback.chunks.count, 1)),
                    step: 1
                )
                .disabled(playback.chunks.isEmpty)
                Text("\(playback.position) / \(playback.chunks.count)")
                    .monospacedDigit()
                    .frame(minWidth: 90, alignment: .trailing)
            }
            .labelStyle(.iconOnly)
            HStack(spacing: 16) {
                Picker("Chunk", selection: $chunking) {
                    ForEach(chunkings, id: \.self) { chunking in
                        Text(chunking.label).tag(chunking)
                    }
                }
                .fixedSize()
                Picker("Speed", selection: $playback.speed) {
                    ForEach(Self.speeds, id: \.self) { speed in
                        Text(speed >= 10000 ? "As fast as possible" : "\(Int(speed)) chunks/s").tag(speed)
                    }
                }
                .fixedSize()
                Picker("Width", selection: $drawingWidth) {
                    ForEach(Self.widths, id: \.points) { width in
                        Text(width.points > 0 ? "\(width.label) (\(Int(width.points)) pt)" : width.label).tag(width.points)
                    }
                }
                .fixedSize()
                Spacer()
            }
            TimingsLine(timings: playback.timings, fromCache: playback.fromCache)
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
            if let error = playback.error {
                Text(error).foregroundStyle(.red).font(.callout)
            }
        }
    }

    /// Room kept beside the drawing for a vertical scroll bar.
    static let scrollBarRoom: CGFloat = 16

    @ViewBuilder private var drawing: some View {
        // The drawing's width comes from the pane, less room for a scroll bar,
        // never from the space beside the bar. Otherwise, with scroll bars
        // always shown, a drawing just the window's height can loop: the bar
        // appears, text wraps narrower and grows taller, the bar goes, text
        // grows shorter, until AppKit gives up on the window's layout.
        GeometryReader { pane in
            ScrollViewReader { proxy in
                ScrollView(.vertical) {
                    Group {
                        if let root = playback.step?.result.root {
                            ElementView(element: root)
                        } else if playback.step != nil {
                            Text("No root element yet").foregroundStyle(.secondary)
                        }
                    }
                    .padding()
                    .frame(
                        width: drawingWidth > 0 ? drawingWidth : max(pane.size.width - Self.scrollBarRoom, 120),
                        alignment: .topLeading
                    )
                    .background(RedrawProbe(stamp: playback.stamp) { playback.drawn($0) })
                    .frame(width: max(pane.size.width - Self.scrollBarRoom, 120), alignment: drawingWidth > 0 ? .top : .topLeading)
                }
                .onChange(of: selection?.revealInDrawing) {
                    guard let path = selection?.path else { return }
                    // Lazy stacks only build what's been on screen, so scroll
                    // through the element's ancestors, outermost first: each
                    // step builds the next. Paths that aren't elements, such as
                    // `/children`, scroll nowhere.
                    Task {
                        for ancestor in ElementPath.ancestors(of: path) + [path] {
                            withAnimation { proxy.scrollTo(ancestor, anchor: .center) }
                            try? await Task.sleep(for: .milliseconds(60))
                        }
                    }
                }
            }
        }
    }
}

/// "parse 0.12 ms · crossing 0.03 ms · decode 0.20 ms · redraw 4.1 ms"
struct TimingsLine: View {
    let timings: Timings?
    let fromCache: Bool

    var body: some View {
        if let t = timings {
            HStack(spacing: 4) {
                if fromCache {
                    Text("kept step:").foregroundStyle(.secondary)
                } else if t.chunks > 1 {
                    Text("\(t.chunks) chunks:").foregroundStyle(.secondary)
                }
                Text(
                    "parse \(Self.ms(t.parse)) · crossing \(Self.ms(t.crossing)) · decode \(Self.ms(t.decode))"
                        + " · redraw \(t.redraw.map(Self.ms) ?? "…")"
                )
            }
            .font(.caption.monospacedDigit())
            .help("Parse: the core. Crossing: serializing to JSON and into Swift. Decode: JSON into the viewer's model. Redraw: until the step was on screen.")
        }
    }

    static func ms(_ d: Duration) -> String {
        String(format: "%.2f ms", d.inSeconds * 1000)
    }
}

/// Reports when the content it sits behind is drawn with a new stamp: an
/// invisible view drawn in the same pass as the content.
struct RedrawProbe: NSViewRepresentable {
    let stamp: Int
    let drawn: @MainActor (Int) -> Void

    func makeNSView(context: Context) -> ProbeView { ProbeView() }

    func updateNSView(_ view: ProbeView, context: Context) {
        view.drawn = drawn
        if view.stamp != stamp {
            view.stamp = stamp
            view.needsDisplay = true
        }
    }

    final class ProbeView: NSView {
        var stamp = 0
        var reported = -1
        var drawn: (@MainActor (Int) -> Void)?

        override func draw(_ dirtyRect: NSRect) {
            guard stamp != reported else { return }
            reported = stamp
            drawn?(stamp)
        }
    }
}
