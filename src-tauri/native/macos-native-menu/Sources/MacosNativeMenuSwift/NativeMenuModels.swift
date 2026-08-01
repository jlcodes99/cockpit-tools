import Foundation

enum NativeMenuProgressTone: String, Codable, Hashable {
    case high
    case medium
    case low
    case critical
}

struct NativeMenuStrings: Codable {
    let view_recommended: String
    let back_to_current: String
    let switch_to_viewed: String
    let refresh: String
    let open_cockpit_tools: String
    let open_details: String
    let view_all_accounts: String
    let settings: String
    let quit: String
    let empty_title: String
    let empty_desc: String
}

struct NativeMenuQuotaRow: Codable, Hashable {
    let label: String
    let value: String
    let progress: Int?
    let progress_tone: NativeMenuProgressTone?
    let subtext: String?
}

struct NativeMenuAccountCard: Codable, Hashable, Identifiable {
    let id: String
    let title: String
    let plan: String?
    let updated_text: String
    let quota_rows: [NativeMenuQuotaRow]
}

struct NativeMenuPlatform: Codable, Hashable, Identifiable {
    let id: String
    let title: String
    let short_title: String
    let nav_target: String
    let accent_hex: String
    let current_account_id: String?
    let recommended_account_id: String?
    let cards: [NativeMenuAccountCard]

    var currentOrFirstAccountId: String? {
        if let current_account_id, self.cards.contains(where: { $0.id == current_account_id }) {
            return current_account_id
        }
        return self.cards.first?.id
    }
}

struct NativeMenuSnapshot: Codable {
    let strings: NativeMenuStrings
    let platforms: [NativeMenuPlatform]
    let selected_platform_id: String
}

struct NativeMenuBarStatus: Codable {
    let platform_id: String
    let short_title: String
    let value_text: String
    let remaining_percent: Int?
}
