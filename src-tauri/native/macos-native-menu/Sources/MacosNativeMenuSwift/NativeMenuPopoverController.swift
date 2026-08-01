import AppKit
import Combine
import SwiftUI

@MainActor
final class NativeMenuPopoverController: NSObject, ObservableObject, NSMenuDelegate {
    static let shared = NativeMenuPopoverController()

    @Published private(set) var snapshot: NativeMenuSnapshot?
    @Published private(set) var selectedPlatformId: String = UserDefaults.standard.string(
        forKey: "cockpit-tools.native-menu.selected-platform"
    ) ?? ""
    @Published private(set) var viewedAccountIds: [String: String] = [:]
    @Published private(set) var refreshingPlatformId: String?
    @Published private(set) var refreshingAccountId: String?
    @Published private(set) var renderRevision: Int = 0

    private weak var statusItem: NSStatusItem?
    private var menu: NSMenu?
    private var refreshStartedAt: Date?
    private var clearRefreshTask: Task<Void, Never>?

    func toggle(snapshotJSON: String, statusItemPointer: UnsafeMutableRawPointer) {
        guard let snapshot = self.decodeSnapshot(from: snapshotJSON) else { return }

        let statusItem = Unmanaged<NSStatusItem>.fromOpaque(statusItemPointer).takeUnretainedValue()
        self.statusItem = statusItem
        self.apply(snapshot: snapshot)
        self.ensureMenu()
        self.rebuildMenu()
        self.presentMenu()
    }

    func selectPlatform(id: String) {
        guard let snapshot, snapshot.platforms.contains(where: { $0.id == id }) else {
            return
        }

        self.selectedPlatformId = id
        UserDefaults.standard.set(id, forKey: "cockpit-tools.native-menu.selected-platform")
        if self.viewedAccountIds[id] == nil,
           let platform = snapshot.platforms.first(where: { $0.id == id }),
           let initialAccountId = platform.currentOrFirstAccountId
        {
            self.viewedAccountIds[id] = initialAccountId
        }
        self.bumpRenderRevision()
        self.rebuildMenu()
        self.refreshVisibleMenuDisplay()
    }

    func moveViewedAccount(delta: Int) {
        guard let platform = self.selectedPlatform, !platform.cards.isEmpty else {
            return
        }

        let currentId = self.viewedAccountIds[platform.id] ?? platform.currentOrFirstAccountId
        let currentIndex = platform.cards.firstIndex(where: { $0.id == currentId }) ?? 0
        let total = platform.cards.count
        let nextIndex = (currentIndex + delta + total) % total
        self.viewedAccountIds[platform.id] = platform.cards[nextIndex].id
        self.bumpRenderRevision()
        self.rebuildMenu()
        self.refreshVisibleMenuDisplay()
    }

    func jumpToRecommendedAccount() {
        guard let platform = self.selectedPlatform,
              let recommendedId = platform.recommended_account_id,
              platform.cards.contains(where: { $0.id == recommendedId })
        else {
            return
        }
        self.viewedAccountIds[platform.id] = recommendedId
        self.bumpRenderRevision()
        self.rebuildMenu()
        self.refreshVisibleMenuDisplay()
    }

    func jumpBackToCurrentAccount() {
        guard let platform = self.selectedPlatform,
              let currentId = platform.current_account_id,
              platform.cards.contains(where: { $0.id == currentId })
        else {
            return
        }
        self.viewedAccountIds[platform.id] = currentId
        self.bumpRenderRevision()
        self.rebuildMenu()
        self.refreshVisibleMenuDisplay()
    }

    func dispatch(action: NativeRustAction) {
        switch action {
        case .refresh:
            guard let platform = self.selectedPlatform else { return }
            let accountId = self.viewedCard?.id
            guard !self.isRefreshing(platformId: platform.id, accountId: accountId) else { return }
            self.beginRefresh(platformId: platform.id, accountId: accountId)
            dispatchRustMenuAction(
                action: "refresh",
                platformId: platform.id,
                accountId: accountId
            )
        case .switchAccount:
            guard let platform = self.selectedPlatform, let viewedCard = self.viewedCard else { return }
            self.closeMenu()
            dispatchRustMenuAction(action: "switch", platformId: platform.id, accountId: viewedCard.id)
        case .openDetails:
            guard let platform = self.selectedPlatform else { return }
            self.closeMenu()
            dispatchRustMenuAction(action: "open_details", platformId: platform.id)
        case .viewAllAccounts:
            guard let platform = self.selectedPlatform else { return }
            self.closeMenu()
            dispatchRustMenuAction(action: "view_all_accounts", platformId: platform.id)
        case .openCockpitTools:
            self.closeMenu()
            dispatchRustMenuAction(action: "open_cockpit_tools")
        case .settings:
            self.closeMenu()
            dispatchRustMenuAction(action: "settings")
        case .quit:
            self.closeMenu()
            dispatchRustMenuAction(action: "quit")
        }
    }

    func menuDidClose(_ menu: NSMenu) {
        if self.statusItem?.menu === menu {
            self.statusItem?.menu = nil
        }
        self.clearStatusItemHighlight()
        // 弹出系统菜单后状态栏按钮 frame 可能变化，重新对齐 tray-icon 点击层。
        if let button = self.statusItem?.button {
            syncTrayClickTargetFrameSoon(for: button)
        }
    }

    var selectedPlatform: NativeMenuPlatform? {
        guard let snapshot else {
            return nil
        }
        return snapshot.platforms.first(where: { $0.id == self.selectedPlatformId }) ?? snapshot.platforms.first
    }

    var viewedCard: NativeMenuAccountCard? {
        guard let platform = self.selectedPlatform else {
            return nil
        }
        let viewedId = self.viewedAccountIds[platform.id] ?? platform.currentOrFirstAccountId
        return platform.cards.first(where: { $0.id == viewedId }) ?? platform.cards.first
    }

    func isViewingCurrentAccount(for platform: NativeMenuPlatform) -> Bool {
        let viewedId = self.viewedAccountIds[platform.id] ?? platform.currentOrFirstAccountId
        return viewedId == platform.current_account_id
    }

    func shouldShowRecommendedAction(for platform: NativeMenuPlatform) -> Bool {
        self.isViewingCurrentAccount(for: platform)
            && platform.recommended_account_id != nil
            && platform.recommended_account_id != platform.current_account_id
    }

    func shouldShowBackAction(for platform: NativeMenuPlatform) -> Bool {
        !self.isViewingCurrentAccount(for: platform) && platform.current_account_id != nil
    }

    func shouldShowSwitchAction(for platform: NativeMenuPlatform) -> Bool {
        !self.isViewingCurrentAccount(for: platform) && self.viewedCard != nil
    }

    func isRefreshing(platformId: String, accountId: String?) -> Bool {
        self.refreshingPlatformId == platformId && self.refreshingAccountId == accountId
    }

    @objc private func handleOpenDetails(_: Any?) {
        self.dispatch(action: .openDetails)
    }

    @objc private func handleOpenCockpitTools(_: Any?) {
        self.dispatch(action: .openCockpitTools)
    }

    @objc private func handleViewAllAccounts(_: Any?) {
        self.dispatch(action: .viewAllAccounts)
    }

    @objc private func handleSettings(_: Any?) {
        self.dispatch(action: .settings)
    }

    @objc private func handleQuit(_: Any?) {
        self.dispatch(action: .quit)
    }

    private func decodeSnapshot(from json: String) -> NativeMenuSnapshot? {
        guard let data = json.data(using: .utf8) else { return nil }
        do {
            return try JSONDecoder().decode(NativeMenuSnapshot.self, from: data)
        } catch {
            return nil
        }
    }

    func update(snapshotJSON: String) {
        guard let snapshot = self.decodeSnapshot(from: snapshotJSON) else { return }
        // 菜单已打开时的增量刷新：保留用户正在浏览的平台/账号。
        self.apply(snapshot: snapshot)
        self.bumpRenderRevision()
        self.finishRefreshIfNeeded()
        self.rebuildMenu()
        self.refreshVisibleMenuDisplay()
    }

    private func apply(snapshot: NativeMenuSnapshot) {
        self.snapshot = snapshot

        let validPlatformIds = Set(snapshot.platforms.map(\.id))
        if !validPlatformIds.contains(self.selectedPlatformId)
            || self.selectedPlatformId.isEmpty
        {
            self.selectedPlatformId = snapshot.selected_platform_id
            UserDefaults.standard.set(
                self.selectedPlatformId,
                forKey: "cockpit-tools.native-menu.selected-platform"
            )
        }

        var nextViewedAccountIds = self.viewedAccountIds
        for platform in snapshot.platforms {
            let currentViewedId = nextViewedAccountIds[platform.id]
            if let currentViewedId,
               platform.cards.contains(where: { $0.id == currentViewedId })
            {
                continue
            }

            if let fallback = platform.currentOrFirstAccountId {
                nextViewedAccountIds[platform.id] = fallback
            } else {
                nextViewedAccountIds.removeValue(forKey: platform.id)
            }
        }
        self.viewedAccountIds = nextViewedAccountIds
    }

    private func ensureMenu() {
        guard self.menu == nil else { return }
        let menu = NSMenu()
        menu.autoenablesItems = false
        menu.delegate = self
        self.menu = menu
    }

    private func rebuildMenu() {
        guard let menu else {
            return
        }
        menu.removeAllItems()

        guard let snapshot else {
            return
        }

        if !snapshot.platforms.isEmpty {
            menu.addItem(self.makeHostingItem(
                NativeMenuSwitcherSectionView(controller: self, snapshot: snapshot)
            ))
        }

        if let platform = self.selectedPlatform {
            menu.addItem(.separator())
            menu.addItem(self.makeHostingItem(
                NativeMenuAccountSectionView(
                    controller: self,
                    platform: platform,
                    strings: snapshot.strings
                )
            ))
            menu.addItem(.separator())

            menu.addItem(self.makeActionMenuItem(
                title: snapshot.strings.open_details,
                systemName: "arrow.up.forward.app",
                action: #selector(self.handleOpenDetails(_:))
            ))
            menu.addItem(self.makeActionMenuItem(
                title: snapshot.strings.view_all_accounts,
                systemName: "person.2",
                action: #selector(self.handleViewAllAccounts(_:))
            ))
        }

        if !menu.items.isEmpty {
            menu.addItem(.separator())
        }
        menu.addItem(self.makeActionMenuItem(
            title: snapshot.strings.open_cockpit_tools,
            systemName: "macwindow",
            action: #selector(self.handleOpenCockpitTools(_:))
        ))
        menu.addItem(self.makeActionMenuItem(
            title: snapshot.strings.settings,
            systemName: "gearshape",
            action: #selector(self.handleSettings(_:))
        ))
        menu.addItem(self.makeActionMenuItem(
            title: snapshot.strings.quit,
            systemName: "power",
            action: #selector(self.handleQuit(_:))
        ))
    }

    private func refreshVisibleMenuDisplay() {
        guard let menu else {
            return
        }
        menu.update()
        for item in menu.items {
            menu.itemChanged(item)
            item.view?.needsLayout = true
            item.view?.layoutSubtreeIfNeeded()
            item.view?.needsDisplay = true
            item.view?.displayIfNeeded()
        }
    }

    private func presentMenu() {
        guard let menu, let statusItem else {
            return
        }

        self.clearStatusItemHighlight()
        statusItem.menu = menu
        statusItem.popUpMenu(menu)
    }

    private func closeMenu() {
        self.menu?.cancelTrackingWithoutAnimation()
        if let menu, self.statusItem?.menu === menu {
            self.statusItem?.menu = nil
        }
        self.clearStatusItemHighlight()
    }

    private func clearStatusItemHighlight() {
        guard let button = self.statusItem?.button else {
            return
        }
        button.highlight(false)
        button.needsDisplay = true
        button.displayIfNeeded()
    }

    private func makeHostingItem<Content: View>(_ view: Content) -> NSMenuItem {
        let controller = NSHostingController(rootView: view)
        let size = controller.sizeThatFits(in: CGSize(
            width: NativeMenuLayout.width,
            height: .greatestFiniteMagnitude
        ))
        let hosting = MenuHostingView(rootView: view)
        hosting.frame = NSRect(
            origin: .zero,
            size: NSSize(width: NativeMenuLayout.width, height: ceil(size.height))
        )

        let item = NSMenuItem()
        item.view = hosting
        item.isEnabled = false
        return item
    }

    private func makeActionMenuItem(title: String, systemName: String, action: Selector) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
        item.target = self
        if let image = NSImage(systemSymbolName: systemName, accessibilityDescription: nil) {
            image.isTemplate = true
            image.size = NSSize(width: 16, height: 16)
            item.image = image
        }
        return item
    }

    private func beginRefresh(platformId: String, accountId: String?) {
        self.clearRefreshTask?.cancel()
        self.refreshStartedAt = Date()
        self.refreshingPlatformId = platformId
        self.refreshingAccountId = accountId
        self.bumpRenderRevision()
        self.refreshVisibleMenuDisplay()
    }

    private func finishRefreshIfNeeded() {
        guard let refreshingPlatformId else {
            return
        }
        let refreshingAccountId = self.refreshingAccountId
        let elapsed = Date().timeIntervalSince(self.refreshStartedAt ?? .distantPast)
        let remainingDelay = max(0, 0.45 - elapsed)

        self.clearRefreshTask?.cancel()
        self.clearRefreshTask = Task { [weak self] in
            if remainingDelay > 0 {
                try? await Task.sleep(nanoseconds: UInt64(remainingDelay * 1_000_000_000))
            }
            guard !Task.isCancelled else {
                return
            }
            await MainActor.run {
                guard let self,
                      self.refreshingPlatformId == refreshingPlatformId,
                      self.refreshingAccountId == refreshingAccountId
                else {
                    return
                }
                self.refreshingPlatformId = nil
                self.refreshingAccountId = nil
                self.refreshStartedAt = nil
                self.bumpRenderRevision()
                self.rebuildMenu()
                self.refreshVisibleMenuDisplay()
            }
        }
    }

    private func bumpRenderRevision() {
        self.renderRevision &+= 1
    }
}

private final class MenuHostingView<Content: View>: NSHostingView<Content> {
    override var allowsVibrancy: Bool {
        true
    }
}

enum NativeRustAction {
    case refresh
    case switchAccount
    case openDetails
    case viewAllAccounts
    case openCockpitTools
    case settings
    case quit
}
