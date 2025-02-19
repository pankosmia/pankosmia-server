#[macro_use]
#[cfg(test)]
mod tests;

use copy_dir::copy_dir;
use git2::{Repository, StatusOptions};
use hallomai::transform;
use home::home_dir;
use rocket::form::Form;
use rocket::fs::{relative, FileServer, TempFile};
use rocket::http::{ContentType, Status};
use rocket::response::{status, stream, Redirect};
use rocket::tokio::time;
use rocket::{catch, catchers, get, post, routes, Build, FromForm, Request, Rocket, State, Responder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, VecDeque};
use std::io::Write;
use std::path::{Components, Path, PathBuf};
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{env, fs};
use ureq;

// STRUCTS

#[derive(Serialize, Deserialize, Clone)]
struct AuthToken {
    service: String,
    token: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Bcv {
    book_code: String,
    chapter: u16,
    verse: u16,
}

#[derive(Serialize, Deserialize, Clone)]
struct Typography {
    font_set: String,
    size: String,
    direction: String,
}

#[derive(Serialize, Deserialize)]
struct AppSettings {
    working_dir: String,
    repo_dir: Mutex<String>,
    languages: Mutex<Vec<String>>,
    gitea_endpoints: BTreeMap<String, String>,
    auth_tokens: Mutex<BTreeMap<String, String>>,
    bcv: Mutex<Bcv>,
    typography: Mutex<Typography>,
}

#[derive(Serialize, Deserialize)]
struct JsonDataResponse {
    is_good: bool,
    reason: String,
}
#[derive(Serialize, Deserialize)]
struct JsonNetStatusResponse {
    is_enabled: bool,
}

#[derive(Serialize, Deserialize)]
struct RemoteRepoRecord {
    name: String,
    abbreviation: String,
    description: String,
    avatar_url: String,
    flavor: String,
    flavor_type: String,
    language_code: String,
    script_direction: String,
    branch_or_tag: String,
    clone_url: String,
}

#[derive(Serialize, Deserialize)]
struct GitStatusRecord {
    path: String,
    change_type: String,
}

#[derive(Serialize, Deserialize)]
struct MetadataSummary {
    name: String,
    description: String,
    flavor_type: String,
    flavor: String,
    language_code: String,
    script_direction: String,
}

#[derive(FromForm)]
struct Upload<'f> {
    file: TempFile<'f>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Client {
    id: String,
    requires: BTreeMap<String, bool>,
    exclude_from_menu: bool,
    exclude_from_dashboard: bool,
    path: String,
    url: String,
}

#[derive(Serialize)]
struct PublicClient {
    id: String,
    requires: BTreeMap<String, bool>,
    exclude_from_menu: bool,
    exclude_from_dashboard: bool,
    url: String,
}

// CONSTANTS AND STATE

static NET_IS_ENABLED: AtomicBool = AtomicBool::new(false);
static DEBUG_IS_ENABLED: AtomicBool = AtomicBool::new(false);

// UTILITY FUNCTIONS

fn os_slash_str() -> &'static str {
    match env::consts::OS {
        "windows" => "\\",
        _ => "/",
    }
}

fn maybe_os_quoted_path_str(s: String) -> String {
    match env::consts::OS {
        "windows" => s.replace("\\", "\\\\").replace("/", "\\\\"),
        _ => s,
    }
}

fn forbidden_path_strings() -> Vec<String> {
    Vec::from([
        "..".to_string(),
        "~".to_string(),
        "/".to_string(),
        "\\".to_string(),
        "&".to_string(),
        "*".to_string(),
        "+".to_string(),
        "|".to_string(),
        " ".to_string(),
        "?".to_string(),
        "#".to_string(),
        "%".to_string(),
        "{".to_string(),
        "}".to_string(),
        "<".to_string(),
        ">".to_string(),
        "$".to_string(),
        "!".to_string(),
        "'".to_string(),
        "\"".to_string(),
        ":".to_string(),
        ";".to_string(),
        "`".to_string(),
        "=".to_string(),
    ])
}

fn mime_types() -> BTreeMap<String, ContentType> {
    BTreeMap::from([
        ("aac".to_string(), ContentType::AAC),
        ("bin".to_string(), ContentType::Binary),
        ("bmp".to_string(), ContentType::BMP),
        ("css".to_string(), ContentType::CSS),
        ("csv".to_string(), ContentType::CSV),
        ("flac".to_string(), ContentType::FLAC),
        ("gif".to_string(), ContentType::GIF),
        ("gz".to_string(), ContentType::GZIP),
        ("htm".to_string(), ContentType::HTML),
        ("html".to_string(), ContentType::HTML),
        ("ico".to_string(), ContentType::Icon),
        ("ics".to_string(), ContentType::Calendar),
        ("jpeg".to_string(), ContentType::JPEG),
        ("jpg".to_string(), ContentType::JPEG),
        ("js".to_string(), ContentType::JavaScript),
        ("md".to_string(), ContentType::Markdown),
        ("mov".to_string(), ContentType::MOV),
        ("mp4".to_string(), ContentType::MP4),
        ("mpeg".to_string(), ContentType::MPEG),
        ("mpeg4".to_string(), ContentType::MP4),
        ("mpg".to_string(), ContentType::MPEG),
        ("ogg".to_string(), ContentType::OGG),
        ("ogv".to_string(), ContentType::OGG),
        ("otf".to_string(), ContentType::OTF),
        ("pdf".to_string(), ContentType::PDF),
        ("png".to_string(), ContentType::PNG),
        ("svg".to_string(), ContentType::SVG),
        ("tar".to_string(), ContentType::TAR),
        ("tif".to_string(), ContentType::TIFF),
        ("tiff".to_string(), ContentType::TIFF),
        ("tsv".to_string(), ContentType::CSV),
        ("ttf".to_string(), ContentType::TTF),
        ("txt".to_string(), ContentType::Plain),
        ("usfm".to_string(), ContentType::Plain),
        ("usj".to_string(), ContentType::JSON),
        ("usx".to_string(), ContentType::XML),
        ("vrs".to_string(), ContentType::Plain),
        ("wasm".to_string(), ContentType::WASM),
        ("wav".to_string(), ContentType::WAV),
        ("weba".to_string(), ContentType::WEBA),
        ("webm".to_string(), ContentType::WEBM),
        ("webp".to_string(), ContentType::WEBP),
        ("woff".to_string(), ContentType::WOFF),
        ("woff2".to_string(), ContentType::WOFF2),
        ("xml".to_string(), ContentType::XML),
        ("zip".to_string(), ContentType::ZIP),
    ])
}

fn check_path_components(path_components: &mut Components<'_>) -> bool {
    let mut ret = true;
    if path_components.clone().collect::<Vec<_>>().len() < 3 {
        return false;
    }
    for path_component in path_components {
        let path_string = path_component
            .clone()
            .as_os_str()
            .to_str()
            .unwrap()
            .to_string();
        if path_string.starts_with(".") {
            return false;
        }
        for forbidden_string in forbidden_path_strings() {
            if path_string.contains(&forbidden_string) {
                ret = false;
                break;
            }
        }
    }
    ret
}
fn check_path_string_components(path_string: String) -> bool {
    let mut ret = true;
    if path_string.starts_with("/") {
        return false;
    }
    let path_string_parts = path_string.split("/");
    for path_string_part in path_string_parts {
        if path_string_part.len() < 2 {
            return false;
        }
        if path_string_part.starts_with(".") {
            return false;
        }
        for forbidden_string in forbidden_path_strings() {
            if path_string_part.contains(&forbidden_string) {
                ret = false;
                break;
            }
        }
    }
    ret
}

fn make_json_data_response(is_good: bool, reason: String) -> String {
    let jr: JsonDataResponse = JsonDataResponse { is_good, reason };
    serde_json::to_string(&jr).unwrap()
}

fn make_net_status_response(is_enabled: bool) -> String {
    let nsr: JsonNetStatusResponse = JsonNetStatusResponse { is_enabled };
    serde_json::to_string(&nsr).unwrap()
}
fn make_bad_json_data_response(reason: String) -> String {
    make_json_data_response(false, reason)
}

fn make_good_json_data_response(reason: String) -> String {
    make_json_data_response(true, reason)
}
fn home_dir_string() -> String {
    home_dir()
        .unwrap()
        .as_os_str()
        .to_str()
        .unwrap()
        .to_string()
}

// SETTINGS
#[get("/languages")]
fn get_languages(state: &State<AppSettings>) -> status::Custom<(ContentType, String)> {
    let languages = state.languages.lock().unwrap().clone();
    match serde_json::to_string(&languages) {
        Ok(v) => status::Custom(Status::Ok, (ContentType::JSON, v)),
        Err(e) => status::Custom(
            Status::InternalServerError,
            (
                ContentType::JSON,
                make_bad_json_data_response(format!(
                    "Could not parse language settings as JSON array: {}",
                    e
                )),
            ),
        ),
    }
}
#[get("/auth-token/<token_key>")]
fn get_auth_token(
    state: &State<AppSettings>,
    token_key: String,
) -> status::Custom<(ContentType, String)> {
    if !state.gitea_endpoints.contains_key(&token_key) {
        return status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response(format!(
                    "Unknown GITEA endpoint name: {}",
                    token_key
                )),
            ),
        );
    }
    let auth_tokens = state
        .auth_tokens
        .lock()
        .unwrap()
        .clone();
    if auth_tokens.contains_key(&token_key) {
        let code = &auth_tokens[&token_key];
        let ok_json = json!({"is_good": true, "code": code});
        match serde_json::to_string(&ok_json) {
            Ok(v) => status::Custom(
                Status::Ok,
                (
                    ContentType::JSON,
                    v
                ),
            ),
            Err(e) => status::Custom(
                Status::InternalServerError,
                (
                    ContentType::JSON,
                    make_bad_json_data_response(format!(
                        "Could not produce JSON for auth token: {}",
                        e
                    )),
                ),
            ),
        }
    } else {
        status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response(format!(
                    "Could not find record for token key '{}'",
                    token_key
                )),
            ),
        )
    }
}

#[derive(Responder)]
enum ContentOrRedirect {
    Content(status::Custom<(ContentType, String)>),
    Redirect(Redirect),
}
#[get("/auth-token/<token_key>?<code>")]
fn get_new_auth_token(
    state: &State<AppSettings>,
    token_key: String,
    code: String,
) -> ContentOrRedirect {
    if !state.gitea_endpoints.contains_key(&token_key) {
        return ContentOrRedirect::Content(
            status::Custom(
                Status::BadRequest,
                (
                    ContentType::JSON,
                    make_bad_json_data_response(format!(
                        "Unknown GITEA endpoint name: {}",
                        token_key
                    )),
                ),
            )
        );
    }
    let mut tokens_inner = state
        .auth_tokens
        .lock()
        .unwrap();
    if code == "" {
        tokens_inner.remove(&token_key);
    } else {
        tokens_inner.insert(token_key, code);
    }
    ContentOrRedirect::Redirect(Redirect::to("/clients/main"))
}

#[get("/typography")]
fn get_typography(state: &State<AppSettings>) -> status::Custom<(ContentType, String)> {
    let typography = state.typography.lock().unwrap().clone();
    match serde_json::to_string(&typography) {
        Ok(v) => status::Custom(Status::Ok, (ContentType::JSON, v)),
        Err(e) => status::Custom(
            Status::InternalServerError,
            (
                ContentType::JSON,
                make_bad_json_data_response(format!(
                    "Could not parse typography settings as JSON object: {}",
                    e
                )),
            ),
        ),
    }
}

#[post("/typography/<font_set>/<size>/<direction>")]
fn post_typography(
    state: &State<AppSettings>,
    font_set: &str,
    size: &str,
    direction: &str,
) -> status::Custom<(ContentType, String)> {
    *state.typography.lock().unwrap() = Typography {
        font_set: font_set.to_string(),
        size: size.to_string(),
        direction: direction.to_string(),
    };
    status::Custom(
        Status::Ok,
        (
            ContentType::JSON,
            make_good_json_data_response("ok".to_string()),
        ),
    )
}

// NETWORK OPERATIONS
#[get("/status")]
fn net_status() -> status::Custom<(ContentType, String)> {
    status::Custom(
        Status::Ok,
        (
            ContentType::JSON,
            make_net_status_response(NET_IS_ENABLED.load(Ordering::Relaxed)),
        ),
    )
}

#[get("/enable")]
fn net_enable(msgs: &State<MsgQueue>) -> status::Custom<(ContentType, String)> {
    msgs.lock()
        .unwrap()
        .push_back("info--5--net--enable".to_string());
    NET_IS_ENABLED.store(true, Ordering::Relaxed);
    status::Custom(
        Status::Ok,
        (
            ContentType::JSON,
            make_good_json_data_response("ok".to_string()),
        ),
    )
}

#[get("/disable")]
fn net_disable(msgs: &State<MsgQueue>) -> status::Custom<(ContentType, String)> {
    msgs.lock()
        .unwrap()
        .push_back("info--5--net--disable".to_string());
    NET_IS_ENABLED.store(false, Ordering::Relaxed);
    status::Custom(
        Status::Ok,
        (
            ContentType::JSON,
            make_good_json_data_response("ok".to_string()),
        ),
    )
}

// DEBUG OPERATIONS
#[get("/status")]
fn debug_status() -> status::Custom<(ContentType, String)> {
    status::Custom(
        Status::Ok,
        (
            ContentType::JSON,
            make_net_status_response(DEBUG_IS_ENABLED.load(Ordering::Relaxed)),
        ),
    )
}

#[get("/enable")]
fn debug_enable(msgs: &State<MsgQueue>) -> status::Custom<(ContentType, String)> {
    msgs.lock()
        .unwrap()
        .push_back("info--5--debug--enable".to_string());
    DEBUG_IS_ENABLED.store(true, Ordering::Relaxed);
    status::Custom(
        Status::Ok,
        (
            ContentType::JSON,
            make_good_json_data_response("ok".to_string()),
        ),
    )
}

#[get("/disable")]
fn debug_disable(msgs: &State<MsgQueue>) -> status::Custom<(ContentType, String)> {
    msgs.lock()
        .unwrap()
        .push_back("info--5--debug--disable".to_string());
    DEBUG_IS_ENABLED.store(false, Ordering::Relaxed);
    status::Custom(
        Status::Ok,
        (
            ContentType::JSON,
            make_good_json_data_response("ok".to_string()),
        ),
    )
}

// SSE
#[get("/")]
async fn notifications_stream<'a>(
    msgs: &'a State<MsgQueue>,
    state: &'a State<AppSettings>,
) -> stream::EventStream![stream::Event + 'a] {
    stream::EventStream! {
        let mut count = 0;
        let mut interval = time::interval(Duration::from_millis(500));
        yield stream::Event::retry(Duration::from_secs(1));
        loop {
            while !msgs.lock().unwrap().is_empty() {
                let msg = msgs.lock().unwrap().pop_front().unwrap();
                yield stream::Event::data(msg)
                    .event("misc")
                    .id(format!("{}", count));
                count+=1;
                interval.tick().await;
            };
            yield stream::Event::data(
                    match NET_IS_ENABLED.load(Ordering::Relaxed) {
                        true => "enabled",
                        false => "disabled"
                    }
            )
            .event("net_status")
            .id(format!("{}", count));
            count+=1;
            yield stream::Event::data(
                    match DEBUG_IS_ENABLED.load(Ordering::Relaxed) {
                        true => "enabled",
                        false => "disabled"
                    }
            )
            .event("debug")
            .id(format!("{}", count));
            count+=1;
            let bcv = state.bcv.lock().unwrap().clone();
            yield stream::Event::data(
                format!("{}--{}--{}", bcv.book_code, bcv.chapter, bcv.verse)
            )
            .event("bcv")
            .id(format!("{}", count));
            count+=1;
            let typography = state.typography.lock().unwrap().clone();
            yield stream::Event::data(
                format!("{}--{}--{}", typography.font_set, typography.size, typography.direction)
            )
            .event("typography")
            .id(format!("{}", count));
            count+=1;
            let gitea_endpoints = state.gitea_endpoints.clone();
            let auth_tokens = state.auth_tokens.lock().unwrap().clone();
            for (ep_name, ep_endpoint) in gitea_endpoints {
                yield stream::Event::data(
                    format!("{}--{}--{}", ep_name, ep_endpoint, auth_tokens.contains_key(&ep_name))
                )
                .event("auth")
                .id(format!("{}", count));
                count+=1;
            }
            interval.tick().await;
        }
    }
}

// i18n

#[get("/raw")]
async fn raw_i18n(state: &State<AppSettings>) -> status::Custom<(ContentType, String)> {
    let path_to_serve = state.working_dir.clone() + os_slash_str() + "i18n.json";
    match fs::read_to_string(path_to_serve) {
        Ok(v) => status::Custom(Status::Ok, (ContentType::JSON, v)),
        Err(e) => status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response(format!("could not read raw i18n: {}", e).to_string()),
            ),
        ),
    }
}

#[get("/negotiated/<filter..>")]
async fn negotiated_i18n(
    state: &State<AppSettings>,
    filter: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let path_to_serve = state.working_dir.clone() + os_slash_str() + "i18n.json";
    let filter_items: Vec<String> = filter
        .display()
        .to_string()
        .split('/')
        .map(String::from)
        .collect();
    if filter_items.len() > 2 {
        return status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response(
                    format!("expected 0 - 2 filter terms, not {}", filter_items.len()).to_string(),
                ),
            ),
        );
    }
    let mut type_filter: Option<String> = None;
    let mut subtype_filter: Option<String> = None;
    if filter_items.len() > 0 && filter_items[0] != "" {
        type_filter = Some(filter_items[0].clone());
        if filter_items.len() > 1 && filter_items[1] != "" {
            subtype_filter = Some(filter_items[1].clone());
        }
    }
    match fs::read_to_string(path_to_serve) {
        Ok(v) => {
            match serde_json::from_str::<Value>(v.as_str()) {
                Ok(sj) => {
                    let languages = state.languages.lock().unwrap().clone();
                    let mut negotiated = Map::new();
                    for (i18n_type, subtypes) in sj.as_object().unwrap() {
                        // println!("{}", i18n_type);
                        match type_filter.clone() {
                            Some(v) => {
                                if v != *i18n_type {
                                    continue;
                                }
                            }
                            None => {}
                        }
                        let mut negotiated_types = Map::new();
                        for (i18n_subtype, terms) in subtypes.as_object().unwrap() {
                            // println!("   {}", i18n_subtype);
                            match subtype_filter.clone() {
                                Some(v) => {
                                    if v != *i18n_subtype {
                                        continue;
                                    }
                                }
                                None => {}
                            }
                            let mut negotiated_terms = Map::new();
                            for (i18n_term, term_languages) in terms.as_object().unwrap() {
                                // println!("      {}", i18n_term);
                                let mut negotiated_translations = Map::new();
                                'user_lang: for user_language in languages.clone() {
                                    for (i18n_language, translation) in
                                        term_languages.as_object().unwrap()
                                    {
                                        // println!("{} {}", i18n_language, languages[0]);
                                        if *i18n_language == user_language {
                                            negotiated_translations.insert(
                                                "language".to_string(),
                                                Value::String(i18n_language.clone()),
                                            );
                                            negotiated_translations.insert(
                                                "translation".to_string(),
                                                translation.clone(),
                                            );
                                            break 'user_lang;
                                        }
                                    }
                                }
                                negotiated_terms.insert(
                                    i18n_term.clone(),
                                    Value::Object(negotiated_translations),
                                );
                            }
                            negotiated_types
                                .insert(i18n_subtype.clone(), Value::Object(negotiated_terms));
                        }
                        negotiated.insert(i18n_type.clone(), Value::Object(negotiated_types));
                    }
                    status::Custom(
                        Status::Ok,
                        (
                            ContentType::JSON,
                            serde_json::to_string(&negotiated).unwrap(),
                        ),
                    )
                }
                Err(e) => status::Custom(
                    Status::BadRequest,
                    (
                        ContentType::JSON,
                        make_bad_json_data_response(
                            format!("could not parse for negotiated i18n: {}", e).to_string(),
                        ),
                    ),
                ),
            }
        }
        Err(e) => status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response(
                    format!("could not read for negotiated i18n: {}", e).to_string(),
                ),
            ),
        ),
    }
}

#[get("/flat/<filter..>")]
async fn flat_i18n(
    state: &State<AppSettings>,
    filter: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let path_to_serve = state.working_dir.clone() + os_slash_str() + "i18n.json";
    let filter_items: Vec<String> = filter
        .display()
        .to_string()
        .split('/')
        .map(String::from)
        .collect();
    if filter_items.len() > 2 {
        return status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response(
                    format!("expected 0 - 2 filter terms, not {}", filter_items.len()).to_string(),
                ),
            ),
        );
    }
    let mut type_filter: Option<String> = None;
    let mut subtype_filter: Option<String> = None;
    if filter_items.len() > 0 && filter_items[0] != "" {
        type_filter = Some(filter_items[0].clone());
        if filter_items.len() > 1 && filter_items[1] != "" {
            subtype_filter = Some(filter_items[1].clone());
        }
    }
    match fs::read_to_string(path_to_serve) {
        Ok(v) => {
            match serde_json::from_str::<Value>(v.as_str()) {
                Ok(sj) => {
                    let languages = state.languages.lock().unwrap().clone();
                    let mut flat = Map::new();
                    for (i18n_type, subtypes) in sj.as_object().unwrap() {
                        // println!("{}", i18n_type);
                        match type_filter.clone() {
                            Some(v) => {
                                if v != *i18n_type {
                                    continue;
                                }
                            }
                            None => {}
                        }
                        for (i18n_subtype, terms) in subtypes.as_object().unwrap() {
                            // println!("   {}", i18n_subtype);
                            match subtype_filter.clone() {
                                Some(v) => {
                                    if v != *i18n_subtype {
                                        continue;
                                    }
                                }
                                None => {}
                            }
                            for (i18n_term, term_languages) in terms.as_object().unwrap() {
                                'user_lang: for user_language in languages.clone() {
                                    for (i18n_language, translation) in
                                        term_languages.as_object().unwrap()
                                    {
                                        // println!("{} {}", i18n_language, languages[0]);
                                        if *i18n_language == user_language {
                                            let flat_key = format!(
                                                "{}:{}:{}",
                                                i18n_type.clone(),
                                                i18n_subtype.clone(),
                                                i18n_term.clone()
                                            );
                                            flat.insert(flat_key, translation.clone());
                                            break 'user_lang;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    status::Custom(
                        Status::Ok,
                        (ContentType::JSON, serde_json::to_string(&flat).unwrap()),
                    )
                }
                Err(e) => status::Custom(
                    Status::BadRequest,
                    (
                        ContentType::JSON,
                        make_bad_json_data_response(
                            format!("could not parse for flat i18n: {}", e).to_string(),
                        ),
                    ),
                ),
            }
        }
        Err(e) => status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response(
                    format!("could not read for flat i18n: {}", e).to_string(),
                ),
            ),
        ),
    }
}

#[get("/untranslated/<lang>")]
async fn untranslated_i18n(
    state: &State<AppSettings>,
    lang: String,
) -> status::Custom<(ContentType, String)> {
    let path_to_serve = state.working_dir.clone() + os_slash_str() + "i18n.json";
    match fs::read_to_string(path_to_serve) {
        Ok(v) => {
            match serde_json::from_str::<Value>(v.as_str()) {
                Ok(sj) => {
                    let mut untranslated: Vec<String> = Vec::new();
                    for (i18n_type, subtypes) in sj.as_object().unwrap() {
                        // println!("{}", i18n_type);
                        for (i18n_subtype, terms) in subtypes.as_object().unwrap() {
                            // println!("   {}", i18n_subtype);
                            for (i18n_term, term_languages) in terms.as_object().unwrap() {
                                // println!("      {}", i18n_term);
                                if !term_languages
                                    .as_object()
                                    .unwrap()
                                    .contains_key(lang.as_str())
                                {
                                    let flat_key = format!(
                                        "{}:{}:{}",
                                        i18n_type.clone(),
                                        i18n_subtype.clone(),
                                        i18n_term.clone()
                                    );
                                    untranslated.push(flat_key);
                                }
                            }
                        }
                    }
                    status::Custom(
                        Status::Ok,
                        (
                            ContentType::JSON,
                            serde_json::to_string(&untranslated).unwrap(),
                        ),
                    )
                }
                Err(e) => status::Custom(
                    Status::BadRequest,
                    (
                        ContentType::JSON,
                        make_bad_json_data_response(
                            format!("could not parse for untranslated i18n: {}", e).to_string(),
                        ),
                    ),
                ),
            }
        }
        Err(e) => status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response(
                    format!("could not read for untranslated i18n: {}", e).to_string(),
                ),
            ),
        ),
    }
}

// NAVIGATION
#[get("/bcv")]
fn get_bcv(state: &State<AppSettings>) -> status::Custom<(ContentType, String)> {
    let bcv = state.bcv.lock().unwrap().clone();
    match serde_json::to_string(&bcv) {
        Ok(v) => status::Custom(Status::Ok, (ContentType::JSON, v)),
        Err(e) => status::Custom(
            Status::InternalServerError,
            (
                ContentType::JSON,
                make_bad_json_data_response(format!(
                    "Could not parse bcv state as JSON object: {}",
                    e
                )),
            ),
        ),
    }
}

#[post("/bcv/<book_code>/<chapter>/<verse>")]
fn post_bcv(
    state: &State<AppSettings>,
    book_code: &str,
    chapter: u16,
    verse: u16,
) -> status::Custom<(ContentType, String)> {
    *state.bcv.lock().unwrap() = Bcv {
        book_code: book_code.to_string(),
        chapter,
        verse,
    };
    status::Custom(
        Status::Ok,
        (
            ContentType::JSON,
            make_good_json_data_response("ok".to_string()),
        ),
    )
}

// GITEA

#[get("/endpoints")]
fn get_gitea_endpoints(state: &State<AppSettings>) -> status::Custom<(ContentType, String)> {
    status::Custom(
        Status::Ok,
        (ContentType::JSON, serde_json::to_string(&state.gitea_endpoints).unwrap()),
    )
}

#[get("/remote-repos/<gitea_server>/<gitea_org>")]
fn gitea_remote_repos(
    gitea_server: &str,
    gitea_org: &str,
) -> status::Custom<(ContentType, String)> {
    if !NET_IS_ENABLED.load(Ordering::Relaxed) {
        return status::Custom(
            Status::Unauthorized,
            (
                ContentType::JSON,
                make_bad_json_data_response("offline mode".to_string()),
            ),
        );
    }
    let gitea_path = format!("https://{}/api/v1/orgs/{}/repos", gitea_server, gitea_org);
    match ureq::get(gitea_path.as_str()).call() {
        Ok(r) => match r.into_json::<Value>() {
            Ok(j) => {
                let mut records: Vec<RemoteRepoRecord> = Vec::new();
                for json_record in j.as_array().unwrap() {
                    let latest = &json_record["catalog"]["latest"];
                    records.push(RemoteRepoRecord {
                        name: json_record["name"].as_str().unwrap().to_string(),
                        abbreviation: json_record["abbreviation"].as_str().unwrap().to_string(),
                        description: json_record["description"].as_str().unwrap().to_string(),
                        avatar_url: json_record["avatar_url"].as_str().unwrap().to_string(),
                        flavor: json_record["flavor"].as_str().unwrap().to_string(),
                        flavor_type: json_record["flavor_type"].as_str().unwrap().to_string(),
                        language_code: json_record["language"].as_str().unwrap().to_string(),
                        script_direction: json_record["language_direction"]
                            .as_str()
                            .unwrap()
                            .to_string(),
                        branch_or_tag: match latest["branch_or_tag_name"].as_str() {
                            Some(s) => s.to_string(),
                            _ => "".to_string(),
                        },
                        clone_url: match latest["released"].as_str() {
                            Some(s) => s.to_string(),
                            _ => "".to_string(),
                        },
                    });
                }
                status::Custom(
                    Status::Ok,
                    (ContentType::JSON, serde_json::to_string(&records).unwrap()),
                )
            }
            Err(e) => {
                return status::Custom(
                    Status::InternalServerError,
                    (
                        ContentType::JSON,
                        make_bad_json_data_response(format!(
                            "could not serve GITEA server response as JSON string: {}",
                            e
                        )),
                    ),
                )
            }
        },
        Err(e) => status::Custom(
            Status::BadGateway,
            (
                ContentType::JSON,
                make_bad_json_data_response(
                    format!("could not read from GITEA server: {}", e).to_string(),
                ),
            ),
        ),
    }
}

// REPO OPERATIONS

#[get("/list-local-repos")]
fn list_local_repos(state: &State<AppSettings>) -> status::Custom<(ContentType, String)> {
    let root_path = state.repo_dir.lock().unwrap().clone();
    let server_paths = fs::read_dir(root_path).unwrap();
    let mut repos: Vec<String> = Vec::new();
    for server_path in server_paths {
        let uw_server_path_ob = server_path.unwrap().path();
        let uw_server_path_ob2 = uw_server_path_ob.clone();
        let server_leaf = uw_server_path_ob2.file_name().unwrap();
        for org_path in fs::read_dir(uw_server_path_ob).unwrap() {
            let uw_org_path_ob = org_path.unwrap().path();
            let uw_org_path_ob2 = uw_org_path_ob.clone();
            let org_leaf = uw_org_path_ob2.file_name().unwrap();
            for repo_path in fs::read_dir(uw_org_path_ob).unwrap() {
                let uw_repo_path_ob = repo_path.unwrap().path();
                let repo_leaf = uw_repo_path_ob.file_name().unwrap();
                let repo_url_string = format!(
                    "{}/{}/{}",
                    server_leaf.to_str().unwrap(),
                    org_leaf.to_str().unwrap(),
                    repo_leaf.to_str().unwrap()
                );
                repos.push(repo_url_string);
            }
        }
    }
    let quoted_repos: Vec<String> = repos
        .into_iter()
        .map(|str: String| format!("{}", str))
        .collect();
    status::Custom(
        Status::Ok,
        (
            ContentType::JSON,
            serde_json::to_string(&quoted_repos).unwrap(),
        ),
    )
}

#[get("/add-and-commit/<repo_path..>")]
async fn add_and_commit(
    state: &State<AppSettings>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let repo_path_string: String = state.repo_dir.lock().unwrap().clone()
        + os_slash_str()
        + &repo_path.display().to_string().clone();
    let result = match Repository::open(repo_path_string) {
        Ok(repo) => {
            repo.index()
                .unwrap()
                .add_all(&["."], git2::IndexAddOption::DEFAULT, None)
                .unwrap();
            repo.index().unwrap().write().unwrap();
            let mut index = repo.index().unwrap();
            let oid = index.write_tree().unwrap();
            let signature = repo.signature().unwrap();
            let parent_commit = repo.head().unwrap().peel_to_commit().unwrap();
            let tree = repo.find_tree(oid).unwrap();
            repo.commit(
                Some("HEAD"),
                &signature,
                &signature,
                "Updated by Pithekos",
                &tree,
                &[&parent_commit],
            )
                .unwrap();
            status::Custom(
                Status::Ok,
                (
                    ContentType::JSON,
                    make_good_json_data_response("ok".to_string()),
                ),
            )
        }
        Err(e) => status::Custom(
            Status::InternalServerError,
            (
                ContentType::JSON,
                make_bad_json_data_response(
                    format!("could not add/commit repo: {}", e).to_string(),
                ),
            ),
        ),
    };
    result
}
#[get("/fetch-repo/<repo_path..>")]
async fn fetch_repo(
    state: &State<AppSettings>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    if !NET_IS_ENABLED.load(Ordering::Relaxed) {
        return status::Custom(
            Status::Unauthorized,
            (
                ContentType::JSON,
                make_bad_json_data_response("offline mode".to_string()),
            ),
        );
    }
    let mut path_components: Components<'_> = repo_path.components();
    if check_path_components(&mut path_components.clone()) {
        let source = path_components
            .next()
            .unwrap()
            .as_os_str()
            .to_str()
            .unwrap();
        let org = path_components
            .next()
            .unwrap()
            .as_os_str()
            .to_str()
            .unwrap();
        let mut repo = path_components
            .next()
            .unwrap()
            .as_os_str()
            .to_str()
            .unwrap()
            .to_string();
        if repo.ends_with(".git") {
            let repo_vec = repo.split(".").collect::<Vec<&str>>();
            let short_repo = &repo_vec[0..repo_vec.len() - 1];
            let short_repo_string = short_repo.join("/");
            repo = short_repo_string.as_str().to_owned();
        }
        let url = "https://".to_string() + &repo_path.display().to_string().replace("\\", "/");
        match Repository::clone(
            &url,
            state.repo_dir.lock().unwrap().clone()
                + os_slash_str()
                + source
                + os_slash_str()
                + org
                + os_slash_str()
                + repo.as_str(),
        ) {
            Ok(_repo) => status::Custom(
                Status::Ok,
                (
                    ContentType::JSON,
                    make_good_json_data_response("ok".to_string()),
                ),
            ),
            Err(e) => {
                println!("Error:{}", e);
                return status::Custom(
                    Status::BadRequest,
                    (
                        ContentType::JSON,
                        make_bad_json_data_response(
                            format!("could not clone repo: {}", e).to_string(),
                        ),
                    ),
                );
            }
        }
    } else {
        status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response("bad repo path".to_string()),
            ),
        )
    }
}

#[get("/delete-repo/<repo_path..>")]
async fn delete_repo(
    state: &State<AppSettings>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let path_components: Components<'_> = repo_path.components();
    if check_path_components(&mut path_components.clone()) {
        let path_to_delete = state.repo_dir.lock().unwrap().clone()
            + os_slash_str()
            + &repo_path.display().to_string();
        match fs::remove_dir_all(path_to_delete) {
            Ok(_) => status::Custom(
                Status::Ok,
                (
                    ContentType::JSON,
                    make_good_json_data_response("ok".to_string()),
                ),
            ),
            Err(e) => status::Custom(
                Status::BadRequest,
                (
                    ContentType::JSON,
                    make_bad_json_data_response(
                        format!("could not delete repo: {}", e).to_string(),
                    ),
                ),
            ),
        }
    } else {
        status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response("bad repo path".to_string()),
            ),
        )
    }
}

#[get("/status/<repo_path..>")]
async fn git_status(
    state: &State<AppSettings>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let repo_path_string: String = state.repo_dir.lock().unwrap().clone()
        + os_slash_str()
        + &repo_path.display().to_string().clone();
    match Repository::open(repo_path_string) {
        Ok(repo) => {
            if repo.is_bare() {
                return status::Custom(
                    Status::BadRequest,
                    (
                        ContentType::JSON,
                        make_bad_json_data_response("cannot get status of bare repo".to_string()),
                    ),
                );
            };
            let mut opts = StatusOptions::new();
            opts.include_untracked(true);
            match repo.statuses(Some(&mut opts)) {
                Ok(statuses) => {
                    let mut status_changes = Vec::new();
                    for entry in statuses
                        .iter()
                        .filter(|e| e.status() != git2::Status::CURRENT)
                    {
                        let i_status = match entry.status() {
                            s if s.contains(git2::Status::INDEX_NEW)
                                || s.contains(git2::Status::WT_NEW) =>
                                {
                                    "new"
                                }
                            s if s.contains(git2::Status::INDEX_MODIFIED)
                                || s.contains(git2::Status::WT_MODIFIED) =>
                                {
                                    "modified"
                                }
                            s if s.contains(git2::Status::INDEX_DELETED)
                                || s.contains(git2::Status::WT_DELETED) =>
                                {
                                    "deleted"
                                }
                            s if s.contains(git2::Status::INDEX_RENAMED)
                                || s.contains(git2::Status::WT_RENAMED) =>
                                {
                                    "renamed"
                                }
                            s if s.contains(git2::Status::INDEX_TYPECHANGE)
                                || s.contains(git2::Status::WT_TYPECHANGE) =>
                                {
                                    "type_change"
                                }
                            _ => "",
                        };

                        if entry.status().contains(git2::Status::IGNORED) {
                            continue;
                        }
                        status_changes.push(GitStatusRecord {
                            path: entry.path().unwrap().to_string(),
                            change_type: i_status.to_string(),
                        });
                        // println!("{} ({})", entry.path().unwrap(), istatus);
                    }
                    status::Custom(
                        Status::Ok,
                        (
                            ContentType::JSON,
                            serde_json::to_string_pretty(&status_changes).unwrap(),
                        ),
                    )
                }
                Err(e) => status::Custom(
                    Status::InternalServerError,
                    (
                        ContentType::JSON,
                        make_bad_json_data_response(
                            format!("could not get repo status: {}", e).to_string(),
                        ),
                    ),
                ),
            }
        }
        Err(e) => status::Custom(
            Status::InternalServerError,
            (
                ContentType::JSON,
                make_bad_json_data_response(format!("could not open repo: {}", e).to_string()),
            ),
        ),
    }
}

// METADATA OPERATIONS
#[get("/metadata/raw/<repo_path..>")]
async fn raw_metadata(
    state: &State<AppSettings>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let path_components: Components<'_> = repo_path.components();
    if check_path_components(&mut path_components.clone()) {
        let path_to_serve = state.repo_dir.lock().unwrap().clone()
            + os_slash_str()
            + &repo_path.display().to_string()
            + "/metadata.json";
        match fs::read_to_string(path_to_serve) {
            Ok(v) => status::Custom(Status::Ok, (ContentType::JSON, v)),
            Err(e) => status::Custom(
                Status::BadRequest,
                (
                    ContentType::JSON,
                    make_bad_json_data_response(
                        format!("could not read metadata: {}", e).to_string(),
                    ),
                ),
            ),
        }
    } else {
        status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response("bad repo path".to_string()),
            ),
        )
    }
}

#[get("/metadata/summary/<repo_path..>")]
async fn summary_metadata(
    state: &State<AppSettings>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let path_components: Components<'_> = repo_path.components();
    if check_path_components(&mut path_components.clone()) {
        let path_to_serve = state.repo_dir.lock().unwrap().clone()
            + os_slash_str()
            + &repo_path.display().to_string()
            + os_slash_str()
            + "metadata.json";
        println!("{}", path_to_serve);
        let file_string = match fs::read_to_string(path_to_serve) {
            Ok(v) => v,
            Err(e) => {
                return status::Custom(
                    Status::BadRequest,
                    (
                        ContentType::JSON,
                        make_bad_json_data_response(
                            format!("could not read metadata: {}", e).to_string(),
                        ),
                    ),
                )
            }
        };
        let raw_metadata_struct: Value =
            match serde_json::from_str(file_string.as_str()) {
                Ok(v) => v,
                Err(e) => {
                    return status::Custom(
                        Status::BadRequest,
                        (
                            ContentType::JSON,
                            make_bad_json_data_response(
                                format!("could not parse metadata: {}", e).to_string(),
                            ),
                        ),
                    )
                }
            };
        let summary = MetadataSummary {
            name: raw_metadata_struct["identification"]["name"]["en"]
                .as_str()
                .unwrap()
                .to_string(),
            description: match raw_metadata_struct["identification"]["description"]["en"].clone() {
                Value::String(v) => v.as_str().to_string(),
                Value::Null => "".to_string(),
                _ => "?".to_string(),
            },
            flavor_type: raw_metadata_struct["type"]["flavorType"]["name"]
                .as_str()
                .unwrap()
                .to_string(),
            flavor: raw_metadata_struct["type"]["flavorType"]["flavor"]["name"]
                .as_str()
                .unwrap()
                .to_string(),
            language_code: raw_metadata_struct["languages"][0]["tag"]
                .as_str()
                .unwrap()
                .to_string(),
            script_direction: match raw_metadata_struct["languages"][0]["scriptDirection"].clone() {
                Value::String(v) => v.as_str().to_string(),
                _ => "?".to_string(),
            },
        };
        match serde_json::to_string(&summary) {
            Ok(v) => status::Custom(Status::Ok, (ContentType::JSON, v)),
            Err(e) => status::Custom(
                Status::InternalServerError,
                (
                    ContentType::JSON,
                    make_bad_json_data_response(
                        format!("could not serialize metadata: {}", e).to_string(),
                    ),
                ),
            ),
        }
    } else {
        status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response("bad repo path!".to_string()),
            ),
        )
    }
}

// INGREDIENT OPERATIONS

#[get("/ingredient/raw/<repo_path..>?<ipath>")]
async fn raw_ingredient(
    state: &State<AppSettings>,
    repo_path: PathBuf,
    ipath: String,
) -> status::Custom<(ContentType, String)> {
    let path_components: Components<'_> = repo_path.components();
    if check_path_components(&mut path_components.clone())
        && check_path_string_components(ipath.clone())
    {
        let path_to_serve = state.repo_dir.lock().unwrap().clone()
            + os_slash_str()
            + &repo_path.display().to_string()
            + "/ingredients/"
            + ipath.as_str();
        match fs::read_to_string(path_to_serve) {
            Ok(v) => {
                let mut split_ipath = ipath.split(".").clone();
                let mut suffix = "unknown";
                if let Some(_) = split_ipath.next() {
                    if let Some(second) = split_ipath.next() {
                        suffix = second;
                    }
                }
                status::Custom(
                    Status::Ok,
                    (
                        match mime_types().get(suffix) {
                            Some(t) => t.clone(),
                            None => ContentType::new("application", "octet-stream"),
                        },
                        v,
                    ),
                )
            }
            Err(e) => status::Custom(
                Status::BadRequest,
                (
                    ContentType::JSON,
                    make_bad_json_data_response(
                        format!("could not read ingredient content: {}", e).to_string(),
                    ),
                ),
            ),
        }
    } else {
        status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response("bad repo path".to_string()),
            ),
        )
    }
}

#[get("/ingredient/as-usj/<repo_path..>?<ipath>")]
async fn get_ingredient_as_usj(
    state: &State<AppSettings>,
    repo_path: PathBuf,
    ipath: String,
) -> status::Custom<(ContentType, String)> {
    let path_components: Components<'_> = repo_path.components();
    if check_path_components(&mut path_components.clone())
        && check_path_string_components(ipath.clone())
    {
        let path_to_serve = state.repo_dir.lock().unwrap().clone()
            + os_slash_str()
            + &repo_path.display().to_string()
            + "/ingredients/"
            + ipath.as_str();
        match fs::read_to_string(path_to_serve) {
            Ok(v) => status::Custom(
                Status::Ok,
                (
                    ContentType::JSON,
                    transform(v, "usfm".to_string(), "usj".to_string()),
                ),
            ),
            Err(e) => status::Custom(
                Status::BadRequest,
                (
                    ContentType::JSON,
                    make_bad_json_data_response(
                        format!("could not read ingredient content: {}", e).to_string(),
                    ),
                ),
            ),
        }
    } else {
        status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response("bad repo path".to_string()),
            ),
        )
    }
}

#[post(
    "/ingredient/as-usj/<repo_path..>?<ipath>",
    format = "multipart/form-data",
    data = "<form>"
)]
async fn post_ingredient_as_usj(
    state: &State<AppSettings>,
    repo_path: PathBuf,
    ipath: String,
    mut form: Form<Upload<'_>>,
) -> status::Custom<(ContentType, String)> {
    let path_components: Components<'_> = repo_path.components();
    let destination = state.repo_dir.lock().unwrap().clone()
        + os_slash_str()
        + &repo_path.display().to_string()
        + "/ingredients/"
        + ipath.clone().as_str();
    if check_path_components(&mut path_components.clone())
        && check_path_string_components(ipath)
        && fs::metadata(destination.clone()).is_ok()
    {
        let _ = form
            .file
            .persist_to(transform(
                destination,
                "usj".to_string(),
                "usfm".to_string(),
            ))
            .await;
        status::Custom(
            Status::Ok,
            (
                ContentType::JSON,
                make_good_json_data_response("ok".to_string()),
            ),
        )
    } else {
        status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response("bad repo path".to_string()),
            ),
        )
    }
}

#[get("/ingredient/prettified/<repo_path..>?<ipath>")]
async fn get_ingredient_prettified(
    state: &State<AppSettings>,
    repo_path: PathBuf,
    ipath: String,
) -> status::Custom<(ContentType, String)> {
    let path_components: Components<'_> = repo_path.components();
    if check_path_components(&mut path_components.clone())
        && check_path_string_components(ipath.clone())
    {
        let path_to_serve = state.repo_dir.lock().unwrap().clone()
            + os_slash_str()
            + &repo_path.display().to_string()
            + "/ingredients/"
            + ipath.as_str();
        let file_string = match fs::read_to_string(path_to_serve) {
            Ok(v) => v,
            Err(e) => {
                return status::Custom(
                    Status::BadRequest,
                    (
                        ContentType::JSON,
                        make_bad_json_data_response(
                            format!("could not read ingredient content: {}", e).to_string(),
                        ),
                    ),
                )
            }
        };
        status::Custom(
            Status::Ok,
            (
                ContentType::HTML,
                format!(
                    r#"
                <html>
                <head>
                <title>Prettified</title>
                <link rel="stylesheet" href="/webfonts/_webfonts.css">
                </head>
                <body>
                <pre>
                {}
                </pre>
                </body>
                </html>
                "#,
                    file_string
                ),
            ),
        )
    } else {
        status::Custom(
            Status::BadRequest,
            (
                ContentType::JSON,
                make_bad_json_data_response("bad repo path".to_string()),
            ),
        )
    }
}

// CLIENTS

#[get("/list-clients")]
fn list_clients(clients: &State<Clients>) -> status::Custom<(ContentType, String)> {
    let client_vec = public_serialize_clients(clients.lock().unwrap().clone());
    status::Custom(
        Status::Ok,
        (
            ContentType::JSON,
            serde_json::to_string(&client_vec).unwrap(),
        ),
    )
}

#[get("/favicon.ico")]
async fn serve_root_favicon() -> Redirect {
    Redirect::to("/clients/main/favicon.ico")
}

#[get("/")]
fn redirect_root() -> Redirect {
    Redirect::to("/clients/main")
}

// ERROR HANDLING

#[catch(404)]
fn not_found_catcher(req: &Request<'_>) -> status::Custom<(ContentType, String)> {
    status::Custom(
        Status::NotFound,
        (
            ContentType::JSON,
            make_bad_json_data_response(format!("Resource {} was not found", req.uri()))
                .to_string(),
        ),
    )
}

#[catch(default)]
fn default_catcher(req: &Request<'_>) -> status::Custom<(ContentType, String)> {
    status::Custom(
        Status::InternalServerError,
        (
            ContentType::JSON,
            make_bad_json_data_response(format!("unknown error while serving {}", req.uri()))
                .to_string(),
        ),
    )
}

// BUILD SERVER

type MsgQueue = Arc<Mutex<VecDeque<String>>>;

fn public_serialize_client(c: Client) -> PublicClient {
    PublicClient {
        id: c.id.clone(),
        requires: c.requires.clone(),
        exclude_from_menu: c.exclude_from_menu.clone(),
        exclude_from_dashboard: c.exclude_from_dashboard.clone(),
        url: c.url.clone(),
    }
}
fn public_serialize_clients(cv: Vec<Client>) -> Vec<PublicClient> {
    cv.into_iter().map(|c| public_serialize_client(c)).collect()
}
type Clients = Mutex<Vec<Client>>;

pub fn rocket(launch_config: Value) -> Rocket<Build> {
    println!("OS = '{}'", env::consts::OS);
    // Set up managed state;
    let msg_queue = MsgQueue::new(Mutex::new(VecDeque::new()));
    // Get settings path, default to well-known homedir location
    let root_path = home_dir_string() + os_slash_str();
    let mut working_dir_path = root_path.clone() + "pankosmia_working";
    if launch_config["working_dir"].as_str().unwrap().len() > 0 {
        working_dir_path = launch_config["working_dir"].as_str().unwrap().to_string();
    };
    let user_settings_path = format!("{}/user_settings.json", working_dir_path);
    let app_state_path = format!("{}/app_state.json", working_dir_path);
    let workspace_dir_exists = Path::new(&working_dir_path).is_dir();
    if !workspace_dir_exists {
        // Make working dir, argument is webfonts dir
        match fs::create_dir_all(&working_dir_path) {
            Ok(_) => {}
            Err(e) => {
                println!("Could not create working dir '{}': {}", working_dir_path, e);
                exit(1);
            }
        };
        // Copy user_settings file to working dir
        let user_settings_template_path = relative!("./templates/user_settings.json");
        let user_settings_json_string = match fs::read_to_string(&user_settings_template_path) {
            Ok(s) => maybe_os_quoted_path_str(s.replace("%%WORKINGDIR%%", &working_dir_path)),
            Err(e) => {
                println!(
                    "Could not read user settings template file '{}': {}",
                    user_settings_template_path, e
                );
                exit(1);
            }
        };
        let mut file_handle = match fs::File::create(&user_settings_path) {
            Ok(h) => h,
            Err(e) => {
                println!(
                    "Could not open user_settings file '{}' to write default: {}",
                    user_settings_path, e
                );
                exit(1);
            }
        };
        match file_handle.write_all(&user_settings_json_string.as_bytes()) {
            Ok(_) => {}
            Err(e) => {
                println!(
                    "Could not write default user_settings file to '{}: {}'",
                    user_settings_path, e
                );
                exit(1);
            }
        }
        // Copy app_state file to working dir
        let app_state_template_path = relative!("./templates/app_state.json");
        let app_state_json_string = match fs::read_to_string(&app_state_template_path) {
            Ok(s) => maybe_os_quoted_path_str(s.replace("%%WORKINGDIR%%", &working_dir_path)),
            Err(e) => {
                println!(
                    "Could not read app state template file '{}': {}",
                    user_settings_template_path, e
                );
                exit(1);
            }
        };
        let mut file_handle = match fs::File::create(&app_state_path) {
            Ok(h) => h,
            Err(e) => {
                println!(
                    "Could not open app_state file '{}' to write default: {}",
                    app_state_path, e
                );
                exit(1);
            }
        };
        match file_handle.write_all(&app_state_json_string.as_bytes()) {
            Ok(_) => {}
            Err(e) => {
                println!(
                    "Could not write default app_state file to '{}: {}'",
                    app_state_path, e
                );
                exit(1);
            }
        }
    }
    // Try to load local setup JSON
    let local_setup_path = launch_config["local_setup_path"].as_str().unwrap();
    let local_setup_json_string = match fs::read_to_string(&local_setup_path) {
        Ok(s) => s,
        Err(e) => {
            println!(
                "Could not read local setup file '{}': {}",
                local_setup_path, e
            );
            exit(1);
        }
    };
    let local_setup_json: Value = match serde_json::from_str(local_setup_json_string.as_str()) {
        Ok(j) => j,
        Err(e) => {
            println!(
                "Could not parse local setup file '{}': {}",
                local_setup_path, e
            );
            exit(1);
        }
    };
    let local_pankosmia_path = local_setup_json["local_pankosmia_path"].as_str().unwrap();
    // Try to load app_setup JSON, substituting pankosmia path
    let app_setup_path = launch_config["app_setup_path"].as_str().unwrap();
    let mut app_setup_json_string = match fs::read_to_string(&app_setup_path) {
        Ok(s) => s,
        Err(e) => {
            println!("Could not read app_setup file '{}': {}", app_setup_path, e);
            exit(1);
        }
    };
    app_setup_json_string = app_setup_json_string.replace(
        "%%PANKOSMIADIR%%",
        maybe_os_quoted_path_str(local_pankosmia_path.to_string()).as_str(),
    );
    let app_setup_json: Value = match serde_json::from_str(app_setup_json_string.as_str()) {
        Ok(j) => j,
        Err(e) => {
            println!("Could not parse app_setup file '{}': {}", app_setup_path, e);
            exit(1);
        }
    };
    // Try to load app state JSON
    let app_state_json_string = match fs::read_to_string(&app_state_path) {
        Ok(s) => s,
        Err(e) => {
            println!("Could not read app_state file '{}': {}", app_state_path, e);
            exit(1);
        }
    };
    let app_state_json: Value = match serde_json::from_str(app_state_json_string.as_str()) {
        Ok(j) => j,
        Err(e) => {
            println!("Could not parse app_state file '{}': {}", app_state_path, e);
            exit(1);
        }
    };
    // Try to load user settings JSON
    let user_settings_json_string = match fs::read_to_string(&user_settings_path) {
        Ok(s) => s,
        Err(e) => {
            println!(
                "Could not read user_settings file '{}': {}",
                user_settings_path, e
            );
            exit(1);
        }
    };
    let user_settings_json: Value = match serde_json::from_str(user_settings_json_string.as_str()) {
        Ok(j) => j,
        Err(e) => {
            println!(
                "Could not parse user_settings file '{}': {}",
                user_settings_path, e
            );
            exit(1);
        }
    };
    // Find or make repo_dir
    let repo_dir_path = match user_settings_json["repo_dir"].as_str() {
        Some(v) => v.to_string(),
        None => {
            println!(
                "Could not parse repo_dir in user_settings file '{}' as a string",
                user_settings_path
            );
            exit(1);
        }
    };
    let repo_dir_path_exists = Path::new(&repo_dir_path).is_dir();
    if !repo_dir_path_exists {
        match fs::create_dir_all(&repo_dir_path) {
            Ok(_) => {}
            Err(e) => {
                println!(
                    "Repo dir '{}' doe not exist and could not be created: {}",
                    repo_dir_path, e
                );
                exit(1);
            }
        };
    }
    // Copy web fonts from path in local config
    let template_webfonts_dir_path = launch_config["webfont_path"].as_str().unwrap();
    let webfonts_dir_path = working_dir_path.clone() + os_slash_str() + "webfonts";
    if !Path::new(&webfonts_dir_path).is_dir() {
        match copy_dir(template_webfonts_dir_path, webfonts_dir_path.clone()) {
            Ok(_) => {}
            Err(e) => {
                println!(
                    "Could not copy web fonts to working directory from {}: {}",
                    template_webfonts_dir_path, e
                );
                exit(1);
            }
        }
    };
    // Merge client config into settings JSON
    let mut clients_merged_array: Vec<Value> = Vec::new();
    let mut client_records_merged_array: Vec<Value> = Vec::new();
    let app_client_records = app_setup_json["clients"].as_array().unwrap();
    for app_client_record in app_client_records.iter() {
        client_records_merged_array.push(app_client_record.clone());
    }
    let my_client_records = user_settings_json["my_clients"].as_array().unwrap();
    for my_client_record in my_client_records.iter() {
        client_records_merged_array.push(my_client_record.clone());
    }
    for client_record in client_records_merged_array.iter() {
        // Get requires from metadata
        let client_metadata_path = client_record["path"].as_str().unwrap().to_string()
            + os_slash_str()
            + "pankosmia_metadata.json";
        let metadata_json: Value = match fs::read_to_string(&client_metadata_path) {
            Ok(mt) => match serde_json::from_str(&mt) {
                Ok(m) => m,
                Err(e) => {
                    println!(
                        "Could not parse metadata file {} as JSON: {}\n{}",
                        &client_metadata_path, e, mt
                    );
                    exit(1);
                }
            },
            Err(e) => {
                println!(
                    "Could not read metadata file {}: {}",
                    client_metadata_path, e
                );
                exit(1);
            }
        };
        let mut debug_flag = false;
        let md_require = metadata_json["require"].as_object().unwrap();
        if md_require.contains_key("debug") {
            debug_flag = md_require.clone()["debug"].as_bool().unwrap();
        }
        let requires = json!({
            "net": md_require.clone()["net"].as_bool().unwrap(),
            "debug": debug_flag
        });
        // Get url from package.json
        let package_json_path =
            client_record["path"].as_str().unwrap().to_string() + os_slash_str() + "package.json";
        let package_json: Value = match fs::read_to_string(&package_json_path) {
            Ok(pj) => match serde_json::from_str(&pj) {
                Ok(p) => p,
                Err(e) => {
                    println!(
                        "Could not parse package.json file {} as JSON: {}\n{}",
                        &package_json_path, e, pj
                    );
                    exit(1);
                }
            },
            Err(e) => {
                println!(
                    "Could not read package.json file {}: {}",
                    package_json_path, e
                );
                exit(1);
            }
        };
        // Build client record
        clients_merged_array.push(json!({
            "id": metadata_json["id"].as_str().unwrap(),
            "path": client_record["path"].as_str().unwrap(),
            "url": package_json["homepage"].as_str().unwrap(),
            "requires": requires,
            "exclude_from_menu": metadata_json["exclude_from_menu"].as_bool().unwrap_or_else(|| false),
            "exclude_from_dashboard": metadata_json["exclude_from_dashboard"].as_bool().unwrap_or_else(|| false)
        }));
    }
    let clients_value = serde_json::to_value(clients_merged_array).unwrap();
    // Process clients metadata to build clients and i18n
    let clients: Clients = match serde_json::from_value(clients_value) {
        Ok(v) => v,
        Err(e) => {
            println!(
                "Could not parse clients array in settings file '{}' as client records: {}",
                app_setup_path, e
            );
            exit(1);
        }
    };
    let i18n_template_path = relative!("./templates").to_string() + os_slash_str() + "i18n.json";
    let mut i18n_json_map: Map<String, Value> = match fs::read_to_string(&i18n_template_path) {
        Ok(it) => match serde_json::from_str(&it) {
            Ok(i) => i,
            Err(e) => {
                println!(
                    "Could not parse i18n template {} as JSON: {}\n{}",
                    &i18n_template_path, e, it
                );
                exit(1);
            }
        },
        Err(e) => {
            println!("Could not read i18n template {}: {}", i18n_template_path, e);
            exit(1);
        }
    };
    let mut i18n_pages_map = Map::new();
    let mut found_main = false;
    let mut locked_clients = clients.lock().unwrap().clone();
    let inner_clients = &mut *locked_clients;
    // Iterate over clients to build i18n
    for client_record in inner_clients {
        if !Path::new(&client_record.path.clone()).is_dir() {
            println!(
                "Client path {} from app_setup file {} is not a directory",
                client_record.path, app_setup_path
            );
            exit(1);
        }
        let build_path = format!("{}/build", client_record.path.clone());
        if !Path::new(&build_path.clone()).is_dir() {
            println!("Client build path within {} from app_setup file {} does not exist or is not a directory", client_record.path.clone(), app_setup_path);
            exit(1);
        }
        let client_metadata_path =
            client_record.path.clone() + os_slash_str() + "pankosmia_metadata.json";
        let metadata_json: Value = match fs::read_to_string(&client_metadata_path) {
            Ok(mt) => match serde_json::from_str(&mt) {
                Ok(m) => m,
                Err(e) => {
                    println!(
                        "Could not parse metadata file {} as JSON: {}\n{}",
                        &client_metadata_path, e, mt
                    );
                    exit(1);
                }
            },
            Err(e) => {
                println!(
                    "Could not read metadata file {}: {}",
                    client_metadata_path, e
                );
                exit(1);
            }
        };
        let metadata_id = metadata_json["id"].as_str().unwrap().to_string();
        let metadata_i18n = metadata_json["i18n"].clone();
        i18n_pages_map.insert(metadata_id.clone(), metadata_i18n);
        if client_record.url.clone() == "/clients/main".to_string() {
            found_main = true;
        }
    }
    i18n_json_map.insert(
        "pages".to_string(),
        Value::Object(i18n_pages_map),
    );
    let i18n_target_path = working_dir_path.clone() + os_slash_str() + "i18n.json";
    let mut i18n_file_handle = match fs::File::create(&i18n_target_path) {
        Ok(h) => h,
        Err(e) => {
            println!(
                "Could not open target i18n file '{}': {}",
                i18n_target_path, e
            );
            exit(1);
        }
    };
    match i18n_file_handle.write_all(
        Value::Object(i18n_json_map)
            .to_string()
            .as_bytes(),
    ) {
        Ok(_) => {}
        Err(e) => {
            println!(
                "Could not write target i18n file to '{}': {}",
                i18n_target_path, e
            );
            exit(1);
        }
    }
    // Throw if no main found
    if !found_main {
        println!("Could not find a client registered at /main among clients in settings file");
        exit(1);
    }

    let mut my_rocket = rocket::build()
        .register("/", catchers![not_found_catcher, default_catcher])
        .manage(AppSettings {
            repo_dir: Mutex::new(repo_dir_path.clone()),
            working_dir: working_dir_path.clone(),
            languages: Mutex::new(
                user_settings_json["languages"]
                    .as_array()
                    .unwrap()
                    .into_iter()
                    .map(|i| {
                        i.as_str()
                            .expect("Non-string in user_settings language array")
                            .to_string()
                    })
                    .collect(),
            ),
            auth_tokens: match user_settings_json["auth_tokens"].clone() {
                Value::Object(v) => {
                    Mutex::new(serde_json::from_value(Value::Object(v)).unwrap())
                }
                _ => Mutex::new(BTreeMap::new()),
            },
            gitea_endpoints: match user_settings_json["gitea_endpoints"].clone() {
                Value::Object(v) => {
                    serde_json::from_value(Value::Object(v)).unwrap()
                }
                _ => BTreeMap::new(),
            },
            typography: match user_settings_json["typography"].clone() {
                Value::Object(v) => {
                    serde_json::from_value(Value::Object(v)).unwrap()
                }
                _ => {
                    println!("Could not read typography from parsed user settings file");
                    exit(1)
                }
            },
            bcv: match app_state_json["bcv"].clone() {
                Value::Object(v) => {
                    serde_json::from_value(Value::Object(v)).unwrap()
                }
                _ => serde_json::from_value(json!({
                    "book_code": "TIT",
                    "chapter": 1,
                    "verse": 1
                }))
                    .unwrap(),
            },
        })
        .mount(
            "/",
            routes![redirect_root, serve_root_favicon, list_clients],
        )
        .mount("/notifications", routes![notifications_stream,])
        .mount(
            "/settings",
            routes![
                get_languages,
                get_auth_token,
                get_new_auth_token,
                get_typography,
                post_typography
            ],
        )
        .mount("/net", routes![net_status, net_enable, net_disable])
        .mount("/debug", routes![debug_status, debug_enable, debug_disable])
        .mount(
            "/i18n",
            routes![raw_i18n, negotiated_i18n, flat_i18n, untranslated_i18n],
        )
        .mount("/navigation", routes![get_bcv, post_bcv])
        .mount("/gitea", routes![
            gitea_remote_repos,
            get_gitea_endpoints
        ])
        .mount(
            "/git",
            routes![
                fetch_repo,
                list_local_repos,
                delete_repo,
                add_and_commit,
                git_status
            ],
        )
        .mount(
            "/burrito",
            routes![
                raw_ingredient,
                get_ingredient_prettified,
                get_ingredient_as_usj,
                post_ingredient_as_usj,
                raw_metadata,
                summary_metadata
            ],
        )
        .mount("/webfonts", FileServer::from(webfonts_dir_path.clone()));
    let client_vec = clients.lock().unwrap().clone();
    for client_record in client_vec {
        my_rocket = my_rocket.mount(
            client_record.url.clone(),
            FileServer::from(client_record.path.clone() + os_slash_str() + "build"),
        );
    }
    my_rocket.manage(msg_queue).manage(clients)
}
