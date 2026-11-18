fn upsert_vscdb_item(conn: &Connection, key: &str, value: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?1, ?2)",
        (key, value),
    )
    .map_err(|e| format!("写入 {} 失败: {}", key, e))?;
    Ok(())
}

// Cursor 3.27 owns membership caches by the JWT subject. Both entry points
// replace the complete session in one transaction, including absent fields.
fn write_cursor_auth(conn: &mut Connection, account: &CursorAccount) -> Result<(), String> {
    conn.busy_timeout(std::time::Duration::from_secs(3))
        .map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for key in [
        "cursorAuth/refreshToken",
        "cursorAuth/stripeMembershipType",
        "cursorAuth/stripeMembershipAuthId",
        "cursorAuth/stripeSubscriptionStatus",
        "cursorAuth/stripeCustomerId",
        "cursorAuth/cachedSignUpType",
        "cursorAuth/cachedScopedProfile",
        "cursorAuth/cachedTeam",
    ] {
        tx.execute("DELETE FROM ItemTable WHERE key = ?1", [key])
            .map_err(|e| e.to_string())?;
    }
    for (key, value) in [
        ("cursorAuth/accessToken", account.access_token.as_str()),
        ("cursorAuth/cachedEmail", account.email.as_str()),
        ("cursor.accessToken", account.access_token.as_str()),
        ("cursor.email", account.email.as_str()),
    ] {
        upsert_vscdb_item(&tx, key, value)?;
    }
    for (key, value) in [
        ("cursorAuth/refreshToken", account.refresh_token.as_deref()),
        (
            "cursorAuth/stripeMembershipType",
            account.membership_type.as_deref(),
        ),
        (
            "cursorAuth/stripeSubscriptionStatus",
            account.subscription_status.as_deref(),
        ),
        (
            "cursorAuth/cachedSignUpType",
            account.sign_up_type.as_deref(),
        ),
    ] {
        if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
            upsert_vscdb_item(&tx, key, value)?;
        }
    }
    if account.membership_type.is_some() {
        if let Some(auth_id) = extract_auth_id_from_access_token(&account.access_token) {
            upsert_vscdb_item(&tx, "cursorAuth/stripeMembershipAuthId", &auth_id)?;
        }
    }
    tx.commit().map_err(|e| e.to_string())
}

