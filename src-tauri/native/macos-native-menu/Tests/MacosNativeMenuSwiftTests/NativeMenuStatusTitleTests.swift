import AppKit
import XCTest
@testable import MacosNativeMenuSwift

// The production symbol is provided by Rust when linking the Tauri app.
@_cdecl("macos_native_menu_dispatch_action")
func testDispatchAction(_ action: UnsafePointer<CChar>?, _ platform: UnsafePointer<CChar>?, _ account: UnsafePointer<CChar>?) {}

final class NativeMenuStatusTitleTests: XCTestCase {
    func testTwoWindowsFitInTwoRowsAndUseLessWidth() {
        for value in ["5h 36% · Week 64%", "5h 36% · 周 64%"] {
            let horizontal = title(value, stacked: false)
            let compact = title(value, stacked: true)
            XCTAssertEqual(compact.string, value.components(separatedBy: " · ").map {
                $0.replacingOccurrences(of: " ", with: "\t")
            }.joined(separator: "\n"))
            XCTAssertLessThan(compact.size().width, horizontal.size().width)
            XCTAssertLessThanOrEqual(compact.size().height, 22)
            XCTAssertEqual((compact.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize, 10)
        }
    }

    func testWeeklyOnlyNeverInventsAnotherWindow() {
        for stacked in [false, true] {
            let weekly = title("周 64%", stacked: stacked)
            XCTAssertEqual(weekly.string, "周 64%")
            XCTAssertEqual((weekly.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize, NSFont.systemFontSize)
        }
    }

    func testAccountAndAPILabelsRemainVisibleWithCorrectColorRanges() {
        for prefix in ["user", "API", "测试😀"] {
            let result = title("5h 36% · 周 64%", prefix: prefix, stacked: true)
            XCTAssertEqual(result.string, "\(prefix) 5h\t36%\n周\t64%")
            XCTAssertEqual(result.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor, .labelColor)
            XCTAssertEqual(result.attribute(.foregroundColor, at: prefix.utf16.count + 1, effectiveRange: nil) as? NSColor, .systemOrange)
            XCTAssertEqual(result.attribute(.foregroundColor, at: result.length - 1, effectiveRange: nil) as? NSColor, .systemOrange)
        }
        for (value, prefix) in [("$12.34", "API"), ("周 64%", "user")] {
            let compact = title(value, prefix: prefix, stacked: true)
            let horizontal = title(value, prefix: prefix, stacked: false)
            XCTAssertTrue(compact.isEqual(to: horizontal))
        }
    }

    func testBothLabelAndAmountColumnsAlignWithDifferentDigitCounts() {
        for prefix in ["", "user", "测试😀", "API"] {
            let compact = title("5h 9% · 周 100%", prefix: prefix, stacked: true)
            let storage = NSTextStorage(attributedString: compact)
            let manager = NSLayoutManager()
            let container = NSTextContainer(size: NSSize(width: 400, height: 40))
            container.lineFragmentPadding = 0
            manager.addTextContainer(container)
            storage.addLayoutManager(manager)
            manager.ensureLayout(for: container)
            func position(_ text: String, last: Bool = false) -> CGFloat {
                let range = (compact.string as NSString).range(of: text)
                let glyphs = manager.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
                let index = last ? NSMaxRange(glyphs) - 1 : glyphs.location
                return manager.location(forGlyphAt: index).x
                    + manager.lineFragmentRect(forGlyphAt: index, effectiveRange: nil).origin.x
            }
            XCTAssertEqual(position("5h"), position("周"), accuracy: 0.1)
            XCTAssertEqual(position("9%", last: true), position("100%", last: true), accuracy: 0.1)
            XCTAssertLessThanOrEqual(compact.size().height, 22)
        }
    }

    func testMissingZeroAndPooledQuotaValues() {
        for (value, percent, color) in [("", Int32(-1), NSColor.secondaryLabelColor), ("Week 0%", 0, .systemRed), ("Week 140%", 140, .systemGreen)] {
            let result = menuBarQuotaTitle(accountPrefix: "", valueText: value, remainingPercent: percent, stacked: true)
            XCTAssertEqual(result.string, value.isEmpty ? "--" : value)
            XCTAssertEqual(result.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor, color)
        }
    }

    func testCompactTitleIsCenteredAtDifferentMenuBarHeightsAndPixelScales() {
        for height: CGFloat in [22, 24, 28, 32, 37, 48, 64] {
            let compact = menuBarQuotaTitle(accountPrefix: "API", valueText: "5h 9% · 周 100%",
                remainingPercent: 9, stacked: true, availableHeight: height)
            let bounds = NSRect(x: 0, y: 0, width: 160, height: height)
            for scale: CGFloat in [1, 2] {
                // The native cell can return a baseline-biased title rect. Only
                // its horizontal placement is relevant to the compact renderer.
                let nativeRect = NSRect(x: 24, y: -5, width: 110, height: 16)
                let rect = NativeMenuQuotaTitleView.drawingRect(bounds: bounds,
                    titleRect: nativeRect, textSize: compact.size(), scale: scale)
                XCTAssertEqual(rect.midY, bounds.midY, accuracy: 0.5 / scale)
                XCTAssertEqual(rect.minX, 24)
                XCTAssertGreaterThanOrEqual(rect.minY, 0)
                XCTAssertLessThanOrEqual(rect.maxY, height)
                XCTAssertEqual(rect.minY * scale, (rect.minY * scale).rounded())
            }
        }
    }

    func testCompactTitleViewRespondsToHeightChangesWithoutTakingMouseEvents() {
        let button = NSButton(frame: NSRect(x: 0, y: 0, width: 140, height: 22))
        let view = NativeMenuQuotaTitleView(button: button)
        button.addSubview(view)
        view.configure(accountPrefix: "", valueText: "5h 36% · 周 64%", remainingPercent: 36)
        XCTAssertEqual((button.attributedTitle.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize, 10)
        XCTAssertEqual(button.attributedTitle.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor, .clear)
        button.setFrameSize(NSSize(width: 140, height: 37))
        XCTAssertEqual(view.bounds.height, 37)
        XCTAssertEqual((button.attributedTitle.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize, 11)
        button.setFrameSize(NSSize(width: 140, height: 22))
        XCTAssertEqual((button.attributedTitle.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize, 10)
        XCTAssertNil(view.hitTest(NSPoint(x: 10, y: 10)))
    }

    func testRenderedQuotaPixelsStayCenteredInShortAndTallButtons() throws {
        for height in [22, 24, 28, 37, 48] {
            for scale in [1, 2] {
                let button = NSButton(frame: NSRect(x: 0, y: 0, width: 140, height: height))
                button.appearance = NSAppearance(named: .aqua)
                button.isBordered = false
                button.cell?.wraps = true
                button.cell?.usesSingleLineMode = false
                let view = NativeMenuQuotaTitleView(button: button)
                button.addSubview(view)
                view.configure(accountPrefix: "", valueText: "5h 59% · 周 52%", remainingPercent: 52)
                let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil,
                    pixelsWide: 140 * scale, pixelsHigh: height * scale,
                    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                    colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
                bitmap.size = button.bounds.size
                button.cacheDisplay(in: button.bounds, to: bitmap)
                var inkRows: [Int] = []
                for y in 0..<bitmap.pixelsHigh {
                    for x in 0..<bitmap.pixelsWide {
                        if let color = bitmap.colorAt(x: x, y: y)?.usingColorSpace(.deviceRGB),
                            color.alphaComponent > 0.5, color.redComponent > 0.8,
                            color.greenComponent > 0.2, color.greenComponent < 0.85,
                            color.blueComponent < 0.3 {
                            inkRows.append(y)
                            break
                        }
                    }
                }
                let top = try XCTUnwrap(inkRows.min())
                let bottom = try XCTUnwrap(inkRows.max())
                let inkCenter = CGFloat(top + bottom + 1) / CGFloat(2 * scale)
                XCTAssertEqual(inkCenter, CGFloat(height) / 2, accuracy: 1,
                    "height=\(height), scale=\(scale)")
            }
        }
    }

    private func title(_ value: String, prefix: String = "", stacked: Bool) -> NSAttributedString {
        menuBarQuotaTitle(accountPrefix: prefix, valueText: value, remainingPercent: 36, stacked: stacked)
    }
}
