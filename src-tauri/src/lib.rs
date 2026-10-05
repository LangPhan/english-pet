use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Mutex, mpsc},
    thread,
    time::{Duration as StdDuration, Instant},
};

use arboard::Clipboard;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Duration, Local, LocalResult, TimeZone, Timelike, Utc};
use fsrs::{FSRS, MemoryState};
use keyring::Entry;
use reqwest::Client;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, RunEvent, State,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_autostart::ManagerExt as AutostartExt;
use tauri_plugin_global_shortcut::ShortcutState;
use tauri_plugin_notification::NotificationExt;

#[cfg(target_os = "windows")]
const VK_CONTROL: i32 = 0x11;
#[cfg(target_os = "windows")]
const VK_SHIFT: i32 = 0x10;
#[cfg(target_os = "windows")]
const VK_C: u16 = 0x43;
#[cfg(target_os = "windows")]
const KEYEVENTF_KEYUP: u32 = 0x0002;

#[cfg(target_os = "windows")]
#[repr(C)]
struct WindowsInput {
    input_type: u32,
    data: WindowsInputData,
}

#[cfg(target_os = "windows")]
#[repr(C)]
union WindowsInputData {
    keyboard: WindowsKeyboardInput,
    mouse_layout: [u64; 4],
}

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy)]
struct WindowsKeyboardInput {
    virtual_key: u16,
    scan_code: u16,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
unsafe extern "system" {
    fn GetAsyncKeyState(virtual_key: i32) -> i16;
    fn GetClipboardSequenceNumber() -> u32;
    fn SendInput(input_count: u32, inputs: *const WindowsInput, input_size: i32) -> u32;
}

const SERVICE_NAME: &str = "English Pet";
const SECRET_NAME: &str = "9router-api-key";
const DEFAULT_ENDPOINT: &str = "http://localhost:20128/v1";
const DEFAULT_MODEL: &str = "";
const HOTKEY: &str = "CommandOrControl+Shift+E";
const REMINDER_COOLDOWN: StdDuration = StdDuration::from_secs(60 * 60);
const NO_CARD_CHECK: StdDuration = StdDuration::from_secs(24 * 60 * 60);
const WINDOW_MARGIN: i32 = 12;
const TOPICS: &[&str] = &[
    "daily_life",
    "work_tech",
    "study_academic",
    "travel_places",
    "food_dining",
    "health_sports",
    "entertainment",
    "emotions_social",
    "other",
];

struct AppState {
    db: Mutex<Connection>,
    http: Client,
    reminder_tx: mpsc::Sender<()>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RoamStep {
    hit_edge: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard {
    due_count: i64,
    total_words: i64,
    next_due_at: Option<String>,
    due_word: Option<String>,
    pet_passthrough: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CapturedWord {
    id: i64,
    term: String,
    translation: Option<String>,
    topic: Option<String>,
    created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VocabularyItem {
    id: i64,
    term: String,
    ipa: Option<String>,
    translation: Option<String>,
    example: Option<String>,
    topic: Option<String>,
    due_at: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VocabularyPage {
    items: Vec<VocabularyItem>,
    total: i64,
    unclassified_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CategorizeResult {
    updated_count: usize,
    failed_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReviewCard {
    id: i64,
    term: String,
    ipa: Option<String>,
    translation: Option<String>,
    example: Option<String>,
    due_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPayload {
    endpoint: String,
    model: String,
    audio_model: String,
    desired_retention: f32,
    quiet_hours_start: u8,
    quiet_hours_end: u8,
    pet_passthrough: bool,
    pet_scale: f32,
    pet_opacity: f32,
    ai_enabled: bool,
    autostart: bool,
    has_api_key: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveWordInput {
    term: String,
    ipa: Option<String>,
    translation: Option<String>,
    example: Option<String>,
    topic: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListVocabularyInput {
    search: Option<String>,
    topic: Option<String>,
    offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BatchDefineVocabularyInput {
    ids: Vec<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateWordInput {
    id: i64,
    ipa: Option<String>,
    translation: String,
    example: Option<String>,
    topic: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaveSettingsInput {
    endpoint: String,
    model: String,
    audio_model: String,
    desired_retention: f32,
    quiet_hours_start: u8,
    quiet_hours_end: u8,
    pet_passthrough: bool,
    pet_scale: f32,
    pet_opacity: f32,
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
    topic: String,
    ipa: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GeneratedDefinition {
    translation: String,
    example: String,
    topic: String,
    ipa: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluatePronunciationInput {
    card_id: i64,
    audio_base64: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PronunciationSegment {
    part: String,
    status: String,
    note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PronunciationAssessment {
    score: u8,
    is_correct: bool,
    feedback: String,
    recognized_text: String,
    segments: Vec<PronunciationSegment>,
}

#[derive(Debug, Deserialize)]
struct DefinitionAssignments {
    definitions: HashMap<String, GeneratedDefinition>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportVocabulary {
    id: i64,
    term: String,
    normalized_term: String,
    #[serde(default)]
    ipa: Option<String>,
    translation: Option<String>,
    example: Option<String>,
    #[serde(default)]
    topic: Option<String>,
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
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
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

#[cfg(target_os = "windows")]
fn selection_capture() -> Result<String, String> {
    for _ in 0..20 {
        let modifiers_down =
            unsafe { GetAsyncKeyState(VK_CONTROL) < 0 || GetAsyncKeyState(VK_SHIFT) < 0 };
        if !modifiers_down {
            break;
        }
        thread::sleep(StdDuration::from_millis(15));
    }
    let modifiers_down =
        unsafe { GetAsyncKeyState(VK_CONTROL) < 0 || GetAsyncKeyState(VK_SHIFT) < 0 };
    if modifiers_down {
        return Err("Hãy nhả Ctrl và Shift rồi thử lại.".into());
    }

    let before = unsafe { GetClipboardSequenceNumber() };
    let key = |virtual_key, flags| WindowsInput {
        input_type: 1,
        data: WindowsInputData {
            keyboard: WindowsKeyboardInput {
                virtual_key,
                scan_code: 0,
                flags,
                time: 0,
                extra_info: 0,
            },
        },
    };
    let inputs = [
        key(VK_CONTROL as u16, 0),
        key(VK_C, 0),
        key(VK_C, KEYEVENTF_KEYUP),
        key(VK_CONTROL as u16, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<WindowsInput>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        return Err("Không thể copy vùng bôi đen. Hãy thử lại ở ứng dụng đang chọn text.".into());
    }

    for _ in 0..20 {
        thread::sleep(StdDuration::from_millis(15));
        if unsafe { GetClipboardSequenceNumber() } == before {
            continue;
        }
        let mut clipboard =
            Clipboard::new().map_err(|error| format!("Không mở được clipboard: {error}"))?;
        let text = clipboard.get_text().map_err(|_| {
            "Vùng bôi đen không chứa text có thể copy. Hãy chọn từ hoặc cụm từ ngắn rồi thử lại."
                .to_string()
        })?;
        let (_, term) = normalize_term(&text)?;
        return Ok(term);
    }
    Err("Không nhận được text mới từ vùng bôi đen. Hãy chọn từ rồi thử lại.".into())
}

#[cfg(not(target_os = "windows"))]
fn selection_capture() -> Result<String, String> {
    Err("Chụp text đang bôi đen chỉ hỗ trợ Windows.".into())
}

fn normalize_ipa(ipa: Option<String>) -> Result<Option<String>, String> {
    let Some(ipa) = ipa
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if ipa.chars().count() > 200 || ipa.chars().any(char::is_control) {
        return Err("IPA không hợp lệ hoặc quá 200 ký tự.".into());
    }
    Ok(Some(ipa))
}

fn validate_assessment(
    assessment: PronunciationAssessment,
) -> Result<PronunciationAssessment, String> {
    if assessment.feedback.trim().is_empty()
        || assessment.feedback.chars().count() > 2_000
        || assessment.recognized_text.chars().count() > 500
        || assessment.segments.len() > 20
        || assessment.segments.iter().any(|segment| {
            segment.part.trim().is_empty()
                || segment.part.chars().count() > 100
                || !matches!(segment.status.as_str(), "correct" | "needs_improvement")
                || segment
                    .note
                    .as_deref()
                    .is_some_and(|note| note.chars().count() > 500)
        })
    {
        return Err("Model trả chấm phát âm không hợp lệ.".into());
    }
    Ok(assessment)
}

fn validate_topic(topic: Option<String>) -> Result<Option<String>, String> {
    let Some(topic) = topic
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    if TOPICS.contains(&topic.as_str()) {
        Ok(Some(topic))
    } else {
        Err("Chủ đề không hợp lệ.".into())
    }
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
              topic TEXT,
              ipa TEXT,
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

    let has_topic = connection
        .prepare("PRAGMA table_info(vocabulary)")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .try_fold(false, |found, column| Ok(found || column? == "topic"))
        })
        .map_err(|error| error.to_string())?;
    if !has_topic {
        connection
            .execute("ALTER TABLE vocabulary ADD COLUMN topic TEXT", [])
            .map_err(|error| error.to_string())?;
    }
    let has_ipa = connection
        .prepare("PRAGMA table_info(vocabulary)")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .try_fold(false, |found, column| Ok(found || column? == "ipa"))
        })
        .map_err(|error| error.to_string())?;
    if !has_ipa {
        connection
            .execute("ALTER TABLE vocabulary ADD COLUMN ipa TEXT", [])
            .map_err(|error| error.to_string())?;
    }
    connection
        .execute_batch("CREATE INDEX IF NOT EXISTS idx_vocabulary_topic ON vocabulary(topic);")
        .map_err(|error| error.to_string())?;

    for (key, value) in [
        ("endpoint", DEFAULT_ENDPOINT),
        ("model", DEFAULT_MODEL),
        ("audio_model", ""),
        ("desired_retention", "0.9"),
        ("quiet_hours_start", "22"),
        ("quiet_hours_end", "8"),
        ("pet_passthrough", "false"),
        ("pet_size", "medium"),
        ("pet_opacity", "1"),
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
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
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

fn pet_scale(connection: &Connection) -> f32 {
    if let Ok(scale) = setting(connection, "pet_scale")
        .and_then(|value| value.parse::<f32>().map_err(|error| error.to_string()))
    {
        if scale.is_finite() && (0.1..=1.25).contains(&scale) {
            return scale;
        }
    }
    match setting(connection, "pet_size").as_deref() {
        Ok("small") => 0.8,
        Ok("large") => 1.25,
        _ => 1.0,
    }
}

fn legacy_pet_size(scale: f32) -> &'static str {
    if scale <= 0.85 {
        "small"
    } else if scale >= 1.15 {
        "large"
    } else {
        "medium"
    }
}

fn pet_dimensions(scale: f32) -> (f64, f64) {
    // ponytail: keep compact canvas room for menu; shrink only the pixel artwork below 100%.
    ((200.0 * f64::from(scale)).round().max(180.0), 140.0)
}

fn roam_bounds(
    work_area: tauri::PhysicalRect<i32, u32>,
    window_size: tauri::PhysicalSize<u32>,
) -> (i32, i32, i32) {
    let min_x = work_area.position.x;
    let max_x = work_area.position.x + work_area.size.width as i32 - window_size.width as i32;
    let floor_y = work_area.position.y + work_area.size.height as i32 - window_size.height as i32;
    (min_x, max_x.max(min_x), floor_y)
}

fn clamp_coordinate(value: i32, minimum: i32, maximum: i32) -> i32 {
    value.clamp(minimum, maximum.max(minimum))
}

fn fit_pet_window(window: &tauri::WebviewWindow, scale: f32) -> Result<(), String> {
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or("Không tìm thấy màn hình.")?;
    let (width, height) = pet_dimensions(scale);
    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|error| error.to_string())?;
    let size = window.outer_size().map_err(|error| error.to_string())?;
    let current = window.outer_position().map_err(|error| error.to_string())?;
    let (min_x, max_x, floor_y) = roam_bounds(*monitor.work_area(), size);
    window
        .set_position(PhysicalPosition::new(
            clamp_coordinate(current.x, min_x, max_x),
            floor_y,
        ))
        .map_err(|error| error.to_string())
}

fn fit_window(window: &tauri::WebviewWindow, width: f64, height: f64) -> Result<(), String> {
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or("Không tìm thấy màn hình.")?;
    let scale = monitor.scale_factor();
    let work_area = monitor.work_area();
    let max_width =
        ((work_area.size.width as i32 - WINDOW_MARGIN * 2).max(1) as f64 / scale).floor();
    let max_height =
        ((work_area.size.height as i32 - WINDOW_MARGIN * 2).max(1) as f64 / scale).floor();
    let width = width.min(max_width);
    let height = height.min(max_height);
    let size = LogicalSize::new(width, height);
    let current = window.outer_position().map_err(|error| error.to_string())?;
    let current_size = window.outer_size().map_err(|error| error.to_string())?;
    let target_width = (width * scale).round() as i32;
    let target_height = (height * scale).round() as i32;
    let right = current.x + current_size.width as i32;
    let bottom = current.y + current_size.height as i32;
    let min_x = work_area.position.x + WINDOW_MARGIN;
    let min_y = work_area.position.y + WINDOW_MARGIN;
    let max_x = work_area.position.x + work_area.size.width as i32 - target_width - WINDOW_MARGIN;
    let max_y = work_area.position.y + work_area.size.height as i32 - target_height - WINDOW_MARGIN;
    window.set_size(size).map_err(|error| error.to_string())?;
    window
        .set_position(PhysicalPosition::new(
            clamp_coordinate(right - target_width, min_x, max_x),
            clamp_coordinate(bottom - target_height, min_y, max_y),
        ))
        .map_err(|error| error.to_string())
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
    let seconds =
        ((end_minutes - now_minutes).rem_euclid(24 * 60) * 60 - i64::from(now.second())).max(1);
    StdDuration::from_secs(seconds as u64)
}

enum ReminderAction {
    Notify(i64),
    Wait(StdDuration),
}

fn reminder_action(
    state: &AppState,
    last_notification: Option<Instant>,
) -> Result<ReminderAction, String> {
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    if let Ok(snoozed_until) = setting(&connection, "reminder_snoozed_until") {
        if let Ok(snoozed_until) = parse_due(&snoozed_until) {
            let current = Utc::now();
            if snoozed_until > current {
                return Ok(ReminderAction::Wait(
                    snoozed_until
                        .signed_duration_since(current)
                        .to_std()
                        .unwrap_or(StdDuration::ZERO),
                ));
            }
            upsert_setting(&connection, "reminder_snoozed_until", "")?;
        }
    }
    let next_due_at: Option<String> = connection
        .query_row(
            "SELECT due_at FROM vocabulary WHERE translation IS NOT NULL AND trim(translation) != '' ORDER BY due_at LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some(next_due_at) = next_due_at else {
        return Ok(ReminderAction::Wait(NO_CARD_CHECK));
    };
    let next_due = parse_due(&next_due_at)?;
    let current = Utc::now();
    if next_due > current {
        return Ok(ReminderAction::Wait(
            next_due
                .signed_duration_since(current)
                .to_std()
                .unwrap_or(StdDuration::ZERO),
        ));
    }

    let quiet_start = setting(&connection, "quiet_hours_start")?
        .parse()
        .unwrap_or(22);
    let quiet_end = setting(&connection, "quiet_hours_end")?
        .parse()
        .unwrap_or(8);
    if is_quiet_hour(Local::now().hour(), quiet_start, quiet_end) {
        return Ok(ReminderAction::Wait(until_quiet_ends(
            Local::now(),
            quiet_end,
        )));
    }
    if let Some(last) = last_notification {
        let elapsed = last.elapsed();
        if elapsed < REMINDER_COOLDOWN {
            return Ok(ReminderAction::Wait(REMINDER_COOLDOWN - elapsed));
        }
    }
    let due_count = connection
        .query_row(
            "SELECT COUNT(*) FROM vocabulary WHERE due_at <= ?1 AND translation IS NOT NULL AND trim(translation) != ''",
            [now()],
            |row| row.get(0),
        )
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
                        let _ = app
                            .notification()
                            .builder()
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
        .query_row(
            "SELECT COUNT(*) FROM vocabulary WHERE due_at <= ?1 AND translation IS NOT NULL AND trim(translation) != ''",
            [&current_time],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let total_words = connection
        .query_row("SELECT COUNT(*) FROM vocabulary", [], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    let next_due_at = connection
        .query_row(
            "SELECT due_at FROM vocabulary WHERE translation IS NOT NULL AND trim(translation) != '' ORDER BY due_at LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let due_word = if due_count > 0 {
        connection
            .query_row(
                "SELECT term FROM vocabulary WHERE due_at <= ?1 AND translation IS NOT NULL AND trim(translation) != '' ORDER BY due_at, id LIMIT 1",
                [&current_time],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
    } else {
        None
    };
    let pet_passthrough = setting(&connection, "pet_passthrough")? == "true";
    Ok(Dashboard {
        due_count,
        total_words,
        next_due_at,
        due_word,
        pet_passthrough,
    })
}

#[tauri::command]
fn get_settings(app: AppHandle, state: State<'_, AppState>) -> Result<SettingsPayload, String> {
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    Ok(SettingsPayload {
        endpoint: setting(&connection, "endpoint")?,
        model: setting(&connection, "model")?,
        audio_model: setting(&connection, "audio_model").unwrap_or_default(),
        desired_retention: setting(&connection, "desired_retention")?
            .parse()
            .unwrap_or(0.9),
        quiet_hours_start: setting(&connection, "quiet_hours_start")?
            .parse()
            .unwrap_or(22),
        quiet_hours_end: setting(&connection, "quiet_hours_end")?
            .parse()
            .unwrap_or(8),
        pet_passthrough: setting(&connection, "pet_passthrough")? == "true",
        pet_scale: pet_scale(&connection),
        pet_opacity: setting(&connection, "pet_opacity")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1.0),
        ai_enabled: setting(&connection, "ai_enabled")? == "true",
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        has_api_key: has_api_key(),
    })
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    input: SaveSettingsInput,
) -> Result<(), String> {
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
    if input.audio_model.chars().count() > 200 {
        return Err("Model chấm phát âm tối đa 200 ký tự.".into());
    }
    if input.quiet_hours_start > 23 || input.quiet_hours_end > 23 {
        return Err("Giờ yên lặng không hợp lệ.".into());
    }
    if !input.pet_scale.is_finite() || !(0.1..=1.25).contains(&input.pet_scale) {
        return Err("Kích thước pet phải nằm trong 10% đến 125%.".into());
    }
    if !input.pet_opacity.is_finite() || !(0.2..=1.0).contains(&input.pet_opacity) {
        return Err("Độ trong suốt phải nằm trong 20% đến 100%.".into());
    }

    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    upsert_setting(&connection, "endpoint", &endpoint)?;
    upsert_setting(&connection, "model", input.model.trim())?;
    upsert_setting(&connection, "audio_model", input.audio_model.trim())?;
    upsert_setting(
        &connection,
        "desired_retention",
        &input.desired_retention.to_string(),
    )?;
    upsert_setting(
        &connection,
        "quiet_hours_start",
        &input.quiet_hours_start.to_string(),
    )?;
    upsert_setting(
        &connection,
        "quiet_hours_end",
        &input.quiet_hours_end.to_string(),
    )?;
    upsert_setting(
        &connection,
        "pet_passthrough",
        &input.pet_passthrough.to_string(),
    )?;
    upsert_setting(&connection, "pet_scale", &input.pet_scale.to_string())?;
    upsert_setting(&connection, "pet_size", legacy_pet_size(input.pet_scale))?;
    upsert_setting(&connection, "pet_opacity", &input.pet_opacity.to_string())?;
    upsert_setting(&connection, "ai_enabled", &input.ai_enabled.to_string())?;
    drop(connection);

    if let Some(key) = input.api_key.map(|key| key.trim().to_string()) {
        let entry = Entry::new(SERVICE_NAME, SECRET_NAME).map_err(|error| error.to_string())?;
        if key.is_empty() {
            let _ = entry.delete_credential();
        } else {
            entry
                .set_password(&key)
                .map_err(|error| error.to_string())?;
        }
    }
    let autostart = app.autolaunch();
    if input.autostart && !autostart.is_enabled().unwrap_or(false) {
        autostart.enable().map_err(|error| error.to_string())?;
    } else if !input.autostart && autostart.is_enabled().unwrap_or(false) {
        autostart.disable().map_err(|error| error.to_string())?;
    }
    set_passthrough(&app, input.pet_passthrough)?;
    let window = app
        .get_webview_window("main")
        .ok_or("Pet window unavailable")?;
    fit_pet_window(&window, input.pet_scale)?;
    reschedule_reminder(&state);
    Ok(())
}

#[tauri::command]
fn capture_clipboard() -> Result<String, String> {
    let mut clipboard =
        Clipboard::new().map_err(|error| format!("Không mở được clipboard: {error}"))?;
    let text = clipboard.get_text().map_err(|_| {
        "Clipboard không có text. Hãy copy từ trước hoặc bôi đen text rồi bấm Ctrl+Shift+E."
            .to_string()
    })?;
    let (_, term) = normalize_term(&text)?;
    Ok(term)
}

#[tauri::command]
fn save_word(state: State<'_, AppState>, input: SaveWordInput) -> Result<CapturedWord, String> {
    let (normalized_term, term) = normalize_term(&input.term)?;
    let ipa = normalize_ipa(input.ipa)?;
    let translation = input
        .translation
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string());
    let example = input
        .example
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string());
    let topic = validate_topic(input.topic)?;
    let timestamp = now();
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    transaction.execute(
        "INSERT INTO vocabulary(term, normalized_term, ipa, translation, example, topic, due_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?7)",
        params![term, normalized_term, ipa, translation, example, topic, timestamp],
    ).map_err(|error| {
        if error.to_string().contains("UNIQUE") { "Từ này đã có trong Inbox.".to_string() } else { error.to_string() }
    })?;
    let id = transaction.last_insert_rowid();
    transaction.commit().map_err(|error| error.to_string())?;
    reschedule_reminder(&state);
    Ok(CapturedWord {
        id,
        term,
        translation,
        topic,
        created_at: timestamp,
    })
}

fn list_vocabulary_from_connection(
    connection: &Connection,
    input: &ListVocabularyInput,
) -> Result<VocabularyPage, String> {
    let search = input.search.as_deref().unwrap_or_default().trim();
    if search.chars().count() > 100 {
        return Err("Từ khóa tìm kiếm tối đa 100 ký tự.".into());
    }
    let topic = match input.topic.as_deref() {
        Some("unclassified") => Some("unclassified"),
        Some(value) if TOPICS.contains(&value) => Some(value),
        Some(_) => return Err("Bộ lọc chủ đề không hợp lệ.".into()),
        None => None,
    };
    let offset = input.offset.unwrap_or(0).max(0);
    let pattern = format!("%{search}%");
    let filter = match topic {
        Some("unclassified") => "topic IS NULL AND ?2 = ''",
        Some(_) => "topic = ?2",
        None => "?2 = ''",
    };
    let sql = format!(
        "SELECT id, term, ipa, translation, example, topic, due_at, created_at FROM vocabulary
         WHERE (term LIKE ?1 OR COALESCE(translation, '') LIKE ?1) AND {filter}
         ORDER BY created_at DESC, id DESC LIMIT 50 OFFSET ?3"
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| error.to_string())?;
    let topic_parameter = topic.filter(|value| *value != "unclassified").unwrap_or("");
    let items = statement
        .query_map(params![pattern, topic_parameter, offset], |row| {
            Ok(VocabularyItem {
                id: row.get(0)?,
                term: row.get(1)?,
                ipa: row.get(2)?,
                translation: row.get(3)?,
                example: row.get(4)?,
                topic: row.get(5)?,
                due_at: row.get(6)?,
                created_at: row.get(7)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let count_sql = format!(
        "SELECT COUNT(*) FROM vocabulary WHERE (term LIKE ?1 OR COALESCE(translation, '') LIKE ?1) AND {filter}"
    );
    let total = connection
        .query_row(&count_sql, params![pattern, topic_parameter], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    let unclassified_count = connection
        .query_row(
            "SELECT COUNT(*) FROM vocabulary WHERE topic IS NULL",
            [],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(VocabularyPage {
        items,
        total,
        unclassified_count,
    })
}

#[tauri::command]
fn list_vocabulary(
    state: State<'_, AppState>,
    input: ListVocabularyInput,
) -> Result<VocabularyPage, String> {
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    list_vocabulary_from_connection(&connection, &input)
}

fn normalize_definition(definition: GeneratedDefinition) -> Result<GeneratedDefinition, String> {
    let translation = definition.translation.trim().to_owned();
    let example = definition.example.trim().to_owned();
    if translation.is_empty()
        || example.is_empty()
        || translation.chars().count() > 2_000
        || example.chars().count() > 5_000
    {
        return Err("Model trả nghĩa hoặc ví dụ không hợp lệ.".into());
    }
    let topic = validate_topic(Some(definition.topic))?.ok_or("Model trả chủ đề không hợp lệ.")?;
    Ok(GeneratedDefinition {
        translation,
        example,
        topic,
        ipa: normalize_ipa(definition.ipa)?,
    })
}

#[tauri::command]
fn update_word(state: State<'_, AppState>, input: UpdateWordInput) -> Result<(), String> {
    if input.id <= 0 {
        return Err("ID từ không hợp lệ.".into());
    }
    let translation = input.translation.trim().to_owned();
    if translation.is_empty() || translation.chars().count() > 2_000 {
        return Err("Nhập nghĩa tiếng Việt, tối đa 2.000 ký tự.".into());
    }
    let example = input
        .example
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if example
        .as_deref()
        .is_some_and(|value| value.chars().count() > 5_000)
    {
        return Err("Ví dụ tối đa 5.000 ký tự.".into());
    }
    let topic = validate_topic(input.topic)?;
    let ipa = normalize_ipa(input.ipa)?;
    let timestamp = now();
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    let updated = connection
        .execute(
            "UPDATE vocabulary
             SET ipa = ?1, translation = ?2, example = ?3, topic = ?4,
                 due_at = CASE WHEN last_reviewed_at IS NULL THEN ?5 ELSE due_at END,
                 updated_at = ?5
             WHERE id = ?6",
            params![ipa, translation, example, topic, timestamp, input.id],
        )
        .map_err(|error| error.to_string())?;
    if updated == 0 {
        return Err("Không tìm thấy từ trong Inbox.".into());
    }
    drop(connection);
    reschedule_reminder(&state);
    Ok(())
}

fn delete_pending_word_from_connection(connection: &Connection, id: i64) -> Result<(), String> {
    if id <= 0 {
        return Err("ID từ không hợp lệ.".into());
    }
    let deleted = connection
        .execute(
            "DELETE FROM vocabulary WHERE id = ?1 AND (translation IS NULL OR trim(translation) = '')",
            [id],
        )
        .map_err(|error| error.to_string())?;
    if deleted == 0 {
        return Err("Chỉ có thể xóa từ chờ định nghĩa.".into());
    }
    Ok(())
}

#[tauri::command]
fn delete_pending_word(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
    delete_pending_word_from_connection(&connection, id)?;
    drop(connection);
    reschedule_reminder(&state);
    Ok(())
}

#[tauri::command]
async fn batch_define_vocabulary(
    state: State<'_, AppState>,
    input: BatchDefineVocabularyInput,
) -> Result<CategorizeResult, String> {
    let mut ids = input
        .ids
        .into_iter()
        .filter(|id| *id > 0)
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Ok(CategorizeResult {
            updated_count: 0,
            failed_count: 0,
        });
    }
    if ids.len() > 100 {
        return Err("Mỗi lần chỉ định nghĩa tối đa 100 từ.".into());
    }
    let (endpoint, model, enabled, words) = {
        let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
        let mut query = connection
            .prepare("SELECT id, term FROM vocabulary WHERE id = ?1 AND (translation IS NULL OR trim(translation) = '')")
            .map_err(|error| error.to_string())?;
        let words = ids
            .iter()
            .filter_map(|id| {
                query
                    .query_row([id], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                    })
                    .optional()
                    .ok()
                    .flatten()
            })
            .collect::<Vec<_>>();
        (
            setting(&connection, "endpoint")?,
            setting(&connection, "model")?,
            setting(&connection, "ai_enabled")? == "true",
            words,
        )
    };
    if !enabled {
        return Err("AI đang tắt trong Settings.".into());
    }
    if model.is_empty() {
        return Err("Chọn model 9Router trong Settings trước.".into());
    }
    let mut updated_count = 0;
    let mut failed_count = 0;
    for chunk in words.chunks(10) {
        let terms = chunk.iter().map(|(_, term)| term).collect::<Vec<_>>();
        let prompt = format!(
            "For each English term, return concise Vietnamese translation, IPA pronunciation, a natural B1 English example, and exactly one topic ID from: {}. Return only JSON {{\"definitions\": {{\"term\": {{\"translation\": \"...\", \"ipa\": \"/.../\", \"example\": \"...\", \"topic\": \"topic_id\"}}}}}}. Terms: {}",
            TOPICS.join(", "),
            serde_json::to_string(&terms).map_err(|error| error.to_string())?
        );
        let mut request = state
            .http
            .post(format!("{endpoint}/chat/completions"))
            .json(&json!({
                "model": model, "stream": false, "response_format": {"type": "json_object"},
                "messages": [{"role": "user", "content": prompt}], "max_tokens": 1_200
            }));
        if let Some(key) = api_key()? {
            request = request.bearer_auth(key);
        }
        let result = async {
            let response: Value = request
                .send()
                .await
                .map_err(|error| format!("Không kết nối được 9Router: {error}"))?
                .error_for_status()
                .map_err(|error| format!("9Router lỗi: {error}"))?
                .json()
                .await
                .map_err(|error| format!("9Router trả response lỗi: {error}"))?;
            let content = response["choices"][0]["message"]["content"]
                .as_str()
                .ok_or("9Router không trả nội dung.")?;
            serde_json::from_str::<DefinitionAssignments>(content)
                .map_err(|_| "Model trả JSON định nghĩa không hợp lệ.".to_string())
        }
        .await;
        let Ok(assignments) = result else {
            failed_count += chunk.len();
            continue;
        };
        let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| error.to_string())?;
        for (id, term) in chunk {
            let Some(definition) = assignments.definitions.get(term).cloned() else {
                failed_count += 1;
                continue;
            };
            let Ok(definition) = normalize_definition(definition) else {
                failed_count += 1;
                continue;
            };
            updated_count += transaction
                .execute(
                    "UPDATE vocabulary SET ipa = ?1, translation = ?2, example = ?3, topic = ?4,
                 due_at = CASE WHEN last_reviewed_at IS NULL THEN ?5 ELSE due_at END,
                 updated_at = ?5
                 WHERE id = ?6 AND (translation IS NULL OR trim(translation) = '')",
                    params![
                        definition.ipa,
                        definition.translation,
                        definition.example,
                        definition.topic,
                        now(),
                        id
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
    }
    reschedule_reminder(&state);
    Ok(CategorizeResult {
        updated_count,
        failed_count,
    })
}

fn backup_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .document_dir()
        .map_err(|error| error.to_string())?
        .join("English Pet Backups");
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    Ok(dir)
}

fn backup_filename(extension: &str) -> String {
    format!(
        "english-pet-{}.{}",
        Utc::now().format("%Y%m%d-%H%M%S"),
        extension
    )
}

fn export_from_connection(connection: &Connection) -> Result<ExportData, String> {
    let mut vocabulary_query = connection.prepare(
        "SELECT id, term, normalized_term, ipa, translation, example, topic, stability, difficulty, due_at, last_reviewed_at, created_at, updated_at
         FROM vocabulary ORDER BY id",
    ).map_err(|error| error.to_string())?;
    let vocabulary = vocabulary_query
        .query_map([], |row| {
            Ok(ExportVocabulary {
                id: row.get(0)?,
                term: row.get(1)?,
                normalized_term: row.get(2)?,
                ipa: row.get(3)?,
                translation: row.get(4)?,
                example: row.get(5)?,
                topic: row.get(6)?,
                stability: row.get(7)?,
                difficulty: row.get(8)?,
                due_at: row.get(9)?,
                last_reviewed_at: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut review_query = connection.prepare(
        "SELECT vocabulary_id, rating, elapsed_days, scheduled_days, reviewed_at, state_before_json, state_after_json
         FROM review_logs ORDER BY id",
    ).map_err(|error| error.to_string())?;
    let review_logs = review_query
        .query_map([], |row| {
            Ok(ExportReviewLog {
                vocabulary_id: row.get(0)?,
                rating: row.get(1)?,
                elapsed_days: row.get(2)?,
                scheduled_days: row.get(3)?,
                reviewed_at: row.get(4)?,
                state_before_json: row.get(5)?,
                state_after_json: row.get(6)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(ExportData {
        version: 2,
        exported_at: now(),
        vocabulary,
        review_logs,
    })
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
    connection
        .backup(rusqlite::MAIN_DB, &path, None)
        .map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}

fn validate_import_word(word: &ExportVocabulary) -> Result<(String, String), String> {
    let (normalized_term, term) = normalize_term(&word.term)?;
    if word
        .translation
        .as_deref()
        .is_some_and(|text| text.chars().count() > 2_000)
        || word
            .example
            .as_deref()
            .is_some_and(|text| text.chars().count() > 5_000)
    {
        return Err("Import chứa nghĩa hoặc ví dụ quá dài.".into());
    }
    validate_topic(word.topic.clone())?;
    normalize_ipa(word.ipa.clone())?;
    parse_due(&word.due_at)?;
    parse_due(&word.created_at)?;
    parse_due(&word.updated_at)?;
    if let Some(reviewed_at) = &word.last_reviewed_at {
        parse_due(reviewed_at)?;
    }
    Ok((normalized_term, term))
}

fn import_into_connection(
    connection: &Connection,
    data: ExportData,
) -> Result<ImportResult, String> {
    if !matches!(data.version, 1 | 2) {
        return Err(
            "File backup không được hỗ trợ. Cần English Pet export version 1 hoặc 2.".into(),
        );
    }
    if data.vocabulary.len() > 100_000 || data.review_logs.len() > 1_000_000 {
        return Err("File backup quá lớn.".into());
    }
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let mut imported_words = 0;
    let mut skipped_words = 0;
    let mut imported_reviews = 0;
    let mut ids = HashMap::new();
    for word in data.vocabulary {
        let original_id = word.id;
        let (normalized_term, term) = validate_import_word(&word)?;
        let duplicate: Option<i64> = transaction
            .query_row(
                "SELECT id FROM vocabulary WHERE normalized_term = ?1",
                [&normalized_term],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if duplicate.is_some() {
            skipped_words += 1;
            continue;
        }
        transaction.execute(
            "INSERT INTO vocabulary(term, normalized_term, ipa, translation, example, topic, stability, difficulty, due_at, last_reviewed_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![term, normalized_term, word.ipa, word.translation, word.example, word.topic, word.stability, word.difficulty,
                word.due_at, word.last_reviewed_at, word.created_at, word.updated_at],
        ).map_err(|error| error.to_string())?;
        if ids
            .insert(original_id, transaction.last_insert_rowid())
            .is_some()
        {
            return Err("File backup có ID từ vựng trùng lặp.".into());
        }
        imported_words += 1;
    }
    for log in data.review_logs {
        let Some(&vocabulary_id) = ids.get(&log.vocabulary_id) else {
            continue;
        };
        if !(1..=4).contains(&log.rating) || log.elapsed_days < 0 || log.scheduled_days < 0 {
            return Err("File backup có review log không hợp lệ.".into());
        }
        parse_due(&log.reviewed_at)?;
        serde_json::from_str::<Value>(&log.state_after_json)
            .map_err(|_| "File backup có FSRS state lỗi.".to_string())?;
        if let Some(state_before) = &log.state_before_json {
            serde_json::from_str::<Value>(state_before)
                .map_err(|_| "File backup có FSRS state lỗi.".to_string())?;
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
    Ok(ImportResult {
        imported_words,
        skipped_words,
        imported_reviews,
    })
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
    let tomorrow = Local::now()
        .date_naive()
        .succ_opt()
        .ok_or("Không tính được ngày mai.")?;
    let local_time = tomorrow
        .and_hms_opt(8, 0, 0)
        .ok_or("Không tính được giờ nhắc.")?;
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
    }
    .to_rfc3339();
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
            "SELECT id, term, ipa, translation, example, due_at FROM vocabulary WHERE due_at <= ?1 AND translation IS NOT NULL AND trim(translation) != '' ORDER BY due_at, id LIMIT 1",
            [now()],
            |row| Ok(ReviewCard {
                id: row.get(0)?, term: row.get(1)?, ipa: row.get(2)?, translation: row.get(3)?, example: row.get(4)?, due_at: row.get(5)?,
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
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| error.to_string())?;
    let row: (Option<f32>, Option<f32>, Option<String>) = transaction
        .query_row(
            "SELECT stability, difficulty, last_reviewed_at FROM vocabulary WHERE id = ?1",
            [card_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| "Không tìm thấy thẻ.".to_string())?;
    let previous = match (row.0, row.1) {
        (Some(stability), Some(difficulty)) => Some(MemoryState {
            stability,
            difficulty,
        }),
        _ => None,
    };
    let elapsed_days = row
        .2
        .as_deref()
        .and_then(|value| parse_due(value).ok())
        .map(|value| (Utc::now() - value).num_days().max(0) as u32)
        .unwrap_or(0);
    let desired_retention: f32 = setting(&transaction, "desired_retention")?
        .parse()
        .unwrap_or(0.9);
    let states = FSRS::default()
        .next_states(previous, desired_retention, elapsed_days)
        .map_err(|error| error.to_string())?;
    let next = match rating {
        1 => states.again,
        2 => states.hard,
        3 => states.good,
        4 => states.easy,
        _ => unreachable!(),
    };
    let scheduled_days = next.interval.round().max(1.0) as i64;
    let reviewed_at = now();
    let due_at = (Utc::now() + Duration::days(scheduled_days)).to_rfc3339();
    let before = previous.map(|state| {
        json!({"stability": state.stability, "difficulty": state.difficulty}).to_string()
    });
    let after = json!({"stability": next.memory.stability, "difficulty": next.memory.difficulty})
        .to_string();
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
        (
            setting(&connection, "endpoint")?,
            setting(&connection, "ai_enabled")? == "true",
        )
    };
    if !enabled {
        return Ok(Vec::new());
    }
    let mut request = state.http.get(format!("{endpoint}/models"));
    if let Some(key) = api_key()? {
        request = request.bearer_auth(key);
    }
    let body: Value = request
        .send()
        .await
        .map_err(|error| format!("Không kết nối được 9Router: {error}"))?
        .error_for_status()
        .map_err(|error| format!("9Router lỗi: {error}"))?
        .json()
        .await
        .map_err(|error| format!("9Router trả response lỗi: {error}"))?;
    Ok(body["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str())
        .map(str::to_owned)
        .collect())
}

#[tauri::command]
async fn generate_content(
    state: State<'_, AppState>,
    input: GenerateInput,
) -> Result<GeneratedContent, String> {
    let (_, term) = normalize_term(&input.term)?;
    let (endpoint, model, enabled) = {
        let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
        (
            setting(&connection, "endpoint")?,
            setting(&connection, "model")?,
            setting(&connection, "ai_enabled")? == "true",
        )
    };
    if !enabled {
        return Err("AI đang tắt trong Settings.".into());
    }
    if model.is_empty() {
        return Err("Chọn model 9Router trong Settings trước.".into());
    }
    let known_translation = input
        .translation
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_owned());
    let prompt = format!(
        "You help a Vietnamese learner learn English. For the term {term:?}{} return only one JSON object with keys translation, ipa, example, exercise, topic. translation: concise Vietnamese meaning. ipa: concise International Phonetic Alphabet pronunciation, including /.../. example: natural English sentence at B1 level. exercise: one short cloze question. topic must be exactly one of: {}. No markdown.",
        known_translation
            .map(|value| format!(" (known Vietnamese meaning: {value:?})"))
            .unwrap_or_default(),
        TOPICS.join(", ")
    );
    let mut request = state
        .http
        .post(format!("{endpoint}/chat/completions"))
        .json(&json!({
            "model": model,
            "stream": false,
            "response_format": {"type": "json_object"},
            "messages": [{"role": "user", "content": prompt}],
            "max_tokens": 300
        }));
    if let Some(key) = api_key()? {
        request = request.bearer_auth(key);
    }
    let response: Value = request
        .send()
        .await
        .map_err(|error| format!("Không kết nối được 9Router: {error}"))?
        .error_for_status()
        .map_err(|error| format!("9Router lỗi: {error}"))?
        .json()
        .await
        .map_err(|error| format!("9Router trả response lỗi: {error}"))?;
    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("9Router không trả nội dung.")?;
    let generated: GeneratedContent = serde_json::from_str(content).map_err(|_| {
        "Model trả JSON không hợp lệ. Hãy thử model khác hoặc nhập tay.".to_string()
    })?;
    if generated.translation.trim().is_empty()
        || generated.example.trim().is_empty()
        || generated.exercise.trim().is_empty()
    {
        return Err("Model trả nội dung thiếu trường bắt buộc.".into());
    }
    validate_topic(Some(generated.topic.clone()))
        .map_err(|_| "Model trả chủ đề không hợp lệ.".to_string())?;
    normalize_ipa(generated.ipa.clone())?;
    Ok(generated)
}

#[tauri::command]
async fn evaluate_pronunciation(
    state: State<'_, AppState>,
    input: EvaluatePronunciationInput,
) -> Result<PronunciationAssessment, String> {
    if input.card_id <= 0 {
        return Err("ID từ không hợp lệ.".into());
    }
    if input.audio_base64.len() > 1_400_000 {
        return Err("Bản ghi quá lớn. Ghi tối đa 10 giây rồi thử lại.".into());
    }
    let audio = BASE64
        .decode(input.audio_base64.as_bytes())
        .map_err(|_| "Bản ghi audio không hợp lệ.".to_string())?;
    if audio.len() < 44
        || audio.len() > 1_000_000
        || &audio[0..4] != b"RIFF"
        || &audio[8..12] != b"WAVE"
    {
        return Err("Bản ghi phải là WAV tối đa 1 MB.".into());
    }
    let (endpoint, model, enabled, term, ipa) = {
        let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
        let (term, ipa): (String, Option<String>) = connection
            .query_row(
                "SELECT term, ipa FROM vocabulary WHERE id = ?1 AND translation IS NOT NULL AND trim(translation) != ''",
                [input.card_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| "Không tìm thấy thẻ để chấm phát âm.".to_string())?;
        let audio_model = setting(&connection, "audio_model").unwrap_or_default();
        (
            setting(&connection, "endpoint")?,
            if audio_model.trim().is_empty() {
                setting(&connection, "model")?
            } else {
                audio_model
            },
            setting(&connection, "ai_enabled")? == "true",
            term,
            ipa,
        )
    };
    if !enabled {
        return Err("AI đang tắt trong Settings.".into());
    }
    if model.trim().is_empty() {
        return Err("Chọn model hỗ trợ audio trong Settings trước.".into());
    }
    let prompt = format!(
        "Listen to this Vietnamese learner pronounce English target {term:?}. Expected IPA: {}. Return only JSON {{\"score\": 0-100, \"isCorrect\": boolean, \"feedback\": \"concise Vietnamese feedback\", \"recognizedText\": \"what was heard\", \"segments\": [{{\"part\": \"sound or word part\", \"status\": \"correct|needs_improvement\", \"note\": \"Vietnamese tip\"}}]}}.",
        ipa.as_deref().unwrap_or("not available")
    );
    let mut request = state
        .http
        .post(format!("{endpoint}/chat/completions"))
        .json(&json!({
            "model": model,
            "stream": false,
            "response_format": {"type": "json_object"},
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": prompt},
                {"type": "input_audio", "input_audio": {"data": input.audio_base64, "format": "wav"}}
            ]}],
            "max_tokens": 500
        }));
    if let Some(key) = api_key()? {
        request = request.bearer_auth(key);
    }
    let response: Value = request
        .send()
        .await
        .map_err(|error| format!("Không kết nối được 9Router: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Model {model:?} không hỗ trợ audio hoặc 9Router lỗi: {error}"))?
        .json()
        .await
        .map_err(|error| format!("9Router trả response lỗi: {error}"))?;
    let content = response["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("9Router không trả chấm phát âm.")?;
    let assessment = serde_json::from_str(content)
        .map_err(|_| "Model trả JSON chấm phát âm không hợp lệ.".to_string())?;
    validate_assessment(assessment)
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

#[tauri::command]
fn set_window_mode(app: AppHandle, state: State<'_, AppState>, mode: String) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("Pet window unavailable")?;
    let (width, height) = match mode.as_str() {
        "capture" | "review" => (460.0, 620.0),
        "settings" | "inbox" | "edit" => (580.0, 800.0),
        "pet" => {
            let connection = state.db.lock().map_err(|_| "Database busy".to_string())?;
            return fit_pet_window(&window, pet_scale(&connection));
        }
        _ => return Err("Chế độ cửa sổ không hợp lệ.".into()),
    };
    fit_window(&window, width, height)
}

#[tauri::command]
fn step_pet_roam(app: AppHandle, delta_x: i32, jump_offset: i32) -> Result<RoamStep, String> {
    let window = app
        .get_webview_window("main")
        .ok_or("Pet window unavailable")?;
    let monitor = window
        .current_monitor()
        .map_err(|error| error.to_string())?
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or("Không tìm thấy màn hình.")?;
    let scale = monitor.scale_factor();
    let window_size = window.outer_size().map_err(|error| error.to_string())?;
    let current = window.outer_position().map_err(|error| error.to_string())?;
    let (min_x, max_x, floor_y) = roam_bounds(*monitor.work_area(), window_size);
    let target_x = current.x + (f64::from(delta_x) * scale).round() as i32;
    let hit_edge = target_x <= min_x || target_x >= max_x;
    let jump = (f64::from(jump_offset.max(0)) * scale).round() as i32;
    window
        .set_position(PhysicalPosition::new(
            clamp_coordinate(target_x, min_x, max_x),
            floor_y - jump,
        ))
        .map_err(|error| error.to_string())?;
    Ok(RoamStep { hit_edge })
}

fn set_passthrough(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or("Pet window unavailable")?;
    window
        .set_ignore_cursor_events(enabled)
        .map_err(|error| error.to_string())
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
    let capture = MenuItem::with_id(app, "capture", "Thêm từ từ clipboard", true, None::<&str>)?;
    let passthrough = MenuItem::with_id(
        app,
        "passthrough",
        "Bật/tắt click xuyên pet",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Thoát", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&review, &capture, &passthrough, &quit])?;
    TrayIconBuilder::with_id("tray")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .tooltip("English Pet")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "review" => {
                let _ = app.emit("open-review", ());
                show_pet(app);
            }
            "capture" => {
                let _ = app.emit("capture-clipboard", ());
                show_pet(app);
            }
            "passthrough" => {
                if let Ok(state) = app.try_state::<AppState>().ok_or(()) {
                    let _ = toggle_passthrough(app.clone(), state);
                }
            }
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
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcut(HOTKEY)
                .expect("valid hotkey")
                .with_handler(|app, _, event| {
                    if event.state == ShortcutState::Pressed {
                        let app = app.clone();
                        thread::spawn(move || match selection_capture() {
                            Ok(term) => {
                                let _ = app.emit("capture-selection", term);
                                show_pet(&app);
                            }
                            Err(error) => {
                                let _ = app.emit("capture-error", error);
                            }
                        });
                    }
                })
                .build(),
        )
        .setup(move |app| {
            let db_path = app_db_path(app.handle())
                .map_err(|error| Box::<dyn std::error::Error>::from(error))?;
            let connection = Connection::open(db_path)?;
            initialize_db(&connection)
                .map_err(|error| Box::<dyn std::error::Error>::from(error))?;
            let (reminder_tx, reminder_rx) = mpsc::channel();
            app.manage(AppState {
                db: Mutex::new(connection),
                http: Client::builder()
                    .timeout(StdDuration::from_secs(20))
                    .build()?,
                reminder_tx,
            });
            setup_tray(app.handle())?;
            let (passthrough, scale) = app
                .state::<AppState>()
                .db
                .lock()
                .ok()
                .map(|db| {
                    (
                        setting(&db, "pet_passthrough").is_ok_and(|value| value == "true"),
                        pet_scale(&db),
                    )
                })
                .unwrap_or((false, 1.0));
            if let Some(window) = app.get_webview_window("main") {
                let _ = fit_pet_window(&window, scale);
            }
            if passthrough {
                let _ = set_passthrough(app.handle(), true);
            }
            start_reminder_scheduler(app.handle().clone(), reminder_rx);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            dashboard,
            get_settings,
            save_settings,
            capture_clipboard,
            save_word,
            list_vocabulary,
            update_word,
            delete_pending_word,
            batch_define_vocabulary,
            export_data,
            backup_sqlite,
            import_data,
            snooze_reminder,
            next_review,
            rate_review,
            ai_models,
            generate_content,
            evaluate_pronunciation,
            toggle_passthrough,
            set_window_mode,
            step_pet_roam
        ])
        .build(context)
        .expect("error while building Tauri application")
        .run(|app, event| {
            if let RunEvent::ExitRequested { .. } = event {
                app.exit(0);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pet_scale_keeps_legacy_settings_and_ignores_invalid_values() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_db(&connection).unwrap();
        upsert_setting(&connection, "pet_size", "small").unwrap();
        assert_eq!(pet_scale(&connection), 0.8);

        upsert_setting(&connection, "pet_scale", "0.1").unwrap();
        assert_eq!(pet_scale(&connection), 0.1);
        upsert_setting(&connection, "pet_scale", "0.09").unwrap();
        assert_eq!(pet_scale(&connection), 0.8);
    }

    #[test]
    fn pet_dimensions_keep_compact_menu_canvas() {
        assert_eq!(pet_dimensions(0.1), (180.0, 140.0));
        assert_eq!(pet_dimensions(1.25), (250.0, 140.0));
        assert_eq!(legacy_pet_size(0.1), "small");
        assert_eq!(legacy_pet_size(1.0), "medium");
        assert_eq!(legacy_pet_size(1.25), "large");
    }

    #[test]
    fn roam_bounds_keep_window_above_taskbar_and_on_screen() {
        let work_area = tauri::PhysicalRect {
            position: PhysicalPosition::new(-1920, 0),
            size: tauri::PhysicalSize::new(1920, 1040),
        };
        assert_eq!(
            roam_bounds(work_area, tauri::PhysicalSize::new(200, 140)),
            (-1920, -200, 900)
        );
        assert_eq!(
            roam_bounds(work_area, tauri::PhysicalSize::new(2_000, 140)),
            (-1920, -1920, 900)
        );
    }

    #[test]
    fn normalizes_one_short_phrase() {
        assert_eq!(
            normalize_term("  take\tcare  ").unwrap(),
            ("take care".into(), "take care".into())
        );
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
    fn initialize_db_migrates_existing_vocabulary() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE vocabulary (
                    id INTEGER PRIMARY KEY, term TEXT NOT NULL, normalized_term TEXT NOT NULL UNIQUE,
                    translation TEXT, example TEXT, stability REAL, difficulty REAL, due_at TEXT NOT NULL,
                    last_reviewed_at TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
                );
                INSERT INTO vocabulary(term, normalized_term, due_at, created_at, updated_at)
                VALUES ('hello', 'hello', '2026-10-04T00:00:00+00:00', '2026-10-01T00:00:00+00:00', '2026-10-01T00:00:00+00:00');",
            )
            .unwrap();

        initialize_db(&connection).unwrap();

        assert_eq!(
            connection
                .query_row(
                    "SELECT topic FROM vocabulary WHERE term = 'hello'",
                    [],
                    |row| { row.get::<_, Option<String>>(0) }
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn vocabulary_listing_filters_and_paginates() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_db(&connection).unwrap();
        for index in 0..51 {
            let term = format!("word {index}");
            let topic = if index == 50 { Some("work_tech") } else { None };
            connection
                .execute(
                    "INSERT INTO vocabulary(term, normalized_term, translation, topic, due_at, created_at, updated_at)
                     VALUES (?1, ?1, ?2, ?3, ?4, ?4, ?4)",
                    params![term, format!("nghĩa {index}"), topic, format!("2026-10-04T00:00:{index:02}+00:00")],
                )
                .unwrap();
        }

        let first_page = list_vocabulary_from_connection(
            &connection,
            &ListVocabularyInput {
                search: None,
                topic: None,
                offset: None,
            },
        )
        .unwrap();
        let second_page = list_vocabulary_from_connection(
            &connection,
            &ListVocabularyInput {
                search: None,
                topic: None,
                offset: Some(50),
            },
        )
        .unwrap();
        let unclassified = list_vocabulary_from_connection(
            &connection,
            &ListVocabularyInput {
                search: Some("nghĩa".into()),
                topic: Some("unclassified".into()),
                offset: None,
            },
        )
        .unwrap();

        assert_eq!(first_page.total, 51);
        assert_eq!(first_page.items.len(), 50);
        assert_eq!(second_page.items.len(), 1);
        assert_eq!(unclassified.total, 50);
        assert_eq!(unclassified.unclassified_count, 50);
    }

    #[test]
    fn pending_words_stay_out_of_review_until_defined() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_db(&connection).unwrap();
        connection
            .execute(
                "INSERT INTO vocabulary(term, normalized_term, due_at, created_at, updated_at)
                 VALUES ('bark', 'bark', ?1, ?1, ?1)",
                ["2026-10-04T00:00:00+00:00"],
            )
            .unwrap();
        let pending: Option<i64> = connection
            .query_row(
                "SELECT id FROM vocabulary WHERE due_at <= ?1 AND translation IS NOT NULL AND trim(translation) != ''",
                ["2026-10-04T00:00:00+00:00"],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(pending, None);
        connection
            .execute(
                "UPDATE vocabulary SET translation = 'sủa' WHERE term = 'bark'",
                [],
            )
            .unwrap();
        let defined: Option<i64> = connection
            .query_row(
                "SELECT id FROM vocabulary WHERE due_at <= ?1 AND translation IS NOT NULL AND trim(translation) != ''",
                ["2026-10-04T00:00:00+00:00"],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert!(defined.is_some());
    }

    #[test]
    fn delete_only_removes_pending_words() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_db(&connection).unwrap();
        let timestamp = "2026-10-04T00:00:00+00:00";
        connection
            .execute(
                "INSERT INTO vocabulary(term, normalized_term, due_at, created_at, updated_at)
                 VALUES ('pending', 'pending', ?1, ?1, ?1)",
                [timestamp],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO vocabulary(term, normalized_term, translation, due_at, created_at, updated_at)
                 VALUES ('defined', 'defined', 'đã định nghĩa', ?1, ?1, ?1)",
                [timestamp],
            )
            .unwrap();

        delete_pending_word_from_connection(&connection, 1).unwrap();
        assert!(delete_pending_word_from_connection(&connection, 2).is_err());
        assert!(delete_pending_word_from_connection(&connection, 0).is_err());
        let remaining: String = connection
            .query_row("SELECT term FROM vocabulary", [], |row| row.get(0))
            .unwrap();
        assert_eq!(remaining, "defined");
    }

    #[test]
    fn topic_validation_only_accepts_taxonomy() {
        assert_eq!(
            validate_topic(Some("work_tech".into())).unwrap().as_deref(),
            Some("work_tech")
        );
        assert_eq!(validate_topic(Some(" ".into())).unwrap(), None);
        assert!(validate_topic(Some("custom topic".into())).is_err());
    }

    #[test]
    fn json_export_import_keeps_word_review_and_topic() {
        let source = Connection::open_in_memory().unwrap();
        initialize_db(&source).unwrap();
        source.execute(
            "INSERT INTO vocabulary(term, normalized_term, translation, example, topic, stability, difficulty, due_at, last_reviewed_at, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params!["take care", "take care", "chăm sóc", "Take care.", "daily_life", 2.5_f32, 4.0_f32,
                "2026-10-04T00:00:00+00:00", "2026-10-03T00:00:00+00:00", "2026-10-01T00:00:00+00:00", "2026-10-03T00:00:00+00:00"],
        ).unwrap();
        source.execute(
            "INSERT INTO review_logs(vocabulary_id, rating, elapsed_days, scheduled_days, reviewed_at, state_before_json, state_after_json)
             VALUES (1, 3, 1, 2, ?1, ?2, ?3)",
            params!["2026-10-03T00:00:00+00:00", r#"{"stability":1.0,"difficulty":5.0}"#, r#"{"stability":2.5,"difficulty":4.0}"#],
        ).unwrap();

        let export = export_from_connection(&source).unwrap();
        assert_eq!(export.version, 2);
        let destination = Connection::open_in_memory().unwrap();
        initialize_db(&destination).unwrap();
        let result = import_into_connection(&destination, export).unwrap();

        assert_eq!(result.imported_words, 1);
        assert_eq!(result.imported_reviews, 1);
        assert_eq!(
            destination
                .query_row("SELECT topic FROM vocabulary", [], |row| row
                    .get::<_, Option<String>>(0))
                .unwrap()
                .as_deref(),
            Some("daily_life")
        );
    }

    #[test]
    fn version_one_import_keeps_words_unclassified() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_db(&connection).unwrap();
        let data: ExportData = serde_json::from_str(
            r#"{
                "version": 1,
                "exportedAt": "2026-10-04T00:00:00+00:00",
                "vocabulary": [{
                    "id": 1, "term": "travel", "normalizedTerm": "travel", "translation": "du lịch",
                    "example": null, "stability": null, "difficulty": null,
                    "dueAt": "2026-10-04T00:00:00+00:00", "lastReviewedAt": null,
                    "createdAt": "2026-10-01T00:00:00+00:00", "updatedAt": "2026-10-01T00:00:00+00:00"
                }],
                "reviewLogs": []
            }"#,
        )
        .unwrap();

        import_into_connection(&connection, data).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT topic FROM vocabulary WHERE term = 'travel'",
                    [],
                    |row| { row.get::<_, Option<String>>(0) }
                )
                .unwrap(),
            None
        );
    }

    #[test]
    fn import_rejects_unknown_version_without_writing() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_db(&connection).unwrap();
        let result = import_into_connection(
            &connection,
            ExportData {
                version: 3,
                exported_at: now(),
                vocabulary: Vec::new(),
                review_logs: Vec::new(),
            },
        );
        assert!(result.is_err());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM vocabulary", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn ipa_normalization_and_validation() {
        assert_eq!(
            normalize_ipa(Some(" /həˈloʊ/ ".into())).unwrap(),
            Some("/həˈloʊ/".into())
        );
        assert_eq!(normalize_ipa(Some("   ".into())).unwrap(), None);
        assert_eq!(normalize_ipa(None).unwrap(), None);
        // control char rejected
        assert!(normalize_ipa(Some("ipa\nvalue".into())).is_err());
        // > 200 chars rejected
        let long_ipa = "a".repeat(201);
        assert!(normalize_ipa(Some(long_ipa)).is_err());
    }

    #[test]
    fn assessment_validation() {
        let valid = PronunciationAssessment {
            score: 85,
            is_correct: true,
            feedback: "Phát âm tốt".into(),
            recognized_text: "hello".into(),
            segments: vec![PronunciationSegment {
                part: "hel".into(),
                status: "correct".into(),
                note: Some("Âm /h/ rõ".into()),
            }],
        };
        assert!(validate_assessment(valid).is_ok());

        let empty_feedback = PronunciationAssessment {
            score: 0,
            is_correct: false,
            feedback: "   ".into(),
            recognized_text: "hello".into(),
            segments: vec![],
        };
        assert!(validate_assessment(empty_feedback).is_err());

        let invalid_status = PronunciationAssessment {
            score: 50,
            is_correct: false,
            feedback: "Thử lại".into(),
            recognized_text: "test".into(),
            segments: vec![PronunciationSegment {
                part: "te".into(),
                status: "unknown".into(),
                note: None,
            }],
        };
        assert!(validate_assessment(invalid_status).is_err());
    }

    #[test]
    fn ipa_migration_and_export_import_roundtrip() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_db(&connection).unwrap();
        connection
            .execute(
                "INSERT INTO vocabulary(term, normalized_term, ipa, translation, example, topic, due_at, created_at, updated_at)
                 VALUES ('sound', 'sound', '/saʊnd/', 'âm thanh', 'Good sound.', 'entertainment', ?1, ?1, ?1)",
                ["2026-10-04T00:00:00+00:00"],
            )
            .unwrap();
        let export = export_from_connection(&connection).unwrap();
        assert_eq!(export.vocabulary.len(), 1);
        assert_eq!(export.vocabulary[0].ipa.as_deref(), Some("/saʊnd/"));

        let destination = Connection::open_in_memory().unwrap();
        initialize_db(&destination).unwrap();
        let result = import_into_connection(&destination, export).unwrap();
        assert_eq!(result.imported_words, 1);
        let imported_ipa: Option<String> = destination
            .query_row(
                "SELECT ipa FROM vocabulary WHERE term = 'sound'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(imported_ipa.as_deref(), Some("/saʊnd/"));
    }
}
