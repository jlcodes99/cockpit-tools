cask "switchme" do
  version "0.22.21"
  sha256 "8ef90baa72c6b7df91452f1b7524eec01238e7e204e0f635fb3edba9ef6f3b04"

  url "https://github.com/adxptived/Switchme/releases/download/v#{version}/Switchme_#{version}_universal.dmg",
      verified: "github.com/adxptived/Switchme/"
  name "Switchme"
  desc "Account manager for AI IDEs (Antigravity and Codex)"
  homepage "https://github.com/adxptived/Switchme"

  auto_updates true

  postflight do
    system_command "/usr/bin/xattr",
                   args: ["-cr", "#{appdir}/Switchme.app"],
                   sudo: true
  end

  app "Switchme.app"

  zap trash: [
    "~/Library/Application Support/com.adxptived.switchme",
    "~/Library/Caches/com.adxptived.switchme",
    "~/Library/Preferences/com.adxptived.switchme.plist",
    "~/Library/Saved Application State/com.adxptived.switchme.savedState",
  ]

  caveats <<~EOS
    The app is automatically quarantined by macOS. A postflight hook has been added to remove this quarantine.
    If you still encounter the "App is damaged" error, please run:
      sudo xattr -rd com.apple.quarantine "/Applications/Switchme.app"
  EOS
end
