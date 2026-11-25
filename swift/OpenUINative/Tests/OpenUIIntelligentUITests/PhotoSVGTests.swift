@testable import OpenUIIntelligentUI
import AppKit
import Testing

let redSVGRectangle = #"<rect width="64" height="32" fill="red"/>"#

func svgDocument(_ body: String, size: String = #"width="64" height="32""#) -> String {
    """
    <svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" \(size)>\(body)</svg>
    """
}

@Suite struct PhotoSVGTests {
    @Test(arguments: ["bytes", "nodes", "depth", "id", "viewBox", "inline-declarations"])
    func aStaticSVGDrawsAtItsCapAndIsRefusedOverIt(_ cap: String) throws {
        let size = try #require(PhotoSize(points: CGSize(width: 64, height: 32), scale: 1))
        for excess in 0...1 {
            let xml: String
            switch cap {
            case "bytes":
                let emptyComment = svgDocument("<!---->" + redSVGRectangle)
                let padding = String(repeating: "a", count: 256 * 1024 - emptyComment.utf8.count + excess)
                xml = svgDocument("<!--\(padding)-->" + redSVGRectangle)
            case "nodes":
                xml = svgDocument(String(repeating: "<g/>", count: 510 + excess) + redSVGRectangle)
            case "depth":
                let count = 30 + excess
                xml = svgDocument(String(repeating: "<g>", count: count) + redSVGRectangle
                                  + String(repeating: "</g>", count: count))
            case "viewBox":
                let width = 1_000_000 + excess
                xml = svgDocument(#"<rect width="1000000" height="500000" fill="red"/>"#,
                                  size: #"width="64" height="32" viewBox="0 0 \#(width) 500000""#)
            case "inline-declarations":
                let declarations = String(repeating: "fill:red;", count: 53 + excess)
                xml = svgDocument(#"<rect width="64" height="32" style="\#(declarations)"/>"#)
            default:
                let id = String(repeating: "a", count: 128 + excess)
                xml = svgDocument("""
                <defs><linearGradient id="\(id)"><stop offset="0" stop-color="red"/>
                <stop offset="1" stop-color="red"/></linearGradient></defs>
                <rect width="64" height="32" fill="url(#\(id))"/>
                """)
            }
            let raster = PhotoSVG.decode(Data(xml.utf8), size: size)
            if excess == 0 {
                let image = try #require(raster)
                let pixel = try #require(NSBitmapImageRep(cgImage: image).colorAt(x: 20, y: 15))
                #expect(pixel.redComponent > 0.85 && pixel.blueComponent < 0.15 && pixel.alphaComponent > 0.85)
            } else {
                #expect(raster == nil, "Over-limit SVG drew for \(cap)")
            }
        }
    }

    @Test func stripsOnlyTheEntityFreeDoctype() throws {
        let size = try #require(PhotoSize(points: CGSize(width: 64, height: 32), scale: 1))
        let body = svgDocument(redSVGRectangle)
        let bare = "<?xml version=\"1.0\"?>\n<!-- exported -->\n<!DOCTYPE\t svg >\n"
        let image = try #require(PhotoSVG.decode(Data((bare + body).utf8), size: size))
        let pixel = try #require(NSBitmapImageRep(cgImage: image).colorAt(x: 20, y: 15))
        #expect(pixel.redComponent > 0.85 && pixel.blueComponent < 0.15 && pixel.alphaComponent > 0.85)
    }

    @Test(arguments: [
        "<!DOCTYPE svg []>",
        "<!DOCTYPE svg [<!ENTITY unused 'red'>]>",
        "<!DOCTYPE svg SYSTEM 'file:///nonexistent-openui-test.dtd'>",
        "<!DOCTYPE svg PUBLIC '-//W3C//DTD SVG 1.1//EN' 'file:///nonexistent-openui-test.dtd'>",
        "<!DOCTYPE svg><!DOCTYPE svg>",
    ])
    func refusesOtherDoctypes(_ header: String) throws {
        let size = try #require(PhotoSize(points: CGSize(width: 64, height: 32), scale: 1))
        let xml = header + svgDocument(redSVGRectangle)
        #expect(PhotoSVG.decode(Data(xml.utf8), size: size) == nil)
    }
}
