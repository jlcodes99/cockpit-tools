# Rebuilding the community preview

This optional configuration is for a manually installed community preview, not upstream's official release pipeline. It preserves the original product identifier, installation identity, updater public key and updater endpoints. It does not include any account, token, configuration export or user installation backup.

Use the exact source commit identified by the PR and the repository's locked dependency versions, with Node/npm, Rust and the normal Windows MSVC/SDK tools available. From the repository root:

```powershell
npm ci
npm run build
if (-not (Test-Path -LiteralPath dist/index.html -PathType Leaf)) { throw 'Missing frontend entry' }
npx tauri build --bundles nsis --no-sign --config src-tauri/tauri.release.conf.json --config src-tauri/tauri.ci.conf.json --config docs/fixes/large-request-opening/community-preview.tauri.json --features tauri/custom-protocol --ci -- --locked
```

The optional `--offline` Cargo argument only works after dependencies are cached. The overlay sets `beforeBuildCommand` to null because the preceding explicit frontend build is required; the release guard in `src-tauri/build_frontend.rs` remains active and checks the referenced assets during compilation. The frontend must be at `dist/index.html` with its referenced assets, relative to the repository, matching `src-tauri/tauri.conf.json`'s `../dist`. Never point this build at an unrelated staging directory or accept an empty directory.

`1.5.2+preview.20261127.1` is a community label. SemVer build metadata does not make it a newer official version; Windows's numeric installer version may be `1.5.2.0`. The package is intentionally unsigned and does not generate updater artifacts. Build tools may download NSIS when its verified cache is absent. The default installer can download WebView2 when a recipient lacks that runtime, so this is not an all-dependencies offline installer.

After Tauri has finished NSIS bundling, extract the installer into a dedicated directory without executing it, then calculate the **packaged application's** SHA256 and verify that same executable:

```powershell
$exe = (Resolve-Path target/preview-extracted/cockpit-tools.exe).Path
$sha = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash
$evidence = (New-Item -ItemType Directory -Path target/preview-acceptance -Force).FullName
pwsh -NoProfile -File scripts/release/verify-embedded-frontend.ps1 -Executable $exe -ExpectedSha256 $sha -EvidenceRoot $evidence
```

Use your actual extraction directory. Tauri CLI 2.12.6 backs up the original build-path EXE, temporarily changes the embedded bundle marker from UNK to NSS, packages it, and restores the build-path file. Therefore that file's hash can differ from the installed file even after bundling. For this build the only differences were those three marker bytes; the extracted file was separately verified. Never treat an unexplained binary difference as acceptable. See the exact hashes in [validation.md](validation.md).

The verifier starts a separate, explicit frontend-smoke process with its own WebView data and no production handlers. It checks real rendered DOM/resource loading and exits that owned process. It does not establish that every business feature or every installation environment works.

Its application EXE must hash-match the packaged file actually verified, and all configured bundled resources and license notices must match the source manifest. Do not distribute the extraction tools, build cache, smoke profile, local diagnostics or installation backups with the installer.

For deployment, complete current work and normally exit Cockpit and clients using its API before replacing the running application. The preview shares the original profile. Future official installation/updates may overwrite it; return to an official package through the author's normal installation route. Keep private backups locally and do not send them to recipients.
