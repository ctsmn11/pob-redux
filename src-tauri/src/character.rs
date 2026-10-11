const HOST: &str = "https://www.pathofexile.com/";
/// pathofexile.com's profile pages only answer user agents starting with "Path of Building".
const UA: &str = "Path of Building (PoB Redux)";

pub(crate) fn query_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Only `#` is escaped, so a console name's `+` still reads as a space.
fn account_param(account: &str) -> String {
    account.replace('#', "%23")
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

async fn get(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let resp = client.get(url).send().await.map_err(|e| format!("pathofexile.com: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| format!("pathofexile.com: {e}"))?;
    match status {
        200..=299 => Ok(body),
        401 | 403 => Err("That account's profile or characters tab is private, or the account does not exist.".into()),
        404 => Err("No account with that name. Include the #1234 part of the name.".into()),
        429 => Err("pathofexile.com is limiting requests. Wait a minute, then try again.".into()),
        _ => Err(format!("pathofexile.com returned HTTP {status}")),
    }
}

async fn profile_name(client: &reqwest::Client, account: &str) -> Option<String> {
    let page = match get(client, &format!("{HOST}account/view-profile/{}", account_param(account))).await {
        Ok(page) => page,
        Err(e) => {
            log::warn!("profile page for {account}: {e}");
            return None;
        }
    };
    let marker = "/view-profile/";
    page.match_indices(marker).find_map(|(i, _)| {
        let rest = &page[i + marker.len()..];
        let end = rest.find('/')?;
        rest[end..].starts_with("/characters").then(|| rest[..end].to_string())
    })
}

pub(crate) async fn account_casing(account: &str) -> Option<String> {
    profile_name(&client().ok()?, account).await
}
