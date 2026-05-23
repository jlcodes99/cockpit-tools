cask "cockpit-tools" do
  version "0.24.7"
  sha256 "ed0b383b6c3d313c242ea80ba3a1e8f2d4cef3366769363a9eb19d7351c30de5"

  url "https://github.com/jlcodes99/cockpit-tools/releases/download/v#{version}/Cockpit.Tools_#{version}_universal.dmg",
      verified: "github.com/jlcodes99/cockpit-tools/"
  name "Cockpit Tools"
  desc "Account manager for AI IDEs (Antigravity and Codex)"
  homepage "https://github.com/jlcodes99/cockpit-tools"

  auto_updates true

  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-cr", "#{appdir}/Cockpit Tools.app"],
                   sudo: true
  end

  app "Cockpit Tools.app"

  zap trash: [
    "~/Library/Application Support/com.jlcodes.cockpit-tools",
    "~/Library/Caches/com.jlcodes.cockpit-tools",
    "~/Library/Preferences/com.jlcodes.cockpit-tools.plist",
    "~/Library/Saved Application State/com.jlcodes.cockpit-tools.savedState",
  ]

  caveats <<~EOS
    The app is automatically quarantined by macOS. A postflight hook has been added to remove this quarantine.
    If you still encounter the "App is damaged" error, please run:
      sudo xattr -rd com.apple.quarantine "/Applications/Cockpit Tools.app"
  EOS
end
