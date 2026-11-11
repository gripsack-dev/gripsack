//! Informational Git provenance never supplies approval authority or secrets.

pub(super) fn redact_remote(value: &str) -> Option<String> {
    if let Ok(mut parsed) = url::Url::parse(value) {
        match parsed.scheme() {
            "https" | "http" | "ssh" | "git" => {
                parsed.set_username("").ok()?;
                parsed.set_password(None).ok()?;
                parsed.set_query(None);
                parsed.set_fragment(None);
                return Some(parsed.to_string());
            }
            _ => return None,
        }
    }
    // SCP-style Git remotes have no URL parser scheme. Normalize only the
    // recognizable host:path form; unknown local/credential forms are omitted.
    let remote = value.rsplit_once('@').map_or(value, |(_, tail)| tail);
    let (host, path) = remote.split_once(':')?;
    if host.is_empty()
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return None;
    }
    let path = path.split(['?', '#']).next()?;
    let mut parsed = url::Url::parse(&format!("ssh://{host}/")).ok()?;
    parsed.set_path(path);
    Some(parsed.to_string())
}

pub(super) fn now_rfc3339() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    rfc3339(seconds)
}

fn rfc3339(epoch_secs: u64) -> String {
    let (year, month, day) = civil_from_days((epoch_secs / 86_400) as i64);
    let seconds = epoch_secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60
    )
}

// Existing trust timestamp conversion: Hinnant's proleptic Gregorian mapping.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2) as i64;
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_never_retains_credentials_query_or_fragment() {
        assert_eq!(
            redact_remote("https://owner:secret@example.test/team/repo?token=secret#secret")
                .as_deref(),
            Some("https://example.test/team/repo")
        );
        assert_eq!(
            redact_remote("git@example.test:team/repo.git").as_deref(),
            Some("ssh://example.test/team/repo.git")
        );
        assert_eq!(redact_remote("/local/path/with-unknown-secrets"), None);
    }

    #[test]
    fn timestamps_follow_calendar_boundaries() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(86_399), "1970-01-01T23:59:59Z");
        assert_eq!(rfc3339(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_709_164_800), "2024-02-29T00:00:00Z");
    }
}
