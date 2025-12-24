use actix_web::{App, HttpRequest, HttpResponse, HttpServer, Responder, get, web};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use clap::Parser;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

static SVG_TEMPLATE: &str = include_str!("../assets/counter.svg");
const IMG_WIDTH: u32 = 45;
const IMG_HEIGHT: u32 = 100;
const PAD_LENGTH: usize = 7;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Address to bind to
    #[arg(long, env("BIND_ADDRESS"), default_value = "127.0.0.1")]
    host: String,

    /// Port to bind to
    #[arg(long, env("BIND_PORT"), default_value_t = 8080)]
    port: u16,

    /// Path to SQLite database
    #[arg(long, env("DB_PATH"), default_value = "db/count.db")]
    db_path: String,

    /// Path to assets directory
    #[arg(long, env("ASSETS_PATH"), default_value = "assets/theme")]
    assets_path: String,

    /// Default theme to use if not specified in query
    #[arg(long, env("DEFAULT_THEME"), default_value = "moebooru")]
    default_theme: String,

    /// Authentication token for setting counts
    #[arg(long, env("API_AUTH_TOKEN"))]
    api_auth_token: Option<String>,
}

struct AppState {
    db: SqlitePool,
    themes: Arc<ThemeMap>,
    default_theme: String,
    api_auth_token: Option<String>,
}

type DigitMap = HashMap<char, String>;
type ThemeMap = HashMap<Arc<str>, ThemeData>;

struct ThemeData {
    id_to_uri: HashMap<String, Arc<str>>,
    digits: DigitMap,
}

async fn init_db(path: &str) -> SqlitePool {
    info!("initializing database at {}", path);

    if let Some(parent) = Path::new(path).parent() {
        std::fs::create_dir_all(parent).expect("create db dir");
    }

    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(path)
        .expect("create db file");

    let pool = SqlitePool::connect(&format!("sqlite://{path}"))
        .await
        .expect("connect db");

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS tb_count (
            id   INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT UNIQUE NOT NULL,
            num  INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&pool)
    .await
    .expect("create table");

    info!("database initialized");
    pool
}

async fn increment(pool: &SqlitePool, name: &str) -> i64 {
    debug!("incrementing counter for {}", name);

    sqlx::query(
        "INSERT INTO tb_count (name, num) VALUES (?1, 1)
         ON CONFLICT(name) DO UPDATE SET num = num + 1",
    )
    .bind(name)
    .execute(pool)
    .await
    .ok();

    let count = sqlx::query_scalar::<_, i64>("SELECT num FROM tb_count WHERE name = ?1")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    debug!("counter {} now at {}", name, count);
    count
}

fn mime_for(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "gif" => "image/gif",
        "jpg" | "jpeg" => "image/jpeg",
        _ => "application/octet-stream",
    }
}

fn load_themes(dir: &str) -> ThemeMap {
    info!("loading themes from {}", dir);
    let mut themes = ThemeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        warn!("failed to read themes directory");
        return themes;
    };

    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }

        let theme_name: Arc<str> = entry.file_name().to_string_lossy().into();
        let mut uri_to_id: HashMap<Arc<str>, String> = HashMap::new();
        let mut digits = DigitMap::new();
        let mut id_counter = 0;
        debug!("loading theme: {}", theme_name);

        let Ok(files) = std::fs::read_dir(entry.path()) else {
            warn!("failed to read theme directory: {}", theme_name);
            continue;
        };

        for file in files.flatten() {
            let path = file.path();
            if let Some((digit, uri)) = load_digit(&path) {
                let id = uri_to_id.entry(uri.clone()).or_insert_with(|| {
                    let new_id = format!("i{}", id_counter);
                    id_counter += 1;
                    new_id
                });
                digits.insert(digit, id.clone());
            }
        }

        if !digits.is_empty() {
            let id_to_uri: HashMap<String, Arc<str>> =
                uri_to_id.into_iter().map(|(uri, id)| (id, uri)).collect();
            info!("loaded theme: {} with {} digits", theme_name, digits.len());
            themes.insert(theme_name, ThemeData { id_to_uri, digits });
        }
    }

    info!("loaded {} themes total", themes.len());
    themes
}

fn load_digit(path: &Path) -> Option<(char, Arc<str>)> {
    let stem = path.file_stem()?.to_str()?;
    let digit = stem.chars().next()?;
    if !digit.is_ascii_digit() {
        return None;
    }

    let bytes = std::fs::read(path).ok()?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let uri = format!("data:{};base64,{}", mime_for(ext), BASE64.encode(&bytes));

    debug!("loaded digit {} ({})", digit, path.display());
    Some((digit, uri.into()))
}

fn render_svg(theme_data: &ThemeData, count: i64) -> String {
    let text = format!("{:0>width$}", count, width = PAD_LENGTH);

    let mut used_ids = HashSet::new();
    let uses = text
        .chars()
        .enumerate()
        .filter_map(|(i, ch)| {
            theme_data.digits.get(&ch).map(|id| {
                used_ids.insert(id.clone());
                let x = i as u32 * IMG_WIDTH;
                format!(r##"<use href="#{id}" x="{x}" y="0" width="{IMG_WIDTH}" height="{IMG_HEIGHT}"/>"##)
            })
        })
        .collect::<Vec<_>>()
        .join("");

    let defs = used_ids
        .iter()
        .filter_map(|id| {
            theme_data.id_to_uri.get(id).map(|uri| {
                format!(
                    r#"<image id="{id}" width="{IMG_WIDTH}" height="{IMG_HEIGHT}" href="{uri}"/>"#
                )
            })
        })
        .collect::<Vec<_>>()
        .join("");

    SVG_TEMPLATE
        .replace("{width}", &(PAD_LENGTH as u32 * IMG_WIDTH).to_string())
        .replace("{height}", &IMG_HEIGHT.to_string())
        .replace("{defs}", &defs)
        .replace("{uses}", &uses)
}

#[derive(serde::Deserialize)]
struct GetQuery {
    #[serde(default)]
    theme: Option<String>,
}

#[get("/get/{name}")]
async fn get_image(
    path: web::Path<String>,
    query: web::Query<GetQuery>,
    state: web::Data<AppState>,
) -> impl Responder {
    let name = path.into_inner();
    debug!("GET /get/{}?theme={:?}", name, query.theme);

    let count = increment(&state.db, &name).await;

    let theme_key = query.theme.as_deref().unwrap_or(&state.default_theme);
    let theme_data = state
        .themes
        .get(theme_key)
        .or_else(|| state.themes.get(&*state.default_theme))
        .or_else(|| state.themes.values().next());

    let Some(td) = theme_data else {
        error!("no themes available");
        return HttpResponse::InternalServerError().body("no themes");
    };

    HttpResponse::Ok()
        .content_type("image/svg+xml")
        .insert_header(("Cache-Control", "no-cache, no-store, must-revalidate"))
        .body(render_svg(td, count))
}

#[derive(serde::Deserialize)]
struct SetQuery {
    count: i64,
}

#[get("/api/set/{name}")]
async fn set_count(
    req: HttpRequest,
    path: web::Path<String>,
    state: web::Data<AppState>,
    query: web::Query<SetQuery>,
) -> impl Responder {
    let name = path.into_inner();
    let count = query.count;

    let expected_token = match &state.api_auth_token {
        Some(t) => t,
        None => return HttpResponse::Unauthorized().body("Missing token configuration"),
    };

    let token = match req
        .headers()
        .get("X-Auth-Token")
        .and_then(|v| v.to_str().ok())
    {
        Some(t) => t,
        None => return HttpResponse::Unauthorized().body("Missing token"),
    };

    if token != expected_token {
        return HttpResponse::Unauthorized().body("Invalid token");
    }

    debug!("SET /api/set/{} count={}", name, count);

    if count < 0 {
        warn!("rejected negative count: {} for {}", count, name);
        return HttpResponse::BadRequest().body("count must be non-negative");
    }

    sqlx::query(
        "INSERT INTO tb_count (name, num) VALUES (?1, ?2)
         ON CONFLICT(name) DO UPDATE SET num = ?2",
    )
    .bind(&name)
    .bind(count)
    .execute(&state.db)
    .await
    .ok();

    info!("set {} to {}", name, count);
    HttpResponse::Ok().body(format!("set count of '{}' to {}", name, count))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let args = Args::parse();
    info!("starting yamcrs server");
    info!("bind: {}:{}", args.host, args.port);
    info!("db: {}", args.db_path);
    info!("assets: {}", args.assets_path);
    info!("default theme: {}", args.default_theme);

    let db = init_db(&args.db_path).await;
    let themes = Arc::new(load_themes(&args.assets_path));

    let host = args.host.clone();
    let port = args.port;

    let state = web::Data::new(AppState {
        db,
        themes,
        default_theme: args.default_theme,
        api_auth_token: args.api_auth_token,
    });

    info!("listening on http://{}:{}", host, port);

    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .service(get_image)
            .service(set_count)
    })
    .bind((host.clone(), port))?
    .run()
    .await
}
