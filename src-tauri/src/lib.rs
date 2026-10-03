use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{mpsc, Mutex},
    thread,
    time::{Duration as StdDuration, Instant},
};

use arboard::Clipboard;
use chrono::{DateTime, Duration, Local, LocalResult, TimeZone, Timelike, Utc};
use fsrs::{FSRS, MemoryState};
use keyring::Entry;
use reqwest::Client;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, RunEvent, State,
};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::ShortcutState;
use tauri_plugin_notification::NotificationExt;

const SERVICE_NAME: &str = "English Pet";
const SECRET_NAME: &str = "9router-api-key";
const DEFAULT_ENDPOINT: &str = "http://localhost:20128/v1";
const DEFAULT_MODEL: &str = "";
const HOTKEY: &str = "CommandOrControl+Shift+E";
const REMINDER_COOLDOWN: StdDuration = StdDuration::from_secs(60 * 60);
const NO_CARD_CHECK: StdDuration = StdDuration::from_secs(24 * 60 * 60);

struct AppState {
    db: Mutex<Connection>,
    http: Client,
    reminder_tx: mpsc::Sender<()>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard {
    due_count: i64,
    total_words: i64,
    next_due_at: Option<String>,
    pet_passthrough: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapturedWord {
    id: i64,
    term: String,
    translation: Option<String>,
    created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewCard {
    id: i64,
    term: String,
    translation: Option<String>,
    example: Option<String>,
    due_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPayload {
    endpoint: String,
    model: String,
    desired_retention: f32,
    quiet_hours_start: u8,
    quiet_hours_end: u8,
    pet_passthrough: bool,
    ai_enabled: bool,
    autostart: bool,
    has_api_key: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveWordInput {
    term: String,
    translation: Option<String>,
    example: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSettingsInput {
    endpoint: String,
    model: String,
    desired_retention: f32,
    quiet_hours_start: u8,
    quiet_hours_end: u8,
    pet_passthrough: bool,
    ai_enabled: bool,
    autostart: bool,
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateInput {
    term: String,
    translation: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeneratedContent {
    translation: String,
    example: String,
    exercise: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportVocabulary {
    id: i64,
    term: String,
    normalized_term: String,
    translation: Option<String>,
    example: Option<String>,
    stability: Option<f32>,
    difficulty: Option<f32>,
    due_at: String,
    last_reviewed_at: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportReviewLog {
    vocabulary_id: i64,
    rating: u8,
    elapsed_days: i64,
    scheduled_days: i64,
    reviewed_at: String,
    state_before_json: Option<String>,
    state_after_json: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportData {
    version: u8,
    exported_at: String,
    vocabulary: Vec<ExportVocabulary>,
    review_logs: Vec<ExportReviewLog>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportResult {
    imported_words: usize,
    skipped_words: usize,
    imported_reviews: usize,
}

fn app_db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|error| error.to_string())?;
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir.join("english-pet.db"))
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn normalize_term(raw: &str) -> Result<(String, String), String> {
    let term = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if term.is_empty() {
        return Err("Nhập từ hoặc cụm từ trước khi lưu.".into());
    }
    if term.chars().count() > 100 {
        return Err("Từ/cụm từ tối đa 100 ký tự.".into());
    }
    if term.lines().count() > 1 {
        return Err("Chỉ lưu một từ hoặc cụm từ ngắn.".into());
    }
    Ok((term.to_lowercase(), term))
}

fn initialize_db(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            PRAGMA foreign_keys = ON;
            PRAGMA journal_mode = WAL;
            PRAGMA busy_timeout = 5000;
            CREATE TABLE IF NOT EXISTS settings (
              key TEXT PRIMARY KEY NOT NULL,
              value TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS vocabulary (
              id INTEGER PRIMARY KEY,
              term TEXT NOT NULL,
              normalized_term TEXT NOT NULL UNIQUE,
              translation TEXT,
              example TEXT,
              stability REAL,
              difficulty REAL,
              due_at TEXT NOT NULL,
              last_reviewed_at TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS review_logs (
              id INTEGER PRIMARY KEY,
              vocabulary_id INTEGER NOT NULL REFERENCES vocabulary(id) ON DELETE CASCADE,
              rating INTEGER NOT NULL CHECK(rating BETWEEN 1 AND 4),
              elapsed_days INTEGER NOT NULL,
              scheduled_days INTEGER NOT NULL,
              reviewed_at TEXT NOT NULL,
              state_before_json TEXT,
              state_after_json TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_vocabulary_due ON vocabulary(due_at);
            CREATE INDEX IF NOT EXISTS idx_review_logs_word_time ON review_logs(vocabulary_id, reviewed_at);
            ",
        )
        .map_err(|error| error.to_string())?;

    for (key, value) in [
        ("endpoint", DEFAULT_ENDPOINT),
        ("model", DEFAULT_MODEL),
        ("desired_retention", "0.9"),
        ("quiet_hours_start", "22"),
        ("quiet_hours_end", "8"),
        ("pet_passthrough", "false"),
        ("ai_enabled", "true"),
        ("reminder_snoozed_until", ""),
    ] {
        connection
            .execute(
                "INSERT OR IGNORE INTO settings(key, value, updated_at) VALUES (?1, ?2, ?3)",
                params![key, value, now()],
            )
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn setting(connection: &Connection, key: &str) -> Result<String, String> {
    connection
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| row.get(0))
        .map_err(|error| error.to_string())
}

fn upsert_setting(connection: &Connection, key: &str, value: &str) -> Result<(), String> {
    connection
        .execute(
            "INSERT INTO settings(key, value, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, now()],
        )
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn has_api_key() -> bool {
    Entry::new(SERVICE_NAME, SECRET_NAME)
        .ok()
        .and_then(|entry| entry.get_password().ok())
        .is_some_and(|key| !key.trim().is_empty())
}

fn api_key() -> Result<Option<String>, String> {
    let entry = Entry::new(SERVICE_NAME, SECRET_NAME).map_err(|error| error.to_string())?;
    match entry.get_password() {
        Ok(key) if !key.trim().is_empty() => Ok(Some(key)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn parse_due(date: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(date)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| error.to_string())
}

fn is_quiet_hour(hour: u32, start: u8, end: u8) -> bool {
    if start == end {
        return false;
    }
    if start < end {
        hour >= u32::from(start) && hour < u32::from(end)
    } else {
        hour >= u32::from(start) || hour < u32::from(end)
    }
}

fn until_quiet_ends(now: DateTime<Local>, end: u8) -> StdDuration {
    let now_minutes = i64::from(now.hour() * 60 + now.minute());
    let end_minutes = i64::from(end) * 60;
    let seconds = ((end_minutes - now_minutes).rem_euclid(24 * 60) * 60 - i64::from(now.second())).max(1);
    StdDuration::from_secs(seconds as u64)
}

enum ReminderAction {
    Notify(i64),
    Wait(StdDuration),
}

fn reminder_action(state: &AppState, last_notification: Option<Instant>) -> Result<ReminderAction, String> {
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    if let Ok(snoozed_until) = setting(&connection, "reminder_snoozed_until") {
        if let Ok(snoozed_until) = parse_due(&snoozed_until) {
            let current = Utc::now();
            if snoozed_until > current {
                return Ok(ReminderAction::Wait(
                    snoozed_until.signed_duration_since(current).to_std().unwrap_or(StdDuration::ZERO),
                ));
            }
            upsert_setting(&connection, "reminder_snoozed_until", "")?;
        }
    }
    let next_due_at: Option<String> = connection
        .query_row("SELECT due_at FROM vocabulary ORDER BY due_at LIMIT 1", [], |row| row.get(0))
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(next_due_at) = next_due_at else {
        return Ok(ReminderAction::Wait(NO_CARD_CHECK));
    };
    let next_due = parse_due(&next_due_at)?;
    let current = Utc::now();
    if next_due > current {
        return Ok(ReminderAction::Wait(
            next_due.signed_duration_since(current).to_std().unwrap_or(StdDuration::ZERO),
        ));
    }

    let quiet_start = setting(&connection, "quiet_hours_start")?.parse().unwrap_or(22);
    let quiet_end = setting(&connection, "quiet_hours_end")?.parse().unwrap_or(8);
    if is_quiet_hour(Local::now().hour(), quiet_start, quiet_end) {
        return Ok(ReminderAction::Wait(until_quiet_ends(Local::now(), quiet_end)));
    }
    if let Some(last) = last_notification {
        let elapsed = last.elapsed();
        if elapsed < REMINDER_COOLDOWN {
            return Ok(ReminderAction::Wait(REMINDER_COOLDOWN - elapsed));
        }
    }
    let due_count = connection
        .query_row("SELECT COUNT(*) FROM vocabulary WHERE due_at <= ?1", [now()], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    Ok(ReminderAction::Notify(due_count))
}

fn start_reminder_scheduler(app: AppHandle, receiver: mpsc::Receiver<()>) {
    thread::spawn(move || {
        let mut last_notification = None;
        loop {
            let action = reminder_action(app.state::<AppState>().inner(), last_notification)
                .unwrap_or(ReminderAction::Wait(StdDuration::from_secs(60 * 60)));
            match action {
                ReminderAction::Wait(delay) => {
                    if receiver.recv_timeout(delay).is_ok() {
                        continue;
                    }
                }
                ReminderAction::Notify(due_count) => {
                    let _ = app.emit("review-reminder", due_count);
                    let pet_is_visible = app
                        .get_webview_window("main")
                        .and_then(|window| window.is_visible().ok())
                        .unwrap_or(false);
                    if !pet_is_visible {
                        let _ = app.notification().builder()
                            .title("English Pet có từ đến hạn")
                            .body(format!("Có {due_count} từ chờ ôn. Mở pet để ôn 1 phút."))
                            .show();
                    }
                    last_notification = Some(Instant::now());
                }
            }
        }
    });
}

fn reschedule_reminder(state: &AppState) {
    let _ = state.reminder_tx.send(());
}

#[tauri::command]
fn dashboard(state: State<'_, AppState>) -> Result<Dashboard, String> {
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    let current_time = now();
    let due_count = connection
        .query_row("SELECT COUNT(*) FROM vocabulary WHERE due_at <= ?1", [current_time], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let total_words = connection
        .query_row("SELECT COUNT(*) FROM vocabulary", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let next_due_at = connection
        .query_row("SELECT due_at FROM vocabulary ORDER BY due_at LIMIT 1", [], |row| row.get(0))
        .optional()
        .map_err(|error| error.to_string())?;
    let pet_passthrough = setting(&connection, "pet_passthrough")? == "true";
    Ok(Dashboard { due_count, total_words, next_due_at, pet_passthrough })
}

#[tauri::command]
fn get_settings(app: AppHandle, state: State<'_, AppState>) -> Result<SettingsPayload, String> {
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    Ok(SettingsPayload {
        endpoint: setting(&connection, "endpoint")?,
        model: setting(&connection, "model")?,
        desired_retention: setting(&connection, "desired_retention")?.parse().unwrap_or(0.9),
        quiet_hours_start: setting(&connection, "quiet_hours_start")?.parse().unwrap_or(22),
        quiet_hours_end: setting(&connection, "quiet_hours_end")?.parse().unwrap_or(8),
        pet_passthrough: setting(&connection, "pet_passthrough")? == "true",
        ai_enabled: setting(&connection, "ai_enabled")? == "true",
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        has_api_key: has_api_key(),
    })
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<'_, AppState>, input: SaveSettingsInput) -> Result<(), String> {
    let endpoint = input.endpoint.trim().trim_end_matches('/').to_string();
    if !endpoint.starts_with("http://") && !endpoint.starts_with("https://") {
        return Err("9Router endpoint phải bắt đầu bằng http:// hoặc https://.".into());
    }
    if !endpoint.ends_with("/v1") {
        return Err("9Router endpoint phải kết thúc bằng /v1.".into());
    }
    if !(0.80..=0.98).contains(&input.desired_retention) {
        return Err("Desired retention phải nằm trong 0.80 đến 0.98.".into());
    }
    if input.quiet_hours_start > 23 || input.quiet_hours_end > 23 {
        return Err("Giờ yên lặng không hợp lệ.".into());
    }

    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    upsert_setting(&connection, "endpoint", &endpoint)?;
    upsert_setting(&connection, "model", input.model.trim())?;
    upsert_setting(&connection, "desired_retention", &input.desired_retention.to_string())?;
    upsert_setting(&connection, "quiet_hours_start", &input.quiet_hours_start.to_string())?;
    upsert_setting(&connection, "quiet_hours_end", &input.quiet_hours_end.to_string())?;
    upsert_setting(&connection, "pet_passthrough", &input.pet_passthrough.to_string())?;
    upsert_setting(&connection, "ai_enabled", &input.ai_enabled.to_string())?;
    drop(connection);

    if let Some(key) = input.api_key.map(|key| key.trim().to_string()) {
        let entry = Entry::new(SERVICE_NAME, SECRET_NAME).map_err(|error| error.to_string())?;
        if key.is_empty() {
            let _ = entry.delete_credential();
        } else {
            entry.set_password(&key).map_err(|error| error.to_string())?;
        }
    }
    let autostart = app.autolaunch();
    if input.autostart && !autostart.is_enabled().unwrap_or(false) {
        autostart.enable().map_err(|error| error.to_string())?;
    } else if !input.autostart && autostart.is_enabled().unwrap_or(false) {
        autostart.disable().map_err(|error| error.to_string())?;
    }
    set_passthrough(&app, input.pet_passthrough)?;
    reschedule_reminder(&state);
    Ok(())
}

#[tauri::command]
fn capture_clipboard() -> Result<String, String> {
    let mut clipboard = Clipboard::new().map_err(|error| format!("Không mở được clipboard: {error}"))?;
    let text = clipboard.get_text().map_err(|_| "Clipboard không có text. Hãy copy từ trước.".to_string())?;
    let (_, term) = normalize_term(&text)?;
    Ok(term)
}

#[tauri::command]
fn save_word(state: State<'_, AppState>, input: SaveWordInput) -> Result<CapturedWord, String> {
    let (normalized_term, term) = normalize_term(&input.term)?;
    let translation = input.translation.filter(|value| !value.trim().is_empty()).map(|value| value.trim().to_string());
    let example = input.example.filter(|value| !value.trim().is_empty()).map(|value| value.trim().to_string());
    let timestamp = now();
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    let transaction = connection.unchecked_transaction().map_err(|error| error.to_string())?;
    transaction.execute(
        "INSERT INTO vocabulary(term, normalized_term, translation, example, due_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?5)",
        params![term, normalized_term, translation, example, timestamp],
    ).map_err(|error| {
        if error.to_string().contains("UNIQUE") { "Từ này đã có trong Inbox.".to_string() } else { error.to_string() }
    })?;
    let id = transaction.last_insert_rowid();
    transaction.commit().map_err(|error| error.to_string())?;
    reschedule_reminder(&state);
    Ok(CapturedWord { id, term, translation, created_at: timestamp })
}

fn backup_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().document_dir().map_err(|error| error.to_string())?.join("English Pet Backups");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

fn backup_filename(extension: &str) -> String {
    format!("english-pet-{}.{}", Utc::now().format("%Y%m%d-%H%M%S"), extension)
}

fn export_from_connection(connection: &Connection) -> Result<ExportData, String> {
    let mut vocabulary_query = connection.prepare(
        "SELECT id, term, normalized_term, translation, example, stability, difficulty, due_at, last_reviewed_at, created_at, updated_at
         FROM vocabulary ORDER BY id",
    ).map_err(|error| error.to_string())?;
    let vocabulary = vocabulary_query.query_map([], |row| Ok(ExportVocabulary {
        id: row.get(0)?, term: row.get(1)?, normalized_term: row.get(2)?, translation: row.get(3)?, example: row.get(4)?,
        stability: row.get(5)?, difficulty: row.get(6)?, due_at: row.get(7)?, last_reviewed_at: row.get(8)?,
        created_at: row.get(9)?, updated_at: row.get(10)?,
    })).map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    let mut review_query = connection.prepare(
        "SELECT vocabulary_id, rating, elapsed_days, scheduled_days, reviewed_at, state_before_json, state_after_json
         FROM review_logs ORDER BY id",
    ).map_err(|error| error.to_string())?;
    let review_logs = review_query.query_map([], |row| Ok(ExportReviewLog {
        vocabulary_id: row.get(0)?, rating: row.get(1)?, elapsed_days: row.get(2)?, scheduled_days: row.get(3)?,
        reviewed_at: row.get(4)?, state_before_json: row.get(5)?, state_after_json: row.get(6)?,
    })).map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
    Ok(ExportData { version: 1, exported_at: now(), vocabulary, review_logs })
}

#[tauri::command]
fn export_data(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let export = {
        let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
        export_from_connection(&connection)?
    };
    let path = backup_dir(&app)?.join(backup_filename("json"));
    let contents = serde_json::to_string_pretty(&export).map_err(|error| error.to_string())?;
    fs::write(&path, contents).map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn backup_sqlite(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let path = backup_dir(&app)?.join(backup_filename("db"));
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    connection.backup(rusqlite::MAIN_DB, &path, None).map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}

fn validate_import_word(word: &ExportVocabulary) -> Result<(String, String), String> {
    let (normalized_term, term) = normalize_term(&word.term)?;
    if word.translation.as_deref().is_some_and(|text| text.chars().count() > 2_000)
        || word.example.as_deref().is_some_and(|text| text.chars().count() > 5_000) {
        return Err("Import chứa nghĩa hoặc ví dụ quá dài.".into());
    }
    parse_due(&word.due_at)?;
    parse_due(&word.created_at)?;
    parse_due(&word.updated_at)?;
    if let Some(reviewed_at) = &word.last_reviewed_at { parse_due(reviewed_at)?; }
    Ok((normalized_term, term))
}

fn import_into_connection(connection: &Connection, data: ExportData) -> Result<ImportResult, String> {
    if data.version != 1 {
        return Err("File backup không được hỗ trợ. Cần English Pet export version 1.".into());
    }
    if data.vocabulary.len() > 100_000 || data.review_logs.len() > 1_000_000 {
        return Err("File backup quá lớn.".into());
    }
    let transaction = connection.unchecked_transaction().map_err(|error| error.to_string())?;
    let mut imported_words = 0;
    let mut skipped_words = 0;
    let mut imported_reviews = 0;
    let mut ids = HashMap::new();
    for word in data.vocabulary {
        let original_id = word.id;
        let (normalized_term, term) = validate_import_word(&word)?;
        let duplicate: Option<i64> = transaction.query_row(
            "SELECT id FROM vocabulary WHERE normalized_term = ?1", [&normalized_term], |row| row.get(0),
        ).optional().map_err(|error| error.to_string())?;
        if duplicate.is_some() {
            skipped_words += 1;
            continue;
        }
        transaction.execute(
            "INSERT INTO vocabulary(term, normalized_term, translation, example, stability, difficulty, due_at, last_reviewed_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![term, normalized_term, word.translation, word.example, word.stability, word.difficulty,
                word.due_at, word.last_reviewed_at, word.created_at, word.updated_at],
        ).map_err(|error| error.to_string())?;
        if ids.insert(original_id, transaction.last_insert_rowid()).is_some() {
            return Err("File backup có ID từ vựng trùng lặp.".into());
        }
        imported_words += 1;
    }
    for log in data.review_logs {
        let Some(&vocabulary_id) = ids.get(&log.vocabulary_id) else { continue; };
        if !(1..=4).contains(&log.rating) || log.elapsed_days < 0 || log.scheduled_days < 0 {
            return Err("File backup có review log không hợp lệ.".into());
        }
        parse_due(&log.reviewed_at)?;
        serde_json::from_str::<Value>(&log.state_after_json).map_err(|_| "File backup có FSRS state lỗi.".to_string())?;
        if let Some(state_before) = &log.state_before_json {
            serde_json::from_str::<Value>(state_before).map_err(|_| "File backup có FSRS state lỗi.".to_string())?;
        }
        transaction.execute(
            "INSERT INTO review_logs(vocabulary_id, rating, elapsed_days, scheduled_days, reviewed_at, state_before_json, state_after_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![vocabulary_id, log.rating, log.elapsed_days, log.scheduled_days, log.reviewed_at,
                log.state_before_json, log.state_after_json],
        ).map_err(|error| error.to_string())?;
        imported_reviews += 1;
    }
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(ImportResult { imported_words, skipped_words, imported_reviews })
}

#[tauri::command]
fn import_data(state: State<'_, AppState>, contents: String) -> Result<ImportResult, String> {
    if contents.len() > 50 * 1024 * 1024 {
        return Err("File backup tối đa 50 MB.".into());
    }
    let data: ExportData = serde_json::from_str(&contents)
        .map_err(|_| "File backup không phải JSON hợp lệ của English Pet.".to_string())?;
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    let result = import_into_connection(&connection, data)?;
    drop(connection);
    reschedule_reminder(&state);
    Ok(result)
}

fn tomorrow_morning() -> Result<DateTime<Utc>, String> {
    let tomorrow = Local::now().date_naive().succ_opt().ok_or("Không tính được ngày mai.")?;
    let local_time = tomorrow.and_hms_opt(8, 0, 0).ok_or("Không tính được giờ nhắc.")?;
    match Local.from_local_datetime(&local_time) {
        LocalResult::Single(time) | LocalResult::Ambiguous(time, _) => Ok(time.with_timezone(&Utc)),
        LocalResult::None => Ok((Utc::now() + Duration::hours(24)).with_timezone(&Utc)),
    }
}

#[tauri::command]
fn snooze_reminder(state: State<'_, AppState>, duration: String) -> Result<String, String> {
    let until = match duration.as_str() {
        "hour" => Utc::now() + Duration::hours(1),
        "tomorrow" => tomorrow_morning()?,
        _ => return Err("Kiểu hoãn không hợp lệ.".into()),
    }.to_rfc3339();
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    upsert_setting(&connection, "reminder_snoozed_until", &until)?;
    drop(connection);
    reschedule_reminder(&state);
    Ok(until)
}

#[tauri::command]
fn next_review(state: State<'_, AppState>) -> Result<Option<ReviewCard>, String> {
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    connection
        .query_row(
            "SELECT id, term, translation, example, due_at FROM vocabulary WHERE due_at <= ?1 ORDER BY due_at, id LIMIT 1",
            [now()],
            |row| Ok(ReviewCard {
                id: row.get(0)?, term: row.get(1)?, translation: row.get(2)?, example: row.get(3)?, due_at: row.get(4)?,
            }),
        )
        .optional()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn rate_review(state: State<'_, AppState>, card_id: i64, rating: u8) -> Result<String, String> {
    if !(1..=4).contains(&rating) {
        return Err("Rating không hợp lệ.".into());
    }
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    let transaction = connection.unchecked_transaction().map_err(|error| error.to_string())?;
    let row: (Option<f32>, Option<f32>, Option<String>) = transaction.query_row(
        "SELECT stability, difficulty, last_reviewed_at FROM vocabulary WHERE id = ?1", [card_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).map_err(|_| "Không tìm thấy thẻ.".to_string())?;
    let previous = match (row.0, row.1) {
        (Some(stability), Some(difficulty)) => Some(MemoryState { stability, difficulty }),
        _ => None,
    };
    let elapsed_days = row.2.as_deref().and_then(|value| parse_due(value).ok())
        .map(|value| (Utc::now() - value).num_days().max(0) as u32).unwrap_or(0);
    let desired_retention: f32 = setting(&transaction, "desired_retention")?.parse().unwrap_or(0.9);
    let states = FSRS::default().next_states(previous, desired_retention, elapsed_days).map_err(|error| error.to_string())?;
    let next = match rating { 1 => states.again, 2 => states.hard, 3 => states.good, 4 => states.easy, _ => unreachable!() };
    let scheduled_days = next.interval.round().max(1.0) as i64;
    let reviewed_at = now();
    let due_at = (Utc::now() + Duration::days(scheduled_days)).to_rfc3339();
    let before = previous.map(|state| json!({"stability": state.stability, "difficulty": state.difficulty}).to_string());
    let after = json!({"stability": next.memory.stability, "difficulty": next.memory.difficulty}).to_string();
    transaction.execute(
        "INSERT INTO review_logs(vocabulary_id, rating, elapsed_days, scheduled_days, reviewed_at, state_before_json, state_after_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![card_id, rating, elapsed_days, scheduled_days, reviewed_at, before, after],
    ).map_err(|error| error.to_string())?;
    transaction.execute(
        "UPDATE vocabulary SET stability = ?2, difficulty = ?3, due_at = ?4, last_reviewed_at = ?5, updated_at = ?5 WHERE id = ?1",
        params![card_id, next.memory.stability, next.memory.difficulty, due_at, reviewed_at],
    ).map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    reschedule_reminder(&state);
    Ok(format!("Ôn lại sau {scheduled_days} ngày"))
}

#[tauri::command]
async fn ai_models(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let (endpoint, enabled) = {
        let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
        (setting(&connection, "endpoint")?, setting(&connection, "ai_enabled")? == "true")
    };
    if !enabled { return Ok(Vec::new()); }
    let mut request = state.http.get(format!("{endpoint}/models"));
    if let Some(key) = api_key()? { request = request.bearer_auth(key); }
    let body: Value = request.send().await.map_err(|error| format!("Không kết nối được 9Router: {error}"))?
        .error_for_status().map_err(|error| format!("9Router lỗi: {error}"))?
        .json().await.map_err(|error| format!("9Router trả response lỗi: {error}"))?;
    Ok(body["data"].as_array().into_iter().flatten().filter_map(|item| item["id"].as_str()).map(str::to_owned).collect())
}

#[tauri::command]
async fn generate_content(state: State<'_, AppState>, input: GenerateInput) -> Result<GeneratedContent, String> {
    let (_, term) = normalize_term(&input.term)?;
    let (endpoint, model, enabled) = {
        let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
        (setting(&connection, "endpoint")?, setting(&connection, "model")?, setting(&connection, "ai_enabled")? == "true")
    };
    if !enabled { return Err("AI đang tắt trong Settings.".into()); }
    if model.is_empty() { return Err("Chọn model 9Router trong Settings trước.".into()); }
    let known_translation = input.translation.filter(|value| !value.trim().is_empty()).map(|value| value.trim().to_owned());
    let prompt = format!("You help a Vietnamese learner learn English. For the term {term:?}{} return only one JSON object with keys translation, example, exercise. translation: concise Vietnamese meaning. example: natural English sentence at B1 level. exercise: one short cloze question. No markdown.", known_translation.map(|value| format!(" (known Vietnamese meaning: {value:?})")).unwrap_or_default());
    let mut request = state.http.post(format!("{endpoint}/chat/completions")).json(&json!({
        "model": model,
        "stream": false,
        "response_format": {"type": "json_object"},
        "messages": [{"role": "user", "content": prompt}],
        "max_tokens": 300
    }));
    if let Some(key) = api_key()? { request = request.bearer_auth(key); }
    let response: Value = request.send().await.map_err(|error| format!("Không kết nối được 9Router: {error}"))?
        .error_for_status().map_err(|error| format!("9Router lỗi: {error}"))?
        .json().await.map_err(|error| format!("9Router trả response lỗi: {error}"))?;
    let content = response["choices"][0]["message"]["content"].as_str().ok_or("9Router không trả nội dung.")?;
    let generated: GeneratedContent = serde_json::from_str(content).map_err(|_| "Model trả JSON không hợp lệ. Hãy thử model khác hoặc nhập tay.".to_string())?;
    if generated.translation.trim().is_empty() || generated.example.trim().is_empty() || generated.exercise.trim().is_empty() {
        return Err("Model trả nội dung thiếu trường bắt buộc.".into());
    }
    Ok(generated)
}

#[tauri::command]
fn toggle_passthrough(app: AppHandle, state: State<'_, AppState>) -> Result<bool, String> {
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    let next = setting(&connection, "pet_passthrough")? != "true";
    upsert_setting(&connection, "pet_passthrough", &next.to_string())?;
    drop(connection);
    set_passthrough(&app, next)?;
    Ok(next)
}

fn set_passthrough(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("Pet window unavailable")?;
    window.set_ignore_cursor_events(enabled).map_err(|error| error.to_string())
}

fn show_pet(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_ignore_cursor_events(false);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let review = MenuItem::with_id(app, "review", "Ôn từ đến hạn", true, None::<&str>)?;
    let capture = MenuItem::with_id(app, "capture", "Thêm từ từ clipboard", true, Some(HOTKEY))?;
    let passthrough = MenuItem::with_id(app, "passthrough", "Bật/tắt click xuyên pet", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Thoát", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&review, &capture, &passthrough, &quit])?;
    TrayIconBuilder::with_id("tray")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .tooltip("English Pet")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "review" => { let _ = app.emit("open-review", ()); show_pet(app); }
            "capture" => { let _ = app.emit("capture-clipboard", ()); show_pet(app); }
            "passthrough" => { if let Ok(state) = app.try_state::<AppState>().ok_or(()) { let _ = toggle_passthrough(app.clone(), state); } }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcut(HOTKEY).expect("valid hotkey")
                .with_handler(|app, _, event| {
                    if event.state == ShortcutState::Pressed {
                        let _ = app.emit("capture-clipboard", ());
                        show_pet(app);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            let db_path = app_db_path(app.handle()).map_err(|error| Box::<dyn std::error::Error>::from(error))?;
            let connection = Connection::open(db_path)?;
            initialize_db(&connection).map_err(|error| Box::<dyn std::error::Error>::from(error))?;
            let (reminder_tx, reminder_rx) = mpsc::channel();
            app.manage(AppState {
                db: Mutex::new(connection),
                http: Client::builder().timeout(StdDuration::from_secs(20)).build()?,
                reminder_tx,
            });
            setup_tray(app.handle())?;
            let passthrough = app.state::<AppState>().db.lock().ok().and_then(|db| setting(&db, "pet_passthrough").ok()).is_some_and(|value| value == "true");
            if passthrough { let _ = set_passthrough(app.handle(), true); }
            start_reminder_scheduler(app.handle().clone(), reminder_rx);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![dashboard, get_settings, save_settings, capture_clipboard, save_word, export_data, backup_sqlite, import_data, snooze_reminder, next_review, rate_review, ai_models, generate_content, toggle_passthrough])
        .build(context)
        .expect("error while building Tauri application")
        .run(|app, event| {
            if let RunEvent::ExitRequested { .. } = event { app.exit(0); }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_one_short_phrase() {
        assert_eq!(normalize_term("  take\tcare  ").unwrap(), ("take care".into(), "take care".into()));
        assert!(normalize_term("\n").is_err());
    }

    #[test]
    fn fsrs_schedules_forward() {
        let next = FSRS::default().next_states(None, 0.9, 0).unwrap().good;
        assert!(next.interval >= 1.0);
        assert!(next.memory.stability > 0.0);
    }

    #[test]
    fn quiet_hours_handle_overnight_and_daytime_ranges() {
        assert!(is_quiet_hour(23, 22, 8));
        assert!(is_quiet_hour(7, 22, 8));
        assert!(!is_quiet_hour(8, 22, 8));
        assert!(is_quiet_hour(12, 9, 17));
        assert!(!is_quiet_hour(18, 9, 17));
        assert!(!is_quiet_hour(12, 9, 9));
    }

    #[test]
    fn json_export_import_keeps_word_and_review_state() {
        let source = Connection::open_in_memory().unwrap();
        initialize_db(&source).unwrap();
        source.execute(
            "INSERT INTO vocabulary(term, normalized_term, translation, example, stability, difficulty, due_at, last_reviewed_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params!["take care", "take care", "chăm sóc", "Take care.", 2.5_f32, 4.0_f32,
                "2026-10-04T00:00:00+00:00", "2026-10-03T00:00:00+00:00", "2026-10-01T00:00:00+00:00", "2026-10-03T00:00:00+00:00"],
        ).unwrap();
        source.execute(
            "INSERT INTO review_logs(vocabulary_id, rating, elapsed_days, scheduled_days, reviewed_at, state_before_json, state_after_json)
             VALUES (1, 3, 1, 2, ?1, ?2, ?3)",
            params!["2026-10-03T00:00:00+00:00", r#"{"stability":1.0,"difficulty":5.0}"#, r#"{"stability":2.5,"difficulty":4.0}"#],
        ).unwrap();

        let destination = Connection::open_in_memory().unwrap();
        initialize_db(&destination).unwrap();
        let result = import_into_connection(&destination, export_from_connection(&source).unwrap()).unwrap();

        assert_eq!(result.imported_words, 1);
        assert_eq!(result.imported_reviews, 1);
        assert_eq!(destination.query_row("SELECT COUNT(*) FROM vocabulary", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        assert_eq!(destination.query_row("SELECT COUNT(*) FROM review_logs", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn import_rejects_unknown_version_without_writing() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_db(&connection).unwrap();
        let result = import_into_connection(&connection, ExportData {
            version: 2,
            exported_at: now(),
            vocabulary: Vec::new(),
            review_logs: Vec::new(),
        });
        assert!(result.is_err());
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM vocabulary", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }
}
