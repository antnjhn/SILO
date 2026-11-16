use serde::Serialize;
use std::collections::HashSet;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

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

// ── SteamGridDB browse (search + paginated artwork) ─────────────────────────
// Unlike the autofill chain above, these commands surface every result to the
// user: the picker searches for a game and pages through its whole art
// catalogue. Wallpaper suitability is a presentation concern (the renderer
// labels and sorts), so nothing here filters on dimensions.

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SgdbGame {
    pub id: String,
    pub name: String,
    pub year: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SgdbArt {
    pub id: u32,
    pub url: String,
    pub thumb: String,
    /// Native image width, never the thumbnail's.
    pub width: u32,
    /// Native image height, never the thumbnail's.
    pub height: u32,
    pub mime: String,
    pub style: Option<String>,
    pub author: Option<String>,
    pub animated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SgdbArtPage {
    /// The page index that was fetched (0-based), echoed back by the API.
    pub page: u32,
    /// Total number of arts for this game + kind.
    pub total: u32,
    /// The page size actually requested.
    pub limit: u32,
    pub arts: Vec<SgdbArt>,
}

// Latin letters with their diacritics folded to the plain letter, so "Pokémon"
// and "POKEMON" are the same word. SteamGridDB titles carry accents that a game
// folder or exe name almost never does, and an accent mismatch made a correct
// title look unrelated ("POKEMON EMERALD" scored 40 against "Pokémon Emerald
// Version" because the accented token never matched).
fn fold_diacritics(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'ď' | 'đ' => 'd',
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => 'e',
            'ĝ' | 'ğ' => 'g',
            'ì' | 'í' | 'î' | 'ï' | 'ī' | 'ĭ' | 'į' => 'i',
            'ł' => 'l',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => 'o',
            'ŕ' | 'ř' => 'r',
            'ś' | 'š' | 'ş' => 's',
            'ť' | 'ţ' => 't',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => 'u',
            'ý' | 'ÿ' => 'y',
            'ź' | 'ż' | 'ž' => 'z',
            'ß' => 's',
            other => other,
        })
        .collect()
}

fn normalize_tokens(text: &str) -> HashSet<String> {
    fold_diacritics(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && (t.len() > 1 || t.chars().all(|c| c.is_ascii_digit())))
        // SteamGridDB names the handheld Pokémon entries "<name> Version". The
        // word is catalogue formatting, not part of the title, and keeping it
        // made "Pokémon Emerald Version" tie with a romhack of equal overlap
        // ("Pokémon Emerald Crest"), which then won on alphabetical order.
        .filter(|t| *t != "version")
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

    candidates.sort_by_key(|a| std::cmp::Reverse(a.confidence));
    candidates
}

// Fetches SteamGridDB artwork (logos = logos; wallpapers = heroes + grids) for a
// Steam app id, using the user's free SGDB API key via the Bearer auth header.
async fn fetch_sgdb_art(client: &reqwest::Client, key: &str, steam_app_id: &str) -> (Vec<String>, Vec<String>) {
    let base = "https://www.steamgriddb.com/api/v2";
    let logos = sgdb_images(client, key, &format!("{}/logos/game/{}", base, steam_app_id), 12).await;
    let heroes = sgdb_images(client, key, &format!("{}/heroes/game/{}", base, steam_app_id), 12).await;
    let grids = sgdb_images(client, key, &format!("{}/grids/game/{}", base, steam_app_id), 12).await;
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
            item.get("full")
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
    // Shared pooled HTTP client (same builder sgdb_client() uses): the chain's
    // Steam fetches and SGDB calls reuse one connection pool, paying the TLS
    // handshake once, under the client's connect/total timeouts instead of
    // hanging the command indefinitely.
    let client = sgdb_client()?;

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
            candidates.sort_by_key(|a| std::cmp::Reverse(a.confidence));
        }
    }

    Ok(MetadataResult {
        query: name,
        candidates,
        has_sgdb,
        sgdb_count,
    })
}

// Returns SteamGridDB game hits for a query via autocomplete, logging the
// matched games so a failed lookup is diagnosable.
/* ── Rate limiting ─────────────────────────────────────────────────────────
   SteamGridDB answers 429 with a Retry-After when its per-minute budget is
   spent. The sweep honours that instead of hammering the endpoint: the deadline
   is shared, so every lookup after the first 429 waits for it, and a wait that
   outlives the sweep ends the sweep rather than burning the rest of the budget
   in the background. */
static RATE_LIMITED_UNTIL: OnceLock<std::sync::Mutex<Option<std::time::Instant>>> = OnceLock::new();

fn note_rate_limit(response: &reqwest::Response) {
    if response.status().as_u16() != 429 {
        return;
    }
    let wait = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(15)
        .clamp(1, 120);
    let until = std::time::Instant::now() + Duration::from_secs(wait);
    let slot = RATE_LIMITED_UNTIL.get_or_init(|| std::sync::Mutex::new(None));
    match slot.lock() {
        Ok(mut guard) => *guard = Some(until),
        Err(poisoned) => *poisoned.into_inner() = Some(until),
    }
    log::warn!("SteamGridDB rate limited; waiting {}s before the next lookup", wait);
}

/// How long the shared rate-limit deadline still has to run, if it is set.
fn rate_limit_wait() -> Option<Duration> {
    let slot = RATE_LIMITED_UNTIL.get_or_init(|| std::sync::Mutex::new(None));
    let until = match slot.lock() {
        Ok(guard) => *guard,
        Err(poisoned) => *poisoned.into_inner(),
    }?;
    until.checked_duration_since(std::time::Instant::now())
}

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
        note_rate_limit(&response);
        log::warn!("SteamGridDB autocomplete -> HTTP {}", response.status());
        return Vec::new();
    }
    let Ok(json) = response.json::<serde_json::Value>().await else {
        return Vec::new();
    };
    let games = parse_sgdb_game_list(&json);
    log::info!(
        "SteamGridDB autocomplete '{}' -> {} games: {}",
        query,
        games.len(),
        games
            .iter()
            .map(|g| format!("{}={}", g.id, g.name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    games.into_iter().take(8).collect()
}

// ── SteamGridDB browse: pure helpers ────────────────────────────────────────

const SGDB_BASE: &str = "https://www.steamgriddb.com/api/v2";
const SGDB_NO_KEY: &str = "SteamGridDB API key is not set. Add it in Settings.";
const SGDB_ART_KINDS: [&str; 3] = ["heroes", "logos", "grids"];
const SGDB_DEFAULT_LIMIT: u32 = 50;
const SGDB_MAX_LIMIT: u32 = 50;
const SGDB_MAX_GAMES: usize = 20;

// A string field, trimmed, with empty treated as absent — SGDB sends `""` for
// values it does not have.
fn sgdb_non_blank(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn json_u32(value: &serde_json::Value) -> Option<u32> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .map(|n| n.min(u32::MAX as u64) as u32)
}

fn u32_field(item: &serde_json::Value, key: &str) -> u32 {
    item.get(key).and_then(json_u32).unwrap_or(0)
}

// `release_date` is a date for most games ("2015-05-19"), a bare year for some
// re-releases, and absent for unreleased titles. The year is the four leading
// digits when there are any, otherwise whatever the API said.
fn sgdb_year(item: &serde_json::Value) -> Option<String> {
    let raw = sgdb_non_blank(item.get("release_date"))?;
    if let Some(head) = raw.get(..4).filter(|head| head.chars().all(|c| c.is_ascii_digit())) {
        return Some(head.to_string());
    }
    Some(raw)
}

fn parse_sgdb_game(item: &serde_json::Value) -> Option<SgdbGame> {
    let id = item.get("id").and_then(|i| i.as_i64())?.to_string();
    Some(SgdbGame {
        id,
        name: sgdb_non_blank(item.get("name")).unwrap_or_default(),
        year: sgdb_year(item),
    })
}

fn parse_sgdb_game_list(json: &serde_json::Value) -> Vec<SgdbGame> {
    json.get("data")
        .and_then(|d| d.as_array())
        .map(|items| items.iter().filter_map(parse_sgdb_game).collect())
        .unwrap_or_default()
}

/// Order search hits for the picker: the game the user actually typed comes
/// first, then the closest token matches, then alphabetically so repeated
/// searches of the same query return the same list.
pub fn order_sgdb_games(query: &str, mut games: Vec<SgdbGame>) -> Vec<SgdbGame> {
    let needle = query.trim().to_lowercase();
    games.sort_by(|a, b| {
        let a_exact = a.name.trim().to_lowercase() == needle;
        let b_exact = b.name.trim().to_lowercase() == needle;
        b_exact
            .cmp(&a_exact)
            .then_with(|| name_similarity(query, &b.name).cmp(&name_similarity(query, &a.name)))
            .then_with(|| a.name.cmp(&b.name))
    });
    games.truncate(SGDB_MAX_GAMES);
    games
}

/// The artwork endpoint for one game, one kind, one page. Logos are the only
/// kind that needs a `types` filter: the default catalogue leaves the animated
/// ones out, and animated logos are exactly what a launcher wants.
pub fn sgdb_art_url(kind: &str, game_id: &str, page: u32, limit: u32) -> String {
    let kind = kind.trim().to_ascii_lowercase();
    let mut url = format!(
        "{}/{}/game/{}?page={}&limit={}",
        SGDB_BASE,
        kind,
        game_id.trim(),
        page,
        limit
    );
    if kind == "logos" {
        url.push_str("&types=static,animated");
    }
    url
}

// `author` is an object ({ name, steam64, avatar }) for most arts but a plain
// string on some endpoints. Both mean the same thing to the picker.
fn sgdb_author(item: &serde_json::Value) -> Option<String> {
    let author = item.get("author")?;
    let name = match author {
        serde_json::Value::String(name) => Some(name.as_str()),
        serde_json::Value::Object(_) => author.get("name").and_then(|n| n.as_str()),
        _ => None,
    }?;
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// One art item -> the fields the picker needs. Returns `None` when the item
/// carries no usable image URL at all, so a malformed entry only costs itself.
pub fn parse_sgdb_art(item: &serde_json::Value) -> Option<SgdbArt> {
    // `url` is the canonical asset, `full` a large variant, `thumb` the last
    // resort; whatever resolved becomes the thumbnail too.
    let url = ["url", "full", "thumb"]
        .iter()
        .find_map(|key| sgdb_non_blank(item.get(*key)))?;
    let thumb = sgdb_non_blank(item.get("thumb")).unwrap_or_else(|| url.clone());
    let mime = sgdb_non_blank(item.get("mime")).unwrap_or_else(|| "image/png".to_string());
    let style = sgdb_non_blank(item.get("style"));
    // Animated SteamGridDB assets are animated WebP; the style also says so.
    let animated = style
        .as_deref()
        .is_some_and(|style| style.eq_ignore_ascii_case("animated"))
        || mime.eq_ignore_ascii_case("image/webp");
    Some(SgdbArt {
        id: u32_field(item, "id"),
        url,
        thumb,
        // Native dimensions, never the thumbnail's. No art is dropped for being
        // the "wrong" shape: the renderer labels and sorts.
        width: u32_field(item, "width"),
        height: u32_field(item, "height"),
        mime,
        style,
        author: sgdb_author(item),
        animated,
    })
}

fn parse_sgdb_art_page(json: &serde_json::Value, requested_page: u32, requested_limit: u32) -> SgdbArtPage {
    let arts: Vec<SgdbArt> = json
        .get("data")
        .and_then(|d| d.as_array())
        .map(|items| items.iter().filter_map(parse_sgdb_art).collect())
        .unwrap_or_default();
    // Pagination sits at the top level of the body. A response without it is not
    // an error: an empty last page legitimately carries no counts, so fall back
    // to what was asked for plus what actually came back.
    let page = json.get("page").and_then(json_u32).unwrap_or(requested_page);
    let limit = json.get("limit").and_then(json_u32).unwrap_or(requested_limit);
    let total = json.get("total").and_then(json_u32).unwrap_or_else(|| {
        requested_page
            .saturating_mul(requested_limit)
            .saturating_add(u32::try_from(arts.len()).unwrap_or(u32::MAX))
    });
    SgdbArtPage {
        page,
        total,
        limit,
        arts,
    }
}

// One pooled HTTP client shared by every SteamGridDB command (search, art
// pages, and fetch_metadata's SGDB + Steam chain). `get_or_init` builds it
// on the first call — one TLS handshake — and every later call clones the
// handle and reuses the pooled connections instead of issuing a fresh one.
// Timeouts keep a hung socket from stalling a Tauri command indefinitely.
// A hard build failure (TLS backend setup, effectively unrecoverable on
// retry) stays cached: each call then reports the same setup error until
// the app is restarted.
static SGDB_CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();

fn sgdb_client() -> Result<reqwest::Client, String> {
    SGDB_CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|e| format!("Could not start an HTTP client: {}", e))
        })
        .clone()
}

// Same key lookup as fetch_metadata. A missing key is an error here, not an
// empty list: the picker has to be able to say "add your key in Settings",
// whereas an empty list would read as "this game has no art".
fn sgdb_api_key(app: &AppHandle) -> Result<String, String> {
    let settings = crate::settings::get_settings(app.clone());
    settings
        .sgdb_api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
        .ok_or_else(|| SGDB_NO_KEY.to_string())
}

fn normalize_art_kind(kind: &str) -> Result<String, String> {
    let kind = kind.trim().to_ascii_lowercase();
    if SGDB_ART_KINDS.contains(&kind.as_str()) {
        Ok(kind)
    } else {
        Err("Unsupported artwork kind".to_string())
    }
}

/// Search SteamGridDB games by name, for the picker's game selector.
#[tauri::command]
pub async fn sgdb_search_games(app: AppHandle, query: String) -> Result<Vec<SgdbGame>, String> {
    let key = sgdb_api_key(&app)?;
    let client = sgdb_client()?;
    let url = format!("{}/search/autocomplete/{}", SGDB_BASE, urlencoding::encode(&query));

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await
        .map_err(|e| format!("SteamGridDB search failed: {}", e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("SteamGridDB search failed (HTTP {})", status));
    }
    let json = response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("SteamGridDB search returned invalid JSON: {}", e))?;

    let games = order_sgdb_games(&query, parse_sgdb_game_list(&json));
    log::info!("SteamGridDB search '{}' -> {} games", query, games.len());
    Ok(games)
}

/// One page of a game's artwork: `heroes`, `logos` or `grids`, with the native
/// size of every asset so the renderer can label it.
#[tauri::command]
pub async fn sgdb_art(
    app: AppHandle,
    game_id: String,
    kind: String,
    page: u32,
    limit: u32,
) -> Result<SgdbArtPage, String> {
    let kind = normalize_art_kind(&kind)?;
    let key = sgdb_api_key(&app)?;
    let client = sgdb_client()?;
    let limit = if limit == 0 {
        SGDB_DEFAULT_LIMIT
    } else {
        limit.clamp(1, SGDB_MAX_LIMIT)
    };
    let url = sgdb_art_url(&kind, &game_id, page, limit);

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await
        .map_err(|e| format!("SteamGridDB {} request failed: {}", kind, e))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("SteamGridDB {} request failed (HTTP {})", kind, status));
    }
    let json = response
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("SteamGridDB {} response was not valid JSON: {}", kind, e))?;

    let art_page = parse_sgdb_art_page(&json, page, limit);
    log::info!(
        "SteamGridDB {} game {} page {} -> {} arts (total {})",
        kind,
        game_id,
        art_page.page,
        art_page.arts.len(),
        art_page.total
    );
    Ok(art_page)
}

/* ── Automatic artwork for a new game ───────────────────────────────────────
After a game is created, SILO asks SteamGridDB for its catalogue and takes
the first logo and the first hero, in the API's own order. Two rules keep
this honest:

* the autocomplete hit has to be a confident match for the title, otherwise
  no art is applied at all (sequels and remasters share too many words for
  "first result" to be safe);
* each image is downloaded into the same wallpapers folder the manual logo /
  wallpaper pickers write to, so the game record holds a local file exactly
  as if the user had chosen it — the remote URL is never stored.

Every failure is reported in `reason` and leaves the field empty; adding a
game never fails because artwork could not be fetched.
───────────────────────────────────────────────────────────────────────── */

/// Token-overlap score a candidate title needs before its artwork is used. At
/// 60 a sequel still qualifies only when it really is the same words; a hit
/// that merely shares one word does not.
pub const AUTO_ART_MIN_CONFIDENCE: u8 = 60;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoArtResult {
    /// Local path now stored on the game, when one was applied.
    pub logo: Option<String>,
    pub wallpaper: Option<String>,
    /// The updated game record so the renderer can refresh its list entry.
    pub game: Option<serde_json::Value>,
    /// Why artwork was not applied, for the log and the modal hint.
    pub reason: Option<String>,
}

impl AutoArtResult {
    fn skipped(reason: impl Into<String>) -> Self {
        AutoArtResult {
            logo: None,
            wallpaper: None,
            game: None,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtPreview {
    pub logo: Option<String>,
    pub wallpaper: Option<String>,
}

/// Drops bracketed segments (`(1993)`, `[Remastered]`): SteamGridDB titles often
/// carry a year or edition there, and that is metadata, not the title.
fn without_bracketed(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0i32;
    for ch in text.chars() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Number tokens in a title, which is what separates a game from its sequel.
/// Arabic and Roman numerals both count: "Hades II" is not "Hades".
fn number_tokens(text: &str) -> std::collections::BTreeSet<String> {
    const ROMAN: [&str; 10] = ["i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x"];
    without_bracketed(&text.to_lowercase())
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .filter(|token| token.chars().all(|c| c.is_ascii_digit()) || ROMAN.contains(token))
        .map(str::to_string)
        .collect()
}

/// True when the autocomplete hit is close enough to the title being added.
///
/// Word overlap alone is not enough: "Portal" against "Portal 2" scores 67, and
/// applying the sequel's art to the first game is exactly the wrong result. A
/// title whose numbers differ is never a confident match, so sequels,
/// remasters numbered differently, and year suffixes are left for the user.
pub fn is_confident_title_match(query: &str, candidate: &str) -> bool {
    name_similarity(query, candidate) >= AUTO_ART_MIN_CONFIDENCE && number_tokens(query) == number_tokens(candidate)
}

/// The first art in API order. SteamGridDB returns its relevance ordering, so
/// "first" is a decision of the API, never of ours.
pub fn first_art(arts: Vec<SgdbArt>) -> Option<SgdbArt> {
    arts.into_iter().next()
}

/// The first art that is an actual image.
///
/// SteamGridDB ranks animated art highly, and animated assets are served as
/// WebM — an `<img>` cannot draw one and neither can the rail, so an automatic
/// pick that lands on one looks exactly like "no artwork was found" (this is
/// what happens for Cyberpunk 2077, whose top logo is animated). Still images
/// win outright; an animated entry is kept only as a last resort when the
/// catalogue has nothing else, and even then only if its bytes are an image.
pub fn first_static_art(arts: Vec<SgdbArt>) -> Option<SgdbArt> {
    let mut animated_fallback: Option<SgdbArt> = None;
    for art in arts {
        if !art.mime.to_ascii_lowercase().starts_with("image/") {
            continue;
        }
        let animated = art
            .style
            .as_deref()
            .is_some_and(|style| style.eq_ignore_ascii_case("animated"));
        if animated {
            if animated_fallback.is_none() {
                animated_fallback = Some(art);
            }
            continue;
        }
        return Some(art);
    }
    animated_fallback
}

/// File extension for a downloaded asset: the URL's own extension when it has
/// a usable one, else what the API said the bytes are, else png.
pub fn art_extension(url: &str, mime: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let from_url = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match from_url.as_str() {
        "png" => return "png",
        "jpg" | "jpeg" => return "jpg",
        "webp" => return "webp",
        "gif" => return "gif",
        _ => {}
    }
    match mime.to_ascii_lowercase().as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
}

// One automatic attempt per game per app run: reopening the Edit modal must not
// spend another API request on a game whose artwork is already settled.
static AUTO_ART_ATTEMPTED: OnceLock<std::sync::Mutex<HashSet<String>>> = OnceLock::new();

fn claim_auto_art_attempt(game_id: &str) -> bool {
    let seen = AUTO_ART_ATTEMPTED.get_or_init(|| std::sync::Mutex::new(HashSet::new()));
    match seen.lock() {
        Ok(mut guard) => guard.insert(game_id.to_string()),
        Err(poisoned) => poisoned.into_inner().insert(game_id.to_string()),
    }
}

// Preview lookups for the discovery prompt are cached per title: the prompt can
// be re-shown (dismissed candidate) without asking SteamGridDB again.
static PREVIEW_CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, ArtPreview>>> = OnceLock::new();

async fn search_best_match(client: &reqwest::Client, key: &str, title: &str) -> Option<SgdbGame> {
    let games = order_sgdb_games(title, sgdb_games(client, key, title).await);
    if games.is_empty() {
        log::info!("auto-art: no SteamGridDB entry for '{}'", title);
        return None;
    }
    // The top hit is normally the right game, but a re-release or a spin-off can
    // outrank it. The list is already ordered best-first, so the first entry
    // that is confident is the answer — rejecting the lookup over one bad first
    // hit is what made a correct title come back with no artwork.
    for game in &games {
        if is_confident_title_match(title, &game.name) {
            if game.name.trim().to_lowercase() != title.trim().to_lowercase() {
                log::info!("auto-art: '{}' matched SteamGridDB entry '{}'", title, game.name);
            }
            return Some(game.clone());
        }
    }
    let closest = &games[0];
    log::info!(
        "auto-art: '{}' has no confident match (closest '{}', {}), skipping",
        title,
        closest.name,
        name_similarity(title, &closest.name)
    );
    None
}

async fn fetch_first_art(client: &reqwest::Client, key: &str, game_id: &str, kind: &str) -> Option<SgdbArt> {
    let url = sgdb_art_url(kind, game_id, 0, 10);
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await
        .map_err(|error| log::warn!("auto-art: {} request failed: {}", kind, error))
        .ok()?;
    if !response.status().is_success() {
        note_rate_limit(&response);
        log::warn!("auto-art: {} -> HTTP {}", kind, response.status());
        return None;
    }
    let json = response.json::<serde_json::Value>().await.ok()?;
    let arts = parse_sgdb_art_page(&json, 0, 10).arts;
    let picked = first_static_art(arts.clone());
    if picked.is_none() {
        log::info!(
            "auto-art: no usable still {} among {} entries for game {}",
            kind,
            arts.len(),
            game_id
        );
    }
    picked
}

async fn download_art(app: &AppHandle, game_id: &str, kind: &str, art: &SgdbArt) -> Result<String, String> {
    let bytes = crate::commands::fetch_wallpaper_bytes(art.url.clone()).await?;
    let dir = crate::commands::get_wallpapers_dir(app);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create the artwork folder: {}", e))?;
    let ext = art_extension(&art.url, &art.mime);
    let file_name = if kind == "logo" {
        format!("logo_{}.{}", game_id, ext)
    } else {
        format!("{}.{}", game_id, ext)
    };
    let dest = dir.join(file_name);
    std::fs::write(&dest, bytes).map_err(|e| format!("Could not save the artwork: {}", e))?;
    Ok(dest.to_string_lossy().into_owned())
}

/// Fetches and stores the first logo and the first hero for a game that has no
/// artwork yet. Only empty fields are filled (`set_game_artwork` refuses to
/// replace an existing image), so a manual choice is never overwritten.
#[tauri::command]
pub async fn auto_art_for_game(app: AppHandle, game_id: String) -> AutoArtResult {
    auto_art(app, game_id, true).await
}

/// The lookup itself, shared by the command and the startup sweep.
/// `once_per_session` keeps a reopened Edit modal from spending a second set of
/// API calls on a game whose artwork is already settled.
async fn auto_art(app: AppHandle, game_id: String, once_per_session: bool) -> AutoArtResult {
    let games = crate::commands::get_games(app.clone());
    let Some(game) = games.iter().find(|g| g.id == game_id).cloned() else {
        return AutoArtResult::skipped("That game is no longer in the library.");
    };

    let has_logo = game.logo_path.as_deref().is_some_and(|p| !p.trim().is_empty());
    let has_wallpaper = game.wallpaper.as_deref().is_some_and(|p| !p.trim().is_empty());
    if has_logo && has_wallpaper {
        return AutoArtResult {
            logo: game.logo_path.clone(),
            wallpaper: game.wallpaper.clone(),
            game: serde_json::to_value(&game).ok(),
            reason: None,
        };
    }

    // A missing key is not an attempt: adding the key later in this same run
    // must still let the next game fetch its artwork.
    let key = match sgdb_api_key(&app) {
        Ok(key) => key,
        Err(reason) => return AutoArtResult::skipped(reason),
    };
    if once_per_session && !claim_auto_art_attempt(&game_id) {
        return AutoArtResult::skipped("Artwork for this game was already looked up in this session.");
    }
    if !once_per_session {
        // The sweep is the authority for the session: claim the game so the
        // Edit modal does not run the same three requests again.
        claim_auto_art_attempt(&game_id);
    }

    let client = match sgdb_client() {
        Ok(client) => client,
        Err(reason) => return AutoArtResult::skipped(reason),
    };
    // A game the user matched by hand goes straight to that entry: searching by
    // name again would quietly undo their choice on the next sweep.
    let remembered = game
        .sgdb_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());

    let match_game = match remembered {
        Some(id) => SgdbGame {
            id: id.to_string(),
            name: game.name.clone(),
            year: None,
        },
        None => match search_best_match(&client, &key, &game.name).await {
            Some(found) => {
                forget_art_failure(&app, &game.name);
                found
            }
            None => {
                // Remembering this is what stops the next launch spending the
                // same request on a title SteamGridDB simply does not have.
                remember_art_failure(&app, &game.name);
                return AutoArtResult::skipped(format!(
                    "SteamGridDB has no confident match for \"{}\". Pick the art manually.",
                    game.name
                ));
            }
        },
    };

    // Whatever was matched, remembered or guessed, is worth keeping: the next
    // lookup then costs one art request instead of a search plus art.
    if remembered.is_none() {
        let _ = crate::commands::remember_sgdb_game(&app, &game_id, &match_game.id);
    }

    let mut notes: Vec<String> = Vec::new();
    let mut logo_path = None;
    let mut wallpaper_path = None;

    if !has_logo {
        match fetch_first_art(&client, &key, &match_game.id, "logos").await {
            Some(art) => match download_art(&app, &game_id, "logo", &art).await {
                Ok(path) => logo_path = Some(path),
                Err(error) => notes.push(format!("logo: {}", error)),
            },
            None => notes.push("no logo found".to_string()),
        }
    }
    if !has_wallpaper {
        match fetch_first_art(&client, &key, &match_game.id, "heroes").await {
            Some(art) => match download_art(&app, &game_id, "wallpaper", &art).await {
                Ok(path) => wallpaper_path = Some(path),
                Err(error) => notes.push(format!("hero: {}", error)),
            },
            None => notes.push("no hero found".to_string()),
        }
    }

    let updated = crate::commands::set_game_artwork(&app, &game_id, logo_path.clone(), wallpaper_path.clone())
        .ok()
        .flatten();

    AutoArtResult {
        logo: updated.as_ref().and_then(|g| g.logo_path.clone()).or(logo_path),
        wallpaper: updated.as_ref().and_then(|g| g.wallpaper.clone()).or(wallpaper_path),
        game: updated.as_ref().and_then(|g| serde_json::to_value(g).ok()),
        reason: if notes.is_empty() { None } else { Some(notes.join("; ")) },
    }
}

/// How many artwork fields a serialized game record actually carries.
fn art_field_count(game: &serde_json::Value) -> usize {
    ["logoPath", "wallpaper"]
        .iter()
        .filter(|key| {
            game.get(**key)
                .and_then(|value| value.as_str())
                .is_some_and(|value| !value.trim().is_empty())
        })
        .count()
}

/// Fills in the logo and wallpaper of every game that is still missing one.
///
/// Adding a game opens Edit and fetches its artwork there, but that path is tied
/// to a modal the user may close straight away, so a game can stay in the
/// library with no artwork at all and nothing ever asks again. This runs once
/// per launch, after the window is up, and walks the games that still have
/// none. Each record that gains art is pushed to the renderer as `art-updated`,
/// so the tile and the hero fill in without a restart.
///
/// It is deliberately sequential and spaced out: SteamGridDB rate limits per
/// minute, and a library of a few dozen games must not trip it.
/* ── Titles SteamGridDB does not have ──────────────────────────────────────
   A first launch with fifty games used to spend fifty lookups on titles that
   will never match, again on every launch and usually against the same answer.
   The failures are remembered for a week, so a sweep only pays for them once;
   an explicit retry, which the Settings row offers, ignores the memory. */
const ART_FAILURE_COOLDOWN: i64 = 7 * 24 * 60 * 60;

fn art_failures_path(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("art-failures.json"))
}

fn load_art_failures(app: &AppHandle) -> std::collections::HashMap<String, i64> {
    art_failures_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_art_failures(app: &AppHandle, failures: &std::collections::HashMap<String, i64>) {
    let Some(path) = art_failures_path(app) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(failures) {
        let _ = std::fs::write(path, text);
    }
}

fn art_failure_key(name: &str) -> String {
    name.trim().to_lowercase()
}

fn remember_art_failure(app: &AppHandle, name: &str) {
    let mut failures = load_art_failures(app);
    failures.insert(art_failure_key(name), chrono::Utc::now().timestamp());
    save_art_failures(app, &failures);
}

fn forget_art_failure(app: &AppHandle, name: &str) {
    let key = art_failure_key(name);
    let mut failures = load_art_failures(app);
    if failures.remove(&key).is_some() {
        save_art_failures(app, &failures);
    }
}

/// Whether a title failed to match recently enough that a sweep should skip it.
fn art_failure_is_recent(failures: &std::collections::HashMap<String, i64>, name: &str) -> bool {
    let Some(failed_at) = failures.get(&art_failure_key(name)) else {
        return false;
    };
    let age = chrono::Utc::now().timestamp() - failed_at;
    age < ART_FAILURE_COOLDOWN
}

pub async fn sweep_missing_art(app: AppHandle, ignore_cooldown: bool) {
    let games = crate::commands::get_games(app.clone());
    let missing: Vec<(String, String, usize)> = games
        .iter()
        .filter(|game| {
            let has_logo = game.logo_path.as_deref().is_some_and(|p| !p.trim().is_empty());
            let has_wallpaper = game.wallpaper.as_deref().is_some_and(|p| !p.trim().is_empty());
            !(has_logo && has_wallpaper)
        })
        .map(|game| {
            let has_logo = game.logo_path.as_deref().is_some_and(|p| !p.trim().is_empty());
            let has_wallpaper = game.wallpaper.as_deref().is_some_and(|p| !p.trim().is_empty());
            (
                game.id.clone(),
                game.name.clone(),
                usize::from(has_logo) + usize::from(has_wallpaper),
            )
        })
        .collect();

    if missing.is_empty() {
        return;
    }
    // No key is not a failure to report: the user has not set one up yet.
    if let Err(reason) = sgdb_api_key(&app) {
        log::info!("auto-art sweep: {} game(s) missing artwork, not fetching: {}", missing.len(), reason);
        return;
    }

    let total = missing.len();
    log::info!("auto-art sweep: {} game(s) missing artwork", total);
    // The window shows a progress bar for this work: the first launch after
    // adding a few games is otherwise a silent wait with letter tiles on screen.
    let _ = app.emit(
        "art-progress",
        serde_json::json!({ "kind": "start", "done": 0, "total": total, "filled": 0 }),
    );
    let failures = load_art_failures(&app);
    let mut filled = 0usize;
    let mut done = 0usize;
    for (id, name, before) in missing {
        if !ignore_cooldown && art_failure_is_recent(&failures, &name) {
            log::info!("auto-art sweep: '{}' had no match recently, skipping", name);
            done += 1;
            let _ = app.emit(
                "art-progress",
                serde_json::json!({ "kind": "step", "done": done, "total": total, "filled": filled }),
            );
            continue;
        }
        // A 429 means the rest of this sweep would only earn more of them.
        if let Some(wait) = rate_limit_wait() {
            if wait > Duration::from_secs(60) {
                log::warn!("auto-art sweep: rate limited for {:?}, stopping", wait);
                break;
            }
            std::thread::sleep(wait);
        }
        let result = auto_art(app.clone(), id, false).await;
        let after = result.game.as_ref().map(art_field_count).unwrap_or(before);
        if let Some(reason) = &result.reason {
            log::warn!("auto-art sweep: '{}' incomplete: {}", name, reason);
        }
        if after > before {
            filled += 1;
            log::info!("auto-art sweep: '{}' now has artwork", name);
            if let Some(game) = result.game {
                let _ = app.emit("art-updated", game);
            }
        }
        done += 1;
        let _ = app.emit(
            "art-progress",
            serde_json::json!({ "kind": "step", "done": done, "total": total, "filled": filled }),
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    log::info!("auto-art sweep: filled {} of {}", filled, total);
    let _ = app.emit(
        "art-progress",
        serde_json::json!({ "kind": "done", "done": total, "total": total, "filled": filled }),
    );
}

/// Runs the artwork sweep on demand, for a library that grew after launch.
///
/// The startup sweep only sees the games that existed when it ran, and setup
/// adds games while it is on screen, so without this a game added during setup
/// waited for the next launch. It answers at once and does the work in the
/// background, reporting through the same `art-progress` events.
#[tauri::command]
pub fn sweep_art(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Asked for on purpose, so a title that failed before is tried again.
        sweep_missing_art(app, true).await;
    });
}

/// Checks a key before it is saved, so a bad paste shows up on the spot instead
/// of as games that quietly never get artwork.
#[tauri::command]
pub async fn validate_sgdb_key(key: String) -> Result<String, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Paste a key first.".to_string());
    }
    let client = sgdb_client()?;
    let url = format!(
        "{}/search/autocomplete/{}",
        SGDB_BASE,
        urlencoding::encode("portal")
    );
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await
        .map_err(|e| format!("Could not reach SteamGridDB: {}", e))?;
    let status = response.status();
    if status.is_success() {
        return Ok("Key works.".to_string());
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err("SteamGridDB rejected that key.".to_string());
    }
    Err(format!("SteamGridDB answered HTTP {}.", status))
}

/// How much of the library is still without artwork, for the Settings row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtStatus {
    pub total: usize,
    pub missing: usize,
    pub has_key: bool,
}

#[tauri::command]
pub fn art_status(app: AppHandle) -> ArtStatus {
    let games = crate::commands::get_games(app.clone());
    let missing = games
        .iter()
        .filter(|game| {
            let has_logo = game.logo_path.as_deref().is_some_and(|p| !p.trim().is_empty());
            let has_wallpaper = game.wallpaper.as_deref().is_some_and(|p| !p.trim().is_empty());
            !(has_logo && has_wallpaper)
        })
        .count();
    ArtStatus {
        total: games.len(),
        missing,
        has_key: sgdb_api_key(&app).is_ok(),
    }
}

/// What the discovery prompt can actually draw.
///
/// A thumbnail is normally the cheap choice, but for animated art SteamGridDB
/// serves the thumbnail as a WebM clip — an `<img>` shows nothing for it, which
/// reads as "no artwork found". In that case the full still image is used
/// instead.
fn preview_url(art: &SgdbArt) -> String {
    let thumb = art.thumb.trim();
    if thumb.is_empty() || thumb.to_ascii_lowercase().ends_with(".webm") {
        return art.url.clone();
    }
    thumb.to_string()
}

/// First logo and first hero for a name, as remote thumbnails only. Used by the
/// discovery prompt so a candidate can show what it will look like; nothing is
/// downloaded or stored here.
#[tauri::command]
pub async fn preview_art(app: AppHandle, name: String) -> ArtPreview {
    let cache_key = name.trim().to_ascii_lowercase();
    if let Some(hit) = PREVIEW_CACHE
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .ok()
        .and_then(|cache| cache.get(&cache_key).cloned())
    {
        return hit;
    }

    let empty = ArtPreview {
        logo: None,
        wallpaper: None,
    };
    let Ok(key) = sgdb_api_key(&app) else {
        return empty;
    };
    let Ok(client) = sgdb_client() else {
        return empty;
    };
    let Some(match_game) = search_best_match(&client, &key, &name).await else {
        return empty;
    };
    let logo = fetch_first_art(&client, &key, &match_game.id, "logos")
        .await
        .map(|art| preview_url(&art));
    // Heroes are the widescreen banner a game page uses. Grids are the vertical
    // capsule art SteamGridDB users mostly pick, so they carry the game even
    // when no hero exists for it.
    let wallpaper = match fetch_first_art(&client, &key, &match_game.id, "heroes").await {
        Some(art) => Some(preview_url(&art)),
        None => fetch_first_art(&client, &key, &match_game.id, "grids")
            .await
            .map(|art| preview_url(&art)),
    };
    let preview = ArtPreview { logo, wallpaper };
    if preview.logo.is_some() || preview.wallpaper.is_some() {
        if let Ok(mut cache) = PREVIEW_CACHE
            .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
            .lock()
        {
            cache.insert(cache_key, preview.clone());
        }
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art(url: &str, mime: &str, style: Option<&str>) -> SgdbArt {
        SgdbArt {
            id: 1,
            url: url.to_string(),
            thumb: url.to_string(),
            width: 1,
            height: 1,
            mime: mime.to_string(),
            style: style.map(str::to_string),
            animated: style.is_some_and(|s| s.eq_ignore_ascii_case("animated")),
            author: None,
        }
    }

    /// The animated logo that ranks first for a game like Cyberpunk 2077 is a
    /// WebM file: an <img> shows nothing for it, so the automatic pick has to
    /// reach past it for the still image underneath.
    #[test]
    fn first_static_art_skips_animated_webm_and_takes_the_next_image() {
        let arts = vec![
            art("https://cdn2.steamgriddb.com/logo/animated.webm", "video/webm", Some("animated")),
            art("https://cdn2.steamgriddb.com/logo/static.png", "image/png", Some("official")),
        ];
        let picked = first_static_art(arts).expect("the still logo should win");
        assert_eq!(picked.url, "https://cdn2.steamgriddb.com/logo/static.png");
    }

    #[test]
    fn first_static_art_falls_back_to_an_animated_image_but_never_to_video() {
        let only_video = vec![art("https://x/a.webm", "video/webm", Some("animated"))];
        assert!(first_static_art(only_video).is_none());

        let animated_image = vec![art("https://x/a.webp", "image/webp", Some("animated"))];
        assert!(first_static_art(animated_image).is_some());
    }

    /// Only static images matter for the auto-art path; without any, the caller
    /// reports "no logo found" instead of downloading something unrenderable.
    #[test]
    fn first_static_art_returns_none_for_an_empty_page() {
        assert!(first_static_art(Vec::new()).is_none());
    }

    #[test]
    fn automatic_art_takes_the_first_result_in_api_order() {
        let json = serde_json::json!({
            "data": [
                { "id": 1, "url": "https://cdn2.steamgriddb.com/logo/first.png", "thumb": "t1", "width": 512, "height": 512 },
                { "id": 2, "url": "https://cdn2.steamgriddb.com/logo/second.png", "thumb": "t2", "width": 1024, "height": 1024 },
                { "id": 3, "url": "https://cdn2.steamgriddb.com/logo/third.png", "thumb": "t3", "width": 2048, "height": 2048 }
            ],
            "total": 3
        });
        let page = parse_sgdb_art_page(&json, 0, 10);
        let chosen = first_art(page.arts).expect("one art");
        assert_eq!(chosen.id, 1, "the API's first result must win, never a random one");
        assert!(chosen.url.ends_with("first.png"));
    }

    #[test]
    fn automatic_art_needs_a_confident_title_match() {
        assert!(is_confident_title_match("Hades", "Hades"));
        assert!(is_confident_title_match("hades", "HADES"));
        // A subtitle is still the same game…
        assert!(is_confident_title_match("The Witcher 3", "The Witcher 3: Wild Hunt"));
        // …but a different number is always a different game.
        assert!(!is_confident_title_match("Portal", "Portal 2"));
        assert!(!is_confident_title_match("Hades II", "Hades"));
        assert!(!is_confident_title_match("Grand Theft Auto V", "Grand Theft Auto IV"));
        assert!(!is_confident_title_match("The Witcher 3", "Cyberpunk 2077"));
        // A bracketed year is edition metadata, not a different game.
        assert!(is_confident_title_match("Doom", "DOOM (1993)"));
        assert!(is_confident_title_match("The Witcher 3", "The Witcher 3 (2015)"));
    }

    #[test]
    fn art_extension_prefers_the_url_and_falls_back_to_the_mime() {
        assert_eq!(art_extension("https://x/y/logo.png", "image/webp"), "png");
        assert_eq!(art_extension("https://x/y/hero.webp?t=1", ""), "webp");
        assert_eq!(art_extension("https://x/y/asset", "image/jpeg"), "jpg");
        assert_eq!(art_extension("https://x/y/asset", "image/gif"), "gif");
        assert_eq!(art_extension("https://x/y/asset", ""), "png");
    }

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
    fn name_similarity_ignores_accents() {
        // A folder called POKEMON EMERALD must still find "Pokémon Emerald
        // Version": the accent used to make the first token unrelated, and the
        // game then sat in the library with no artwork at all.
        assert_eq!(name_similarity("POKEMON EMERALD", "Pokémon Emerald"), 100);
        assert!(is_confident_title_match("POKEMON EMERALD", "Pokémon Emerald Version"));
        assert!(is_confident_title_match("Cafe International", "Café International"));
        // "Version" is catalogue wording: the real entry must beat a romhack
        // that merely shares the first two words.
        assert_eq!(name_similarity("POKEMON EMERALD", "Pokémon Emerald Version"), 100);
        assert!(
            name_similarity("POKEMON EMERALD", "Pokémon Emerald Version")
                > name_similarity("POKEMON EMERALD", "Pokémon Emerald Crest")
        );
        // Folding accents must not merge different numbers.
        assert!(!is_confident_title_match("Pokémon Emerald 2", "Pokémon Emerald"));
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
        assert!((60..100).contains(&score), "got {}", score);
    }

    #[test]
    fn name_similarity_single_char_tokens() {
        // Single non-digit letters normalize away -> empty set -> 0.
        assert_eq!(name_similarity("A", "A"), 0);
        // Single-digit tokens are kept, so "7" == "7".
        assert_eq!(name_similarity("7", "7"), 100);
    }

    #[test]
    fn sgdb_art_url_for_heroes_asks_for_the_whole_catalogue() {
        let url = sgdb_art_url("heroes", "1234", 2, 50);
        assert!(url.contains("/heroes/game/1234"), "got {}", url);
        assert!(url.contains("page=2"), "got {}", url);
        assert!(url.contains("limit=50"), "got {}", url);
        // Heroes must not be narrowed: every style and type is wanted.
        assert!(!url.contains("types="), "got {}", url);
    }

    #[test]
    fn sgdb_art_url_only_filters_types_for_logos() {
        let logos = sgdb_art_url("logos", "1234", 0, 50);
        assert!(logos.contains("/logos/game/1234"), "got {}", logos);
        assert!(logos.contains("types=static,animated"), "got {}", logos);

        let grids = sgdb_art_url("grids", "1234", 0, 50);
        assert!(grids.contains("/grids/game/1234"), "got {}", grids);
        assert!(!grids.contains("types="), "got {}", grids);
    }

    #[test]
    fn parse_sgdb_art_reads_every_field() {
        let item = serde_json::json!({
          "id": 42,
          "url": "https://cdn.example/full.png",
          "thumb": "https://cdn.example/thumb.png",
          "width": 1920,
          "height": 620,
          "mime": "image/png",
          "style": "official",
          "author": { "name": "Artist", "steam64": "76561198000000000", "avatar": "https://cdn.example/a.png" },
          "upvotes": 5,
          "downvotes": 0,
          "nsfw": false,
          "humor": false,
          "language": "en"
        });
        let art = parse_sgdb_art(&item).expect("full fixture must parse");
        assert_eq!(art.id, 42);
        assert_eq!(art.url, "https://cdn.example/full.png");
        assert_eq!(art.thumb, "https://cdn.example/thumb.png");
        assert_eq!(art.width, 1920);
        assert_eq!(art.height, 620);
        assert_eq!(art.mime, "image/png");
        assert_eq!(art.style.as_deref(), Some("official"));
        assert_eq!(art.author.as_deref(), Some("Artist"));
        assert!(!art.animated);
    }

    #[test]
    fn parse_sgdb_art_flags_animated_assets() {
        let webp = serde_json::json!({
          "id": 7,
          "url": "https://cdn.example/anim.webp",
          "mime": "image/webp",
          "style": "official"
        });
        assert!(parse_sgdb_art(&webp).expect("webp parses").animated);

        let styled = serde_json::json!({
          "id": 8,
          "url": "https://cdn.example/anim.png",
          "mime": "image/png",
          "style": "animated"
        });
        assert!(parse_sgdb_art(&styled).expect("styled parses").animated);
    }

    #[test]
    fn parse_sgdb_art_falls_back_from_url_to_full_to_thumb() {
        let full = serde_json::json!({ "id": 1, "full": "https://cdn.example/full.png" });
        let art = parse_sgdb_art(&full).expect("full-only item parses");
        assert_eq!(art.url, "https://cdn.example/full.png");
        // No thumb of its own, so the resolved url stands in.
        assert_eq!(art.thumb, "https://cdn.example/full.png");

        let thumb = serde_json::json!({ "id": 2, "thumb": "https://cdn.example/thumb.png" });
        let art = parse_sgdb_art(&thumb).expect("thumb-only item parses");
        assert_eq!(art.url, "https://cdn.example/thumb.png");
        assert_eq!(art.thumb, "https://cdn.example/thumb.png");

        let urlless = serde_json::json!({ "id": 3, "width": 1920, "height": 620, "mime": "image/png" });
        assert!(parse_sgdb_art(&urlless).is_none());
    }

    #[test]
    fn parse_sgdb_art_accepts_author_as_a_plain_string() {
        let item = serde_json::json!({
          "id": 9,
          "url": "https://cdn.example/x.png",
          "author": "Some Person"
        });
        assert_eq!(
            parse_sgdb_art(&item).expect("string author parses").author.as_deref(),
            Some("Some Person")
        );

        let blank = serde_json::json!({ "id": 10, "url": "https://cdn.example/y.png", "author": "" });
        assert_eq!(parse_sgdb_art(&blank).expect("blank author parses").author, None);
    }

    #[test]
    fn order_sgdb_games_puts_the_exact_match_first() {
        let games = vec![
            SgdbGame {
                id: "1".to_string(),
                name: "Portal Stories".to_string(),
                year: Some("2016".to_string()),
            },
            SgdbGame {
                id: "2".to_string(),
                name: "Portal 3".to_string(),
                year: None,
            },
            SgdbGame {
                id: "3".to_string(),
                name: "  portal ".to_string(),
                year: Some("2007".to_string()),
            },
            SgdbGame {
                id: "4".to_string(),
                name: "Unrelated".to_string(),
                year: None,
            },
        ];

        let ordered = order_sgdb_games("Portal", games);
        // The exact match arrives last but must come out first.
        assert_eq!(ordered[0].id, "3");
        // The rest follow by similarity, so the unrelated game is last.
        assert_eq!(ordered[3].id, "4");
        let second = name_similarity("Portal", &ordered[1].name);
        let third = name_similarity("Portal", &ordered[2].name);
        assert!(second >= third, "{} vs {}", second, third);
        // Equal scores fall back to the name, for a stable list.
        assert_eq!(ordered[1].name, "Portal 3");
        assert_eq!(ordered[2].name, "Portal Stories");
    }

    #[test]
    fn order_sgdb_games_caps_the_result_at_twenty() {
        let games: Vec<SgdbGame> = (0..25)
            .map(|i| SgdbGame {
                id: i.to_string(),
                name: format!("Game {}", i),
                year: None,
            })
            .collect();
        assert_eq!(order_sgdb_games("Game", games).len(), 20);
    }

    #[test]
    fn a_recent_failure_is_skipped_and_an_old_one_is_tried_again() {
        let mut failures = std::collections::HashMap::new();
        let now = chrono::Utc::now().timestamp();
        failures.insert(art_failure_key("Hollow Knight"), now);
        assert!(art_failure_is_recent(&failures, " hollow KNIGHT "));
        failures.insert(art_failure_key("Old Game"), now - ART_FAILURE_COOLDOWN - 1);
        assert!(!art_failure_is_recent(&failures, "Old Game"));
        assert!(!art_failure_is_recent(&failures, "Never Tried"));
    }
}
