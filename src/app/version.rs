pub fn sanitize_git_sha_short(sha: Option<&'static str>) -> Option<&'static str> {
    let sha = sha.and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });

    let Some(sha) = sha else {
        return None;
    };

    let len = sha.len();
    let is_valid_len = (7..=40).contains(&len);
    let is_hex = sha.chars().all(|c| c.is_ascii_hexdigit());

    if !is_valid_len || !is_hex {
        log::warn!(
            "ERR_VERSION_SHA_001: invalid git sha short {:?} (len={}, hex={}); omitting from version output",
            sha,
            len,
            is_hex
        );
        return None;
    }

    Some(sha)
}

pub fn format_display_version(
    app_name: &'static str,
    app_version: &'static str,
    app_git_sha_short: Option<&'static str>,
) -> String {
    let sha = sanitize_git_sha_short(app_git_sha_short);
    if let Some(sha) = sha {
        format!("{app_name} {app_version} ({sha})")
    } else {
        format!("{app_name} {app_version}")
    }
}

pub fn format_clap_version_component(
    app_version: &'static str,
    app_git_sha_short: Option<&'static str>,
) -> &'static str {
    let sha = sanitize_git_sha_short(app_git_sha_short);
    if let Some(sha) = sha {
        Box::leak(format!("{app_version} ({sha})").into_boxed_str())
    } else {
        app_version
    }
}
