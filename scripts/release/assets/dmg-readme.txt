Cockpit Tools 安装与常见问题
================================================================

一、安装
----------------------------------------------------------------
1. 把 Cockpit Tools 拖到窗口右侧的“Applications（应用程序）”文件夹。
2. 在“启动台”或“应用程序”文件夹中打开 Cockpit Tools。

二、如果提示“应用已损坏，无法打开”
----------------------------------------------------------------
这不是安装包真的损坏，也不是应用携带病毒。
macOS 的安全机制（Gatekeeper）要求从网络下载的应用必须经过 Apple
开发者签名与公证；当前开源发布流程尚未接入签名与公证，因此部分系统
版本会直接提示“应用已损坏，无法打开”。

按下面的任一种方式处理，处理完成后即可正常打开。

方式一：从系统设置中允许打开（推荐，不需要输入密码）
1. 尝试打开一次应用，看到提示后点击“完成”，取消该提示。
2. 打开“系统设置” -> “隐私与安全性”。
3. 下滑到“安全性”区域，找到关于 Cockpit Tools 被阻止的提示，
   点击“仍要打开”，再确认打开。

方式二：命令行修复（需要输入开机密码）
1. 确认 Cockpit Tools 已经拖入“应用程序”文件夹。
2. 打开“终端”（在“启动台”搜索“终端”或“Terminal”）。
3. 执行下面的命令，按提示输入开机密码（输入时不会显示字符）：

   sudo xattr -rd com.apple.quarantine "/Applications/Cockpit Tools.app"

注意：如果你修改过应用名称或安装位置，请把命令中的路径换成实际路径。

补充说明
----------------------------------------------------------------
· 升级安装新版本后如果再次出现该提示，按上面的方式再处理一次即可。
· 只从官方 Release 页面下载安装包。


Cockpit Tools - install and troubleshooting
================================================================

1. Install
----------------------------------------------------------------
1. Drag Cockpit Tools into the Applications folder on the right.
2. Open Cockpit Tools from Launchpad or the Applications folder.

2. If macOS says the app "is damaged and can't be opened"
----------------------------------------------------------------
The installer is not corrupted and the app does not contain malware.
Gatekeeper, the macOS security feature, requires apps downloaded from the
internet to be signed and notarized with an Apple Developer ID. The current
open-source release pipeline does not run notarization yet, so some macOS
versions show the "is damaged and can't be opened" message.

Use either method below to fix it. The app opens normally afterwards.

Method 1: Allow it in System Settings (recommended, no password needed)
1. Try to open the app once, then dismiss the warning.
2. Open System Settings -> Privacy & Security.
3. Scroll down to the Security section, find the message about Cockpit
   Tools being blocked, click "Open Anyway", then confirm.

Method 2: Terminal (asks for your login password)
1. Make sure Cockpit Tools is already in the Applications folder.
2. Open Terminal (search for "Terminal" in Launchpad).
3. Run the following command and enter your login password when asked
   (the password is not shown while you type):

   sudo xattr -rd com.apple.quarantine "/Applications/Cockpit Tools.app"

Note: if you renamed the app or installed it somewhere else, replace the
path in the command with the real path.

Notes
----------------------------------------------------------------
· After installing an update, the warning may appear again. Just repeat
  the steps above.
· Download installers only from the official Releases page.
