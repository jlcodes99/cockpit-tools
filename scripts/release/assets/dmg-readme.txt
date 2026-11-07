Cockpit Tools 安装与常见问题
================================================================

一、安装
----------------------------------------------------------------
1. 把 Cockpit Tools 拖到窗口右侧的“Applications（应用程序）”文件夹。
2. 在“启动台”或“应用程序”文件夹中打开 Cockpit Tools。
3. 首次打开可能显示“从互联网下载，是否打开”的正常确认。

二、签名与安全提示
----------------------------------------------------------------
正式发布流程要求 Developer ID 签名、Apple 公证与公证票据校验。
本地开发包及历史未签名版本不自动具备这些验证结果。

如提示“应用已损坏”或无法验证开发者：
1. 从官方 Releases 重新下载已注明签名与公证的版本。
2. 核对该版本的 SHA256SUMS.txt，并重新安装到“应用程序”。
3. 仍有问题时，请反馈版本、芯片架构与完整提示。

不要直接把安全警告视为误报。正常安装无需关闭 Gatekeeper 或
删除 com.apple.quarantine 隔离属性。

官方发布页：https://github.com/jlcodes99/cockpit-tools/releases


Cockpit Tools - install and troubleshooting
================================================================

1. Install
----------------------------------------------------------------
1. Drag Cockpit Tools into the Applications folder on the right.
2. Open Cockpit Tools from Launchpad or the Applications folder.
3. A normal first-launch confirmation about an Internet download may appear.

2. Signatures and security warnings
----------------------------------------------------------------
The official release pipeline requires Developer ID signing, Apple
notarization and stapled-ticket verification. Local development builds and
historical unsigned versions do not automatically carry these results.

If macOS reports damage or an unverified developer:
1. Download a version marked as signed and notarized from official Releases.
2. Verify its SHA256SUMS.txt and reinstall into Applications.
3. If the issue persists, report the version, architecture and full message.

Do not assume security warnings are false alarms. Normal installation does
not require disabling Gatekeeper or removing com.apple.quarantine.

Official releases: https://github.com/jlcodes99/cockpit-tools/releases
