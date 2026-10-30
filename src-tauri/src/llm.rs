use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use crate::settings::{RoastLevel, Settings};

const FALLBACK_LINES: [&str; 10] = [
    "Boo. That was your productivity dying.",
    "I counted your tabs. They counted back.",
    "The void asked what you're working on. I lied.",
    "Your cursor blinked first. It knows.",
    "I haunt this corner until you stand up.",
    "That meeting could have been a scream.",
    "I licked the save button. You're welcome.",
    "Something's behind you. It's your backlog.",
    "I whispered your password to the moths.",
    "Coffee won't save you. I already drank it.",
];

const MAX_CHARS: usize = 220;
const MAX_SENTENCES: usize = 2;
const MAX_TOKENS: u32 = 80;
const TIMEOUT: Duration = Duration::from_secs(8);

static NEXT_LINE: AtomicUsize = AtomicUsize::new(0);

pub fn fallback_line() -> String {
    let i = NEXT_LINE.fetch_add(1, Ordering::Relaxed);
    FALLBACK_LINES[i % FALLBACK_LINES.len()].to_string()
}

/// First two sentences or 220 characters, whichever is shorter.
pub fn hard_cap(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let two = first_n_sentences(trimmed, MAX_SENTENCES);
    if two.chars().count() <= MAX_CHARS {
        two.to_string()
    } else {
        two.chars().take(MAX_CHARS).collect()
    }
}

pub async fn complete(
    settings: &Settings,
    trigger: &str,
    app_name: &str,
    bundle_id: &str,
) -> String {
    match request_joke(settings, trigger, app_name, bundle_id, local_hour()).await {
        Ok(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                fallback_line()
            } else {
                trimmed.to_string()
            }
        }
        Err(_) => fallback_line(),
    }
}

async fn request_joke(
    settings: &Settings,
    trigger: &str,
    app_name: &str,
    bundle_id: &str,
    hour: u32,
) -> Result<String, String> {
    let api_key = settings.api_key.trim();
    if api_key.is_empty() {
        return Err("missing api_key".into());
    }

    let body = chat_body(settings, trigger, app_name, bundle_id, hour);
    if cfg!(debug_assertions) {
        eprintln!("joke request: {body}");
    }

    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|err| err.to_string())?;
    let response = client
        .post(chat_url(&settings.base_url))
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await
        .map_err(|err| err.to_string())?;
    if !response.status().is_success() {
        return Err(format!("http {}", response.status()));
    }
    let parsed: Value = response.json().await.map_err(|err| err.to_string())?;
    content_from_response(&parsed).ok_or_else(|| "empty content".into())
}

fn chat_url(base_url: &str) -> String {
    format!("{}/chat/completions", base_url.trim().trim_end_matches('/'))
}

fn chat_body(
    settings: &Settings,
    trigger: &str,
    app_name: &str,
    bundle_id: &str,
    hour: u32,
) -> Value {
    json!({
        "model": settings.model,
        "max_tokens": MAX_TOKENS,
        "messages": [
            {"role": "system", "content": system_prompt(settings.profile.roast_level)},
            {"role": "user", "content": user_payload(settings, trigger, app_name, bundle_id, hour).to_string()}
        ]
    })
}

fn user_payload(
    settings: &Settings,
    trigger: &str,
    app_name: &str,
    bundle_id: &str,
    hour: u32,
) -> Value {
    json!({
        "name": settings.profile.name,
        "role": settings.profile.role,
        "interests": settings.profile.interests,
        "roast_level": settings.profile.roast_level,
        "do_not_joke": settings.profile.do_not_joke,
        "pet_name": settings.profile.pet_name,
        "app_name": app_name,
        "bundle_id": bundle_id,
        "trigger": trigger,
        "time_of_day": time_of_day(hour),
    })
}

fn system_prompt(roast: RoastLevel) -> String {
    let roast = match roast {
        RoastLevel::Gentle => "teasing",
        RoastLevel::Mean => "pointed",
        RoastLevel::Cursed => "absurdist / cute-horrific",
    };
    format!(
        "You are a spooky pet, not an assistant. One short joke or heckle. \
         Use the profile and the app. Stay in character. Do not offer help. \
         Do not mention policies, screenshots, or being an AI. \
         Obey the do-not-joke list. If the app is unknown, roast the human anyway. \
         Always one punchline. Never helpful. Roast style: {roast}."
    )
}

fn time_of_day(hour: u32) -> &'static str {
    match hour {
        5..=11 => "morning",
        12..=16 => "afternoon",
        17..=20 => "evening",
        _ => "night",
    }
}

fn local_hour() -> u32 {
    #[cfg(unix)]
    {
        unix_local_hour().clamp(0, 23)
    }
    #[cfg(not(unix))]
    {
        12
    }
}

#[cfg(unix)]
fn unix_local_hour() -> u32 {
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        tm_gmtoff: i64,
        tm_zone: *const i8,
    }

    unsafe extern "C" {
        fn time(tloc: *mut i64) -> i64;
        fn localtime_r(clock: *const i64, result: *mut Tm) -> *mut Tm;
    }

    unsafe {
        let mut t: i64 = 0;
        if time(&mut t) == -1 {
            return 12;
        }
        let mut tm = std::mem::MaybeUninit::<Tm>::uninit();
        if localtime_r(&t, tm.as_mut_ptr()).is_null() {
            return 12;
        }
        tm.assume_init().tm_hour.clamp(0, 23) as u32
    }
}

fn content_from_response(value: &Value) -> Option<String> {
    let text = value
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn first_n_sentences(text: &str, n: usize) -> &str {
    let mut counted = 0;
    for (i, ch) in text.char_indices() {
        if !matches!(ch, '.' | '!' | '?') {
            continue;
        }
        let next = i + ch.len_utf8();
        let at_end = next >= text.len();
        let next_is_space = text[next..]
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace());
        if at_end || next_is_space {
            counted += 1;
            if counted >= n {
                return text[..next].trim_end();
            }
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Profile;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration as StdDuration;

    const USER_FIELDS: [&str; 10] = [
        "name",
        "role",
        "interests",
        "roast_level",
        "do_not_joke",
        "pet_name",
        "app_name",
        "bundle_id",
        "trigger",
        "time_of_day",
    ];

    #[test]
    fn fallback_line_is_always_a_short_known_line() {
        for _ in 0..20 {
            let line = fallback_line();
            assert!(FALLBACK_LINES.contains(&line.as_str()));
            assert!(!line.is_empty());
            assert!(line.chars().count() <= MAX_CHARS);
        }
    }

    #[test]
    fn hard_cap_keeps_two_short_sentences() {
        assert_eq!(hard_cap("Boo. Hi."), "Boo. Hi.");
        assert_eq!(hard_cap("  Boo!  "), "Boo!");
        assert_eq!(hard_cap(""), "");
    }

    #[test]
    fn hard_cap_drops_a_third_sentence() {
        assert_eq!(hard_cap("One. Two. Three."), "One. Two.");
        assert_eq!(hard_cap("Wait! What? Nope."), "Wait! What?");
    }

    #[test]
    fn hard_cap_cuts_at_220_chars() {
        let long = "a".repeat(300);
        let capped = hard_cap(&long);
        assert_eq!(capped.chars().count(), MAX_CHARS);

        let two_long = format!("{}. {}.", "b".repeat(150), "c".repeat(150));
        let capped = hard_cap(&two_long);
        assert_eq!(capped.chars().count(), MAX_CHARS);
    }

    #[test]
    fn complete_returns_a_fallback_line_when_key_is_missing() {
        let settings = Settings::default();
        assert!(settings.api_key.is_empty());
        let text = tauri::async_runtime::block_on(complete(
            &settings,
            "click",
            "TextEdit",
            "com.apple.TextEdit",
        ));
        assert!(FALLBACK_LINES.contains(&text.as_str()));
        assert!(hard_cap(&text).chars().count() <= MAX_CHARS);
    }

    #[test]
    fn time_of_day_matches_a9_ranges() {
        for hour in 0..5 {
            assert_eq!(time_of_day(hour), "night", "hour {hour}");
        }
        for hour in 5..12 {
            assert_eq!(time_of_day(hour), "morning", "hour {hour}");
        }
        for hour in 12..17 {
            assert_eq!(time_of_day(hour), "afternoon", "hour {hour}");
        }
        for hour in 17..21 {
            assert_eq!(time_of_day(hour), "evening", "hour {hour}");
        }
        for hour in 21..24 {
            assert_eq!(time_of_day(hour), "night", "hour {hour}");
        }
    }

    #[test]
    fn roast_prompt_maps_a19() {
        assert!(system_prompt(RoastLevel::Gentle).contains("teasing"));
        assert!(system_prompt(RoastLevel::Mean).contains("pointed"));
        assert!(system_prompt(RoastLevel::Cursed).contains("absurdist / cute-horrific"));
        let prompt = system_prompt(RoastLevel::Gentle);
        assert!(prompt.contains("spooky pet"));
        assert!(prompt.contains("Do not offer help"));
        assert!(prompt.contains("Do not mention policies, screenshots, or being an AI"));
        assert!(prompt.contains("do-not-joke"));
        assert!(prompt.contains("If the app is unknown, roast the human anyway"));
        assert!(prompt.contains("Never helpful"));
    }

    #[test]
    fn user_payload_contains_only_allowed_fields() {
        let settings = Settings {
            api_key: "sk-secret-must-not-appear".into(),
            profile: Profile {
                name: "Dmytro".into(),
                role: "engineer".into(),
                interests: "cats".into(),
                roast_level: RoastLevel::Cursed,
                do_not_joke: vec!["meetings".into()],
                pet_name: "Bones".into(),
            },
            ..Settings::default()
        };
        let payload = user_payload(&settings, "click", "Slack", "com.tinyspeck.slackmacgap", 19);
        let obj = payload.as_object().expect("object");
        let mut keys: Vec<_> = obj.keys().cloned().collect();
        keys.sort();
        let mut expected: Vec<_> = USER_FIELDS.iter().map(|s| (*s).to_string()).collect();
        expected.sort();
        assert_eq!(keys, expected);
        assert_eq!(payload["app_name"], "Slack");
        assert_eq!(payload["bundle_id"], "com.tinyspeck.slackmacgap");
        assert_eq!(payload["trigger"], "click");
        assert_eq!(payload["time_of_day"], "evening");
        let raw = payload.to_string();
        assert!(!raw.contains("sk-secret"));
        assert!(!raw.contains("screenshot"));
        assert!(!raw.contains("clipboard"));
        assert!(!raw.contains("http://"));
        assert!(!raw.contains("https://"));
    }

    #[test]
    fn chat_body_is_system_plus_user_without_the_key() {
        let settings = Settings {
            api_key: "sk-secret-must-not-appear".into(),
            ..Settings::default()
        };
        let body = chat_body(&settings, "timer", "TextEdit", "com.apple.TextEdit", 9);
        assert_eq!(body["max_tokens"], MAX_TOKENS);
        assert_eq!(body["model"], settings.model);
        let messages = body["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user");
        let raw = body.to_string();
        assert!(!raw.contains("sk-secret"));
        assert!(raw.contains("timer"));
        assert!(raw.contains("TextEdit"));
        assert!(raw.contains("morning"));
    }

    #[test]
    fn complete_reads_model_content_from_chat_completions() {
        let joke = "The void filed a ticket about your tabs.";
        let (base_url, server) = spawn_completion(200, &chat_content(joke));
        let mut settings = Settings::default();
        settings.api_key = "sk-test".into();
        settings.base_url = base_url;
        let text = tauri::async_runtime::block_on(complete(
            &settings,
            "click",
            "Slack",
            "com.tinyspeck.slackmacgap",
        ));
        server.join().expect("server");
        assert_eq!(text, joke);
    }

    #[test]
    fn complete_falls_back_on_empty_content_and_http_error() {
        let (base_url, server) = spawn_completion(200, &chat_content("   "));
        let mut settings = Settings::default();
        settings.api_key = "sk-test".into();
        settings.base_url = base_url;
        let text = tauri::async_runtime::block_on(complete(&settings, "timer", "X", "y"));
        server.join().expect("server");
        assert!(FALLBACK_LINES.contains(&text.as_str()));

        let (base_url, server) = spawn_completion(500, "{\"error\":\"nope\"}");
        settings.base_url = base_url;
        let text = tauri::async_runtime::block_on(complete(&settings, "app_switch", "X", "y"));
        server.join().expect("server");
        assert!(FALLBACK_LINES.contains(&text.as_str()));
    }

    #[test]
    fn posted_json_has_only_privacy_fields_and_key_stays_in_header() {
        let (base_url, captured) = spawn_capture();
        let mut settings = Settings::default();
        settings.api_key = "sk-secret-header-only".into();
        settings.base_url = base_url;
        settings.profile.name = "Dmytro".into();
        let _ = tauri::async_runtime::block_on(complete(
            &settings,
            "app_switch",
            "Slack",
            "com.tinyspeck.slackmacgap",
        ));
        let request = captured.join().expect("capture");
        assert!(request.contains("POST /v1/chat/completions"));
        let request_l = request.to_ascii_lowercase();
        assert!(
            request_l.contains("authorization: bearer sk-secret-header-only"),
            "{request}"
        );
        let body = request.split("\r\n\r\n").nth(1).unwrap_or("");
        assert!(!body.contains("sk-secret-header-only"));
        let parsed: Value = serde_json::from_str(body).expect("json body");
        assert_eq!(parsed["max_tokens"], 80);
        let user: Value = serde_json::from_str(parsed["messages"][1]["content"].as_str().unwrap())
            .expect("user json");
        let mut keys: Vec<_> = user.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        let mut expected: Vec<_> = USER_FIELDS.iter().map(|s| (*s).to_string()).collect();
        expected.sort();
        assert_eq!(keys, expected);
    }

    fn chat_content(text: &str) -> String {
        json!({
            "choices": [{"message": {"content": text}}]
        })
        .to_string()
    }

    fn spawn_completion(status: u16, body: &str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let body = body.to_string();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let _ = read_http(&mut stream);
            let status_line = if status == 200 {
                "HTTP/1.1 200 OK"
            } else {
                "HTTP/1.1 500 Internal Server Error"
            };
            let resp = format!(
                "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        });
        (format!("http://{addr}/v1"), handle)
    }

    fn spawn_capture() -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_http(&mut stream);
            let body = "{\"choices\":[{\"message\":{\"content\":\"ok\"}}]}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            request
        });
        (format!("http://{addr}/v1"), handle)
    }

    fn read_http(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(StdDuration::from_secs(2)))
            .ok();
        let mut data = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    data.extend_from_slice(&buf[..n]);
                    if let Some(split) = find_headers_end(&data) {
                        let headers = &data[..split];
                        let header_text = String::from_utf8_lossy(headers);
                        let len = content_length(&header_text).unwrap_or(0);
                        if data.len() >= split + 4 + len {
                            break;
                        }
                    }
                }
                Err(_) => break,
            }
        }
        String::from_utf8_lossy(&data).into_owned()
    }

    fn find_headers_end(data: &[u8]) -> Option<usize> {
        data.windows(4).position(|w| w == b"\r\n\r\n")
    }

    fn content_length(headers: &str) -> Option<usize> {
        headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse().ok()
            } else {
                None
            }
        })
    }
}
