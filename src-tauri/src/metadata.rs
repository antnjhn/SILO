use serde::Serialize;
use std::collections::HashSet;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize)]
pub struct MetadataCandidate {
  pub name: String,
  pub source: String,
  #[serde(rename = "appId")]
  pub app_id: Option<String>,
  pub logo: Option<String>,
  pub wallpaper: Option<String>,
  // Every available image for the online-art picker, in preference order.
  #[serde(default)]
  pub logos: Vec<String>,
  #[serde(default)]
  pub wallpapers: Vec<String>,
  pub confidence: u8,
  pub year: Option<String>,
  pub rating: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetadataResult {
  pub query: String,
  pub candidates: Vec<MetadataCandidate>,
  #[serde(rename = "hasSgdb")]
  pub has_sgdb: bool,
  #[serde(rename = "sgdbCount")]
  pub sgdb_count: usize,
}

fn normalize_tokens(text: &str) -> HashSet<String> {
  text
    .to_lowercase()
    .split(|c: char| !c.is_alphanumeric())
    .filter(|t| !t.is_empty() && (t.len() > 1 || t.chars().all(|c| c.is_ascii_digit())))
    .map(|t| t.to_string())
    .collect()
}

// Token-overlap similarity in 0-100. Returns 100 for identical token sets.
pub fn name_similarity(query: &str, candidate: &str) -> u8 {
  let query_tokens = normalize_tokens(query);
  let candidate_tokens = normalize_tokens(candidate);
  if query_tokens.is_empty() || candidate_tokens.is_empty() {
    return 0;
  }
  let intersection = query_tokens.intersection(&candidate_tokens).count();
  let overlap = (2 * intersection) as f32 / (query_tokens.len() + candidate_tokens.len()) as f32;
  let mut confidence = (overlap * 100.0).round() as u8;
  if query_tokens == candidate_tokens {
    confidence = 100;
  }
  confidence.min(100)
}

// Returns every URL in priority order that returns an HTTP success, in parallel.
// The online-art picker shows all of these so the user can choose a specific image.
async fn available_images(client: &reqwest::Client, urls: &[String]) -> Vec<String> {
  let handles: Vec<_> = urls
    .iter()
    .map(|url| {
      let client = client.clone();
      let url = url.clone();
      tauri::async_runtime::spawn(async move {
        if let Ok(response) = client.get(&url).send().await {
          if response.status().is_success() {
            return Some(url);
          }
        }
        None
      })
    })
    .collect();
  let mut available = Vec::new();
  for handle in handles {
    if let Ok(Some(url)) = handle.await {
      available.push(url);
    }
  }
  available
}

async fn fetch_steam_candidates(client: &reqwest::Client, query: &str) -> Vec<MetadataCandidate> {
  let url = format!(
    "https://store.steampowered.com/api/storesearch/?term={}&l=english&cc=US",
    urlencoding::encode(query)
  );

  let mut candidates = Vec::new();
  let Ok(response) = client.get(&url).send().await else {
    return candidates;
  };
  let Ok(json) = response.json::<serde_json::Value>().await else {
    return candidates;
  };
  let Some(items) = json.get("items").and_then(|i| i.as_array()) else {
    return candidates;
  };

  let mut handles = Vec::new();
  for item in items
    .iter()
    .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("app") && item.get("id").is_some())
    .take(5)
  {
    let Some(id) = item.get("id").and_then(|i| i.as_i64()) else {
      continue;
    };
    let id_str = id.to_string();
    let name = item.get("name").and_then(|n| n.as_str()).unwrap_or(query).to_string();
    let client = client.clone();
    let query = query.to_string();

    handles.push(tauri::async_runtime::spawn(async move {
      let confidence = name_similarity(&query, &name);
      let logos = available_images(&client, &[
        format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/logo.png", id_str),
        format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{}/logo.png", id_str),
      ])
      .await;
      let wallpapers = available_images(&client, &[
        format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/library_hero.jpg", id_str),
        format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/page_bg_generated_v6.jpg", id_str),
        format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/header.jpg", id_str),
      ])
      .await;

      MetadataCandidate {
        name,
        source: "Steam".to_string(),
        app_id: Some(id_str),
        logo: logos.first().cloned(),
        wallpaper: wallpapers.first().cloned(),
        logos,
        wallpapers,
        confidence,
        year: None,
        rating: None,
      }
    }));
  }

  for handle in handles {
    if let Ok(candidate) = handle.await {
      candidates.push(candidate);
    }
  }

  candidates.sort_by(|a, b| b.confidence.cmp(&a.confidence));
  candidates
}

// Fetches SteamGridDB artwork (logos = logos; wallpapers = heroes + grids) for a
// Steam app id, using the user's free SGDB API key via the Bearer auth header.
async fn fetch_sgdb_art(client: &reqwest::Client, key: &str, steam_app_id: &str) -> (Vec<String>, Vec<String>) {
  let base = "https://www.steamgriddb.com/api/v2";
  let logos = sgdb_images(client, key, &format!("{}/logos/game/{}", base, steam_app_id), 3).await;
  let heroes = sgdb_images(client, key, &format!("{}/heroes/game/{}", base, steam_app_id), 2).await;
  let grids = sgdb_images(client, key, &format!("{}/grids/game/{}", base, steam_app_id), 2).await;
  log::info!(
    "SteamGridDB app {} -> {} logos, {} heroes, {} grids",
    steam_app_id,
    logos.len(),
    heroes.len(),
    grids.len()
  );
  let mut wallpapers = heroes;
  wallpapers.extend(grids);
  (logos, wallpapers)
}

// Reads up to `max` image URLs from a SteamGridDB artwork endpoint. Prefers the
// full-size `full` URL and falls back to `url`. Non-2xx responses are logged so a
// bad/rate-limited key is visible in the logs instead of failing silently.
async fn sgdb_images(client: &reqwest::Client, key: &str, url: &str, max: usize) -> Vec<String> {
  let Ok(response) = client
    .get(url)
    .header("Authorization", format!("Bearer {}", key))
    .send()
    .await
  else {
    log::warn!("SteamGridDB request failed to send: {}", url);
    return Vec::new();
  };
  if !response.status().is_success() {
    log::warn!("SteamGridDB {} -> HTTP {}", url, response.status());
    return Vec::new();
  }
  let Ok(json) = response.json::<serde_json::Value>().await else {
    return Vec::new();
  };
  let Some(items) = json.get("data").and_then(|d| d.as_array()) else {
    return Vec::new();
  };
  items
    .iter()
    .take(max)
    .filter_map(|item| {
      item
        .get("full")
        .or_else(|| item.get("url"))
        .and_then(|f| f.as_str())
        .map(|s| s.to_string())
    })
    .collect()
}

// Metadata fallback chain: Steam art first (keyless), then SteamGridDB custom art
// (when the user's free SGDB API key is set). Manual fallback is handled by the frontend.
#[tauri::command]
pub async fn fetch_metadata(app: AppHandle, name: String) -> Result<MetadataResult, String> {
  let client = reqwest::Client::builder()
    .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
    .build()
    .map_err(|e| e.to_string())?;

  let mut candidates = fetch_steam_candidates(&client, &name).await;

  let settings = crate::settings::get_settings(app);
  let mut has_sgdb = false;
  let mut sgdb_count = 0;
  if let Some(key) = settings.sgdb_api_key.as_deref() {
    if !key.trim().is_empty() {
      has_sgdb = true;
      // SteamGridDB is its own source. Each autocomplete hit becomes its own candidate
      // carrying that specific game's logos/heroes/grids, so e.g. each Assassin's Creed
      // title shows its own art instead of one game's images leaking onto every entry.
      for game in sgdb_games(&client, key, &name).await {
        let game_name = game.name.clone();
        let confidence = name_similarity(&name, &game_name);
        let (logos, wallpapers) = fetch_sgdb_art(&client, key, &game.id).await;
        sgdb_count += logos.len() + wallpapers.len();
        candidates.push(MetadataCandidate {
          name: game_name,
          source: "SteamGridDB".to_string(),
          app_id: Some(game.id),
          logo: logos.first().cloned(),
          logos,
          wallpaper: wallpapers.first().cloned(),
          wallpapers,
          confidence,
          year: None,
          rating: None,
        });
      }
      candidates.sort_by(|a, b| b.confidence.cmp(&a.confidence));
    }
  }

  Ok(MetadataResult {
    query: name,
    candidates,
    has_sgdb,
    sgdb_count,
  })
}

struct SgdbGame {
  id: String,
  name: String,
}

// Returns SteamGridDB game hits (id + name) for a query via autocomplete, logging the
// matched games so a failed lookup is diagnosable.
async fn sgdb_games(client: &reqwest::Client, key: &str, query: &str) -> Vec<SgdbGame> {
  let url = format!(
    "https://www.steamgriddb.com/api/v2/search/autocomplete/{}",
    urlencoding::encode(query)
  );
  let Ok(response) = client
    .get(&url)
    .header("Authorization", format!("Bearer {}", key))
    .send()
    .await
  else {
    log::warn!("SteamGridDB autocomplete failed to send: {}", url);
    return Vec::new();
  };
  if !response.status().is_success() {
    log::warn!("SteamGridDB autocomplete -> HTTP {}", response.status());
    return Vec::new();
  }
  let Ok(json) = response.json::<serde_json::Value>().await else {
    return Vec::new();
  };
  let games: Vec<SgdbGame> = json
    .get("data")
    .and_then(|d| d.as_array())
    .map(|arr| {
      arr
        .iter()
        .filter_map(|item| {
          let id = item.get("id").and_then(|i| i.as_i64())?.to_string();
          let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
          Some(SgdbGame { id, name })
        })
        .collect()
    })
    .unwrap_or_default();
  log::info!(
    "SteamGridDB autocomplete '{}' -> {} games: {}",
    query,
    games.len(),
    games.iter().map(|g| format!("{}={}", g.id, g.name)).collect::<Vec<_>>().join(", ")
  );
  games.into_iter().take(4).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn name_similarity_identical_is_100() {
    assert_eq!(name_similarity("Red Dead Redemption 2", "Red Dead Redemption 2"), 100);
    assert_eq!(name_similarity("Portal", "Portal"), 100);
  }

  #[test]
  fn name_similarity_is_case_insensitive() {
    assert_eq!(name_similarity("WITCHER 3", "witcher 3"), 100);
    assert_eq!(name_similarity("Witcher 3", "WITCHER 3"), 100);
  }

  #[test]
  fn name_similarity_empty_is_0() {
    assert_eq!(name_similarity("", "Witcher 3"), 0);
    assert_eq!(name_similarity("Witcher 3", ""), 0);
    assert_eq!(name_similarity("", ""), 0);
  }

  #[test]
  fn name_similarity_sequel_vs_base_is_high_but_less_than_100() {
    // "Red Dead Redemption 2" vs "Red Dead Redemption" share {red, dead, redemption};
    // the "2" token is kept, so they must NOT be treated as identical.
    let score = name_similarity("Red Dead Redemption 2", "Red Dead Redemption");
    assert!(score >= 60 && score < 100, "got {}", score);
  }

  #[test]
  fn name_similarity_single_char_tokens() {
    // Single non-digit letters normalize away -> empty set -> 0.
    assert_eq!(name_similarity("A", "A"), 0);
    // Single-digit tokens are kept, so "7" == "7".
    assert_eq!(name_similarity("7", "7"), 100);
  }
}
