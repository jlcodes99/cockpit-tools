# Security Policy

## Supported versions

Security fixes target the latest stable release. Please include the exact Cockpit
Tools version and operating system in a report, and check the latest release when
it is safe to do so. Older releases may also be affected; report that information
rather than assuming that an upgrade fixes the issue. There is no commitment to
backport fixes to every older release.

## Reporting a vulnerability

Do not put vulnerability details, working exploits, or credentials in a public
issue, pull request, or discussion.

Open the upstream repository's **Security** tab. If **Report a vulnerability** is
available, use it to submit a private report. If private reporting is unavailable
and no private contact has been published, open an issue that only asks the
maintainer for a private reporting channel. Do not include the vulnerability or
its reproduction steps in that issue.

In the private report, include the affected version/platform, expected and actual
behavior, impact, and the smallest safe reproduction. Use dummy accounts and
redacted examples. Never send live OAuth tokens, API keys, cookies, MFA secrets,
account exports, or complete authentication files. Logs and screenshots can also
contain these values; review them before sharing.

Coordinate any public disclosure with the maintainer. This policy does not
promise a response deadline or a fix date. Ordinary bugs that do not expose
sensitive data or cross a security boundary can use the public issue tracker.
