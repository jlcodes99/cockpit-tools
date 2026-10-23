// Every test Host owns a real, private SQLite identity store. Account/collection
// overrides alone do not isolate the process-wide test data-directory variable.
// Keep this owner alive in blocking workers so cancelling a test route cannot
// remove its database while the worker is still using it.
struct NativeIdentityFixture(tempfile::TempDir);

impl Default for NativeIdentityFixture {
    fn default() -> Self {
        Self(tempfile::tempdir().expect("create private native Host identity directory"))
    }
}

impl NativeIdentityFixture {
    fn path(&self) -> PathBuf {
        self.0.path().join("codex_api_account_identity.sqlite")
    }

    fn resolve(
        &self,
        account: &CodexAccount,
    ) -> Result<crate::modules::codex_api_identity::InstallationProfile, String> {
        crate::modules::codex_api_identity::resolve_at(&self.path(), account)
    }
}
