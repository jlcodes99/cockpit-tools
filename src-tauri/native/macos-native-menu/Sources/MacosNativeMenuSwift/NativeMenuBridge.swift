import AppKit
import CoreFoundation
import Foundation

@_silgen_name("macos_native_menu_dispatch_action")
private func rustDispatchMenuAction(
    _ action: UnsafePointer<CChar>?,
    _ platformId: UnsafePointer<CChar>?,
    _ accountId: UnsafePointer<CChar>?
)

private func runNativeMenuControllerSync(
    label: String,
    _ operation: @escaping @MainActor () -> Void
) {
    precondition(Thread.isMainThread, "[NativeMenu] \(label) 必须在主线程执行")
    MainActor.assumeIsolated {
        operation()
    }
}

private let nativeMenuRunLoopModes: [RunLoop.Mode] = [
    .default,
    .eventTracking,
    .modalPanel,
]

private func runNativeMenuController(
    label: String,
    _ operation: @escaping @MainActor () -> Void
) {
    if Thread.isMainThread {
        runNativeMenuControllerSync(label: label, operation)
        return
    }

    RunLoop.main.perform(inModes: nativeMenuRunLoopModes) {
        runNativeMenuControllerSync(label: label, operation)
    }
    CFRunLoopWakeUp(CFRunLoopGetMain())
}

func dispatchRustMenuAction(action: String, platformId: String? = nil, accountId: String? = nil) {
    action.withCString { actionPointer in
        if let platformId {
            platformId.withCString { platformPointer in
                if let accountId {
                    accountId.withCString { accountPointer in
                        rustDispatchMenuAction(actionPointer, platformPointer, accountPointer)
                    }
                } else {
                    rustDispatchMenuAction(actionPointer, platformPointer, nil)
                }
            }
        } else {
            rustDispatchMenuAction(actionPointer, nil, nil)
        }
    }
}

@_cdecl("macos_native_menu_toggle")
public func macos_native_menu_toggle(
    snapshotJSONPointer: UnsafePointer<CChar>?,
    statusItemPointer: UnsafeMutableRawPointer?
) {
    guard let snapshotJSONPointer, let statusItemPointer else { return }
    let snapshotJSON = String(cString: snapshotJSONPointer)
    runNativeMenuController(label: "toggle") {
        NativeMenuPopoverController.shared.toggle(
            snapshotJSON: snapshotJSON,
            statusItemPointer: statusItemPointer
        )
    }
}

@_cdecl("macos_native_menu_update_snapshot")
public func macos_native_menu_update_snapshot(
    snapshotJSONPointer: UnsafePointer<CChar>?
) {
    guard let snapshotJSONPointer else { return }
    let snapshotJSON = String(cString: snapshotJSONPointer)
    runNativeMenuController(label: "update_snapshot") {
        NativeMenuPopoverController.shared.update(snapshotJSON: snapshotJSON)
    }
}

/// tray-icon 在 NSStatusBarButton 上叠加 `TaoTrayTarget` 子视图接收鼠标事件。
/// 直接改 button 的 title/attributedTitle 会拉宽状态栏项，但不会同步放大该点击层，
/// 结果是图标/文字区域“看起来在”，左右键却完全无响应。
func syncTrayClickTargetFrame(for button: NSStatusBarButton) {
    button.layoutSubtreeIfNeeded()
    let bounds = button.bounds
    guard bounds.width > 0, bounds.height > 0 else { return }

    var matched = false
    for subview in button.subviews {
        let className = NSStringFromClass(type(of: subview))
        // tray-icon 注册名为 TaoTrayTarget；兼容可能的命名变化。
        if className.contains("TaoTrayTarget") || className.contains("TrayTarget") {
            subview.frame = bounds
            subview.autoresizingMask = [.width, .height]
            matched = true
        }
    }

    // 兜底：若类名匹配失败，只同步覆盖整个 button 的透明事件层（通常仅此一个子视图）。
    if !matched, button.subviews.count == 1, let only = button.subviews.first {
        only.frame = bounds
        only.autoresizingMask = [.width, .height]
    }
}

func syncTrayClickTargetFrameSoon(for button: NSStatusBarButton) {
    syncTrayClickTargetFrame(for: button)
    // 状态栏变宽后 bounds 有时会在下一帧才稳定，补一次异步对齐。
    DispatchQueue.main.async {
        syncTrayClickTargetFrame(for: button)
    }
}

@_cdecl("macos_native_menu_update_status_item")
public func macos_native_menu_update_status_item(
    accountPrefixPointer: UnsafePointer<CChar>?,
    valueTextPointer: UnsafePointer<CChar>?,
    remainingPercent: Int32,
    enabled: Int32,
    stacked: Int32,
    statusItemPointer: UnsafeMutableRawPointer?
) {
    guard let statusItemPointer else { return }
    let accountPrefix = accountPrefixPointer.map(String.init(cString:)) ?? ""
    let valueTextRaw = valueTextPointer.map(String.init(cString:)) ?? ""

    runNativeMenuController(label: "update_status_item") {
        let statusItem = Unmanaged<NSStatusItem>
            .fromOpaque(statusItemPointer)
            .takeUnretainedValue()
        guard let button = statusItem.button else { return }

        statusItem.length = NSStatusItem.variableLength
        guard enabled != 0 else {
            button.subviews.filter { $0 is NativeMenuQuotaTitleView }.forEach { $0.removeFromSuperview() }
            button.attributedTitle = NSAttributedString(string: "")
            button.title = ""
            button.imagePosition = .imageOnly
            button.cell?.wraps = false
            button.cell?.usesSingleLineMode = true
            button.setAccessibilityValue(nil)
            button.needsDisplay = true
            syncTrayClickTargetFrameSoon(for: button)
            return
        }

        let attributedTitle = menuBarQuotaTitle(
            accountPrefix: accountPrefix,
            valueText: valueTextRaw,
            remainingPercent: remainingPercent,
            stacked: stacked != 0,
            availableHeight: button.bounds.height
        )
        let twoLines = attributedTitle.string.contains("\n")
        button.cell?.wraps = twoLines
        button.cell?.usesSingleLineMode = !twoLines
        button.imagePosition = .imageLeading
        button.attributedTitle = attributedTitle
        if twoLines {
            let titleView = button.subviews.compactMap { $0 as? NativeMenuQuotaTitleView }.first
                ?? NativeMenuQuotaTitleView(button: button)
            if titleView.superview == nil { button.addSubview(titleView) }
            titleView.frame = button.bounds
            titleView.configure(accountPrefix: accountPrefix, valueText: valueTextRaw,
                remainingPercent: remainingPercent)
        } else {
            button.subviews.filter { $0 is NativeMenuQuotaTitleView }.forEach { $0.removeFromSuperview() }
        }
        button.setAccessibilityValue(attributedTitle.string
            .replacingOccurrences(of: "\n", with: ", ")
            .replacingOccurrences(of: "\t", with: " "))
        button.needsDisplay = true
        // 标题变宽后必须立刻同步点击层，否则左右键全部失效。
        syncTrayClickTargetFrameSoon(for: button)
    }
}
