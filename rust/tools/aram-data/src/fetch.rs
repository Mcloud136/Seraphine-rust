//! Network sources for the ARAM snapshot: the Wiki Lua module (primary), the
//! jsdelivr bootstrap snapshot, ddragon versions and the Tencent hero list.

use std::collections::HashMap;

use serde_json::Value;

pub const USER_AGENT: &str = "curl/8.13.0";
pub const WIKI_URL: &str = "https://wiki.leagueoflegends.com/en-us/Module:ChampionData/data";
pub const WIKI_PRE_START: &str = "-- &lt;pre>";
pub const WIKI_PRE_END: &str = "-- &lt;/pre>";
pub const DDRAGON_VERSIONS_URL: &str = "https://ddragon.leagueoflegends.com/api/versions.json";
pub const HERO_LIST_URL: &str = "https://game.gtimg.cn/images/lol/act/img/js/heroList/hero_list.js";
pub const BOOTSTRAP_URL: &str =
    "https://cdn.jsdelivr.net/npm/balance-buff-viewer-plugin@latest/dist/balance.json";
pub const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;

pub struct Fetcher {
    client: reqwest::Client,
}

impl Fetcher {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|error| format!("http client: {error}"))?;
        Ok(Self { client })
    }

    pub async fn text(&self, url: &str) -> Result<String, String> {
        let response = self
            .client
            .get(url)
            .header(reqwest::header::ACCEPT, "*/*")
            .send()
            .await
            .map_err(|error| format!("{url}: {error}"))?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!("{url}: HTTP {status}"));
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("{url}: {error}"))?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(format!("{url}: exceeds byte budget"));
        }
        String::from_utf8(bytes.to_vec()).map_err(|error| format!("{url}: not UTF-8: {error}"))
    }

    pub async fn wiki_script(&self) -> Result<String, String> {
        let html = self.text(WIKI_URL).await?;
        let start = html.find(WIKI_PRE_START).ok_or_else(|| {
            "wiki page does not contain the Lua data block (browser challenge or layout change)"
                .to_string()
        })? + WIKI_PRE_START.len();
        let end = html
            .find(WIKI_PRE_END)
            .filter(|end| *end > start)
            .ok_or_else(|| "wiki Lua data block is not terminated".to_string())?;
        Ok(html_unescape(&html[start..end]))
    }

    pub async fn latest_version(&self) -> Result<String, String> {
        let text = self.text(DDRAGON_VERSIONS_URL).await?;
        let versions: Value =
            serde_json::from_str(&text).map_err(|error| format!("ddragon versions: {error}"))?;
        versions
            .as_array()
            .and_then(|items| items.first())
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "ddragon returned no versions".to_string())
    }

    pub async fn champion_names(&self, url: &str) -> HashMap<String, String> {
        match self.text(url).await {
            Ok(text) => match serde_json::from_str::<Value>(&text) {
                Ok(root) => root
                    .get("hero")
                    .and_then(Value::as_array)
                    .map(|heroes| {
                        heroes
                            .iter()
                            .filter_map(|hero| {
                                let id = hero.get("heroId").and_then(value_as_id)?;
                                let name = hero.get("name").and_then(Value::as_str)?.trim();
                                (!id.is_empty() && !name.is_empty()).then(|| (id, name.to_string()))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                Err(error) => {
                    eprintln!("warning: champion names unavailable: {error}");
                    HashMap::new()
                }
            },
            Err(error) => {
                eprintln!("warning: champion names unavailable: {error}");
                HashMap::new()
            }
        }
    }

    pub async fn bootstrap(&self) -> Result<Value, String> {
        let text = self.text(BOOTSTRAP_URL).await?;
        let value: Value =
            serde_json::from_str(&text).map_err(|error| format!("bootstrap: {error}"))?;
        value
            .is_object()
            .then_some(value)
            .ok_or_else(|| "bootstrap source did not return an object".to_string())
    }
}

fn value_as_id(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    value.as_i64().map(|number| number.to_string())
}

pub fn html_unescape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        let tail = &rest[index..];
        let Some(end) = tail.find(';').filter(|end| *end <= 12) else {
            output.push('&');
            rest = &tail[1..];
            continue;
        };
        let entity = &tail[1..end];
        let decoded = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            other => decode_numeric(other),
        };
        match decoded {
            Some(ch) => {
                output.push(ch);
                rest = &tail[end + 1..];
            }
            None => {
                output.push('&');
                rest = &tail[1..];
            }
        }
    }
    output.push_str(rest);
    output
}

fn decode_numeric(entity: &str) -> Option<char> {
    let code = entity
        .strip_prefix("#x")
        .or_else(|| entity.strip_prefix("#X"));
    let value = match code {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => entity.strip_prefix('#')?.parse::<u32>().ok()?,
    };
    char::from_u32(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_html_entities() {
        assert_eq!(html_unescape("-- &lt;pre&gt;"), "-- <pre>");
        assert_eq!(html_unescape("a &amp; b &#39;c&#x27;"), "a & b 'c'");
    }
}
