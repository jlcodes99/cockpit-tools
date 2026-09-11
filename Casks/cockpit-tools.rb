cask "cockpit-tools" do
  version "1.3.49"
  sha256 "3d7adbd4d824d95e2658e1dc044dc6bc0e9d6efa4791ff7f833f10c86289a514"

  url "https://github.com/jlcodes99/cockpit-tools/releases/download/v#{version}/Cockpit.Tools_#{version}_universal.dmg"
  name "Cockpit Tools"
  desc "Account manager for AI IDEs (Antigravity and Codex)"
  homepage "https://github.com/jlcodes99/cockpit-tools"

  auto_updates true

  app "Cockpit Tools.app"

  postflight_steps do
    run "/usr/bin/xattr",
        args: ["-cr", "{{appdir}}/Cockpit Tools.app"],
        sudo: true
  end

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
