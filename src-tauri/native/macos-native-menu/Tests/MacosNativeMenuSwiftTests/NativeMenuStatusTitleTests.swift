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

    private func title(_ value: String, prefix: String = "", stacked: Bool) -> NSAttributedString {
        menuBarQuotaTitle(accountPrefix: prefix, valueText: value, remainingPercent: 36, stacked: stacked)
    }
}
