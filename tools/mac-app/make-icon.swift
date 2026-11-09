// Draws KONTRA's macOS app icon: the project's mark (KONTRA/website/assets/icon.svg,
// a 20×18 frame split down the middle) in its pink, on a graphite tile.
// Usage: swift make-icon.swift out.png   (writes a 1024×1024 PNG)
import AppKit

let size = 1024
let out = CommandLine.arguments.dropFirst().first ?? "icon-1024.png"

guard
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0),
    let ctx = NSGraphicsContext(bitmapImageRep: rep)?.cgContext,
    let srgb = CGColorSpace(name: CGColorSpace.sRGB)
else {
    FileHandle.standardError.write("could not create a drawing context\n".data(using: .utf8)!)
    exit(1)
}

func rgb(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(
        srgbRed: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255,
        blue: CGFloat(hex & 0xff) / 255, alpha: alpha)
}

// Apple's macOS icon grid: an 824 px tile centred on the 1024 canvas, corner radius ~185.
let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
let shape = CGPath(roundedRect: tile, cornerWidth: 185, cornerHeight: 185, transform: nil)

// Soft shadow under the tile.
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: rgb(0x000000, 0.35))
ctx.addPath(shape)
ctx.setFillColor(rgb(0x1e1f21))
ctx.fillPath()
ctx.restoreGState()

// Graphite gradient, lighter at the top like the app's title bar.
ctx.saveGState()
ctx.addPath(shape)
ctx.clip()
if let gradient = CGGradient(
    colorsSpace: srgb, colors: [rgb(0x34363a), rgb(0x1a1b1d)] as CFArray, locations: [0, 1])
{
    ctx.drawLinearGradient(
        gradient, start: CGPoint(x: 512, y: tile.maxY), end: CGPoint(x: 512, y: tile.minY),
        options: [])
}
ctx.restoreGState()

// Hairline highlight on the tile's edge.
ctx.addPath(shape)
ctx.setStrokeColor(rgb(0xffffff, 0.08))
ctx.setLineWidth(3)
ctx.strokePath()

// The mark, from icon.svg's 26-unit viewBox: "M3 4h20v18H3zM13 4v18", stroke 2, #e6b6c7.
let unit: CGFloat = 22
let origin = CGPoint(x: 512 - 13 * unit, y: 512 - 13 * unit)
func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {  // SVG y runs down; Core Graphics y runs up.
    CGPoint(x: origin.x + x * unit, y: origin.y + (26 - y) * unit)
}
ctx.setStrokeColor(rgb(0xe6b6c7))
ctx.setLineWidth(2 * unit)
ctx.setLineJoin(.miter)
let corner = point(3, 22)
ctx.stroke(CGRect(x: corner.x, y: corner.y, width: 20 * unit, height: 18 * unit))
ctx.move(to: point(13, 4))
ctx.addLine(to: point(13, 22))
ctx.strokePath()

guard let png = rep.representation(using: .png, properties: [:]) else { exit(1) }
do {
    try png.write(to: URL(fileURLWithPath: out))
} catch {
    FileHandle.standardError.write("could not write \(out): \(error)\n".data(using: .utf8)!)
    exit(1)
}
