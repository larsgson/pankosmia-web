#[doc(hidden)]
use rocket::{Build, Rocket};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::env;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub mod auth;
pub mod catalog;
pub mod identity;
pub mod server;
pub mod store;
pub mod structs;
mod utils;
use crate::utils::bootstrap::{
    build_client_record, build_clients_and_i18n, copy_and_customize_webfonts,
    initialize_working_dir, load_configs, maybe_make_repo_dir, merged_clients,
};
use crate::utils::files::load_json;
use crate::utils::json::get_string_value_by_key;
use crate::utils::launch::{add_app_settings, add_catchers, add_routes, add_static_routes};
use crate::utils::paths::{
    home_dir_string, os_slash_str, source_local_setup_path, user_settings_path, webfonts_path,
};
pub mod endpoints;
mod static_vars;
#[allow(unused_imports)]
use crate::static_vars::{DEBUG_IS_ENABLED, NET_IS_ENABLED};
use crate::structs::ClientConfigSection;

#[warn(unused_imports)]

type MsgQueue = Arc<Mutex<VecDeque<String>>>;

fn ensure_trailing_slash(s: &str) -> String {
    if s.is_empty() || s.ends_with(os_slash_str()) {
        s.to_string()
    } else {
        format!("{}{}", s, os_slash_str())
    }
}

pub fn rocket(launch_config: Value) -> Rocket<Build> {
    println!("OS = '{}'", env::consts::OS);

    // Locate the product / client_config JSON. Preferred location:
    // `<APP_RESOURCES_DIR>/product/{product,client_config}.json`.
    // Legacy fallback: `<binary>/../../lib/app_resources/product/...`
    // (pankosmia-web bundled the binary under `target/lib/app_resources/...`).
    let app_resources_for_product =
        get_string_value_by_key(&launch_config, "app_resources_path");
    let preferred_product_path = format!(
        "{}product{}product.json",
        ensure_trailing_slash(&app_resources_for_product),
        os_slash_str(),
    );
    let binary_path = env::current_exe().unwrap();
    let binary_grandparent = binary_path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.to_str())
        .unwrap_or("");
    let legacy_product_path = format!(
        "{}{}lib{}app_resources{}product{}product.json",
        binary_grandparent,
        os_slash_str(),
        os_slash_str(),
        os_slash_str(),
        os_slash_str(),
    );
    let product_path = if Path::new(&preferred_product_path).is_file() {
        preferred_product_path
    } else {
        legacy_product_path
    };
    let product_json = match load_json(product_path.as_str()) {
        Ok(j) => j,
        Err(e) => panic!(
            "Could not read and parse product json as {}: {}",
            product_path, e
        ),
    };
    let product_short_name = product_json["short_name"].as_str().unwrap().to_string();
    println!("Product = {}", &product_short_name);

    // Same logic for client_config.json (absent → empty config, no panic).
    let preferred_client_config_path = format!(
        "{}product{}client_config.json",
        ensure_trailing_slash(&app_resources_for_product),
        os_slash_str(),
    );
    let legacy_client_config_path = format!(
        "{}{}lib{}app_resources{}product{}client_config.json",
        binary_grandparent,
        os_slash_str(),
        os_slash_str(),
        os_slash_str(),
        os_slash_str(),
    );
    let client_config_path = if Path::new(&preferred_client_config_path).is_file() {
        preferred_client_config_path
    } else {
        legacy_client_config_path
    };
    let client_config_json = match load_json(client_config_path.as_str()) {
        Ok(j) => j,
        Err(_e) => {
            println!("WARNING: No client config file found");
            json!({})
        }
    };
    let mut client_config = BTreeMap::new();
    let client_config_json_object = client_config_json
        .as_object()
        .expect("client config as object");
    for (section_k, section_v) in client_config_json_object {
        let section: Vec<ClientConfigSection> =
            serde_json::from_value(section_v.clone()).expect("client config as struct");
        client_config.insert(section_k.to_string(), section);
    }

    // Default workspace path
    let root_path = home_dir_string() + os_slash_str();
    let mut working_dir_path = format!(
        "{}pankosmia{}{}",
        root_path.clone(),
        os_slash_str(),
        &product_short_name
    );

    // Override default if another value is supplied
    let launch_working_dir = get_string_value_by_key(&launch_config, "working_dir");
    if launch_working_dir.len() > 3 {
        // Try not to mangle entire FS with empty path strings
        working_dir_path = launch_working_dir.clone();
    };

    // Initialise the working dir if it doesn't exist OR is missing
    // its core config files (Railway / Docker volume-mounted-but-
    // empty scenario: `/data` is pre-created by the orchestrator but
    // has no Pankosmia state in it yet).
    let needs_init = !Path::new(&working_dir_path).is_dir()
        || !Path::new(&user_settings_path(&working_dir_path)).is_file();
    if needs_init {
        let app_resources_dir = get_string_value_by_key(&launch_config, "app_resources_path");
        let local_setup_json =
            load_json(source_local_setup_path(app_resources_dir).as_str()).unwrap();
        initialize_working_dir(
            &local_setup_json["local_pankosmia_path"]
                .as_str()
                .unwrap()
                .to_string(),
            &app_resources_dir,
            &working_dir_path,
        );
    }
    // Always (re)create a clean temp dir on boot, regardless of init.
    let temp_dir_path = format!("{}{}temp", &working_dir_path, os_slash_str());
    if Path::new(&temp_dir_path).exists() {
        match std::fs::remove_dir_all(&temp_dir_path) {
            Ok(_) => (),
            Err(e) => panic!("Could not delete temp directory: {}", e),
        }
    }
    match std::fs::create_dir(&temp_dir_path) {
        Ok(_) => (),
        Err(e) => panic!("Could not create temp directory: {}", e),
    }

    // Load the config JSONs
    let (app_setup_json, user_settings_json, app_state_json) =
        load_configs(&working_dir_path, &launch_config);

    // Find or make repo_dir
    let repo_dir_path = get_string_value_by_key(&user_settings_json, "repo_dir");
    maybe_make_repo_dir(&repo_dir_path);
    // Check for app_resources_dir
    let app_resources_dir_path = match &user_settings_json["app_resources_dir"] {
        Value::Null => panic!("app_resources_dir does not exist in user_settings.json"),
        Value::String(s) => s.to_string(),
        _ => panic!("app_resources_dir exists in user_settings.json but is not a string"),
    };
    if !Path::new(&app_resources_dir_path).is_dir() {
        panic!(
            "app_resources_dir setting '{}' in user_settings.json is not a directory",
            app_resources_dir_path
        );
    }

    // Copy web fonts from path in local config
    let template_webfonts_dir_path = get_string_value_by_key(&launch_config, "webfont_path");
    let webfonts_dir_path = webfonts_path(&working_dir_path);
    copy_and_customize_webfonts(
        template_webfonts_dir_path,
        &webfonts_dir_path,
        &user_settings_json,
    );

    // Merge client config (from app setup and user settings) into settings JSON
    let client_records_merged_array = merged_clients(&app_setup_json, &user_settings_json);

    // Construct clients as Values
    let mut clients_merged_array: Vec<Value> = Vec::new();
    for client_record in client_records_merged_array.iter() {
        clients_merged_array.push(build_client_record(&client_record));
    }
    // Build complete clients with i18n
    let clients = build_clients_and_i18n(
        clients_merged_array,
        &app_resources_dir_path,
        &working_dir_path,
    );

    // *** LAUNCH ROCKET ***
    let mut my_rocket = rocket::build();

    // Error handlers
    my_rocket = add_catchers(my_rocket);

    // Routes
    my_rocket = add_routes(my_rocket);
    let client_vec = clients.lock().unwrap().clone();
    my_rocket = add_static_routes(
        my_rocket,
        client_vec,
        &app_resources_dir_path,
        &webfonts_dir_path,
    );

    // State
    my_rocket = add_app_settings(
        my_rocket,
        &repo_dir_path,
        &app_resources_dir_path,
        &working_dir_path,
        &user_settings_json,
        &app_state_json,
        &product_json,
        client_config,
    );
    let msg_queue = MsgQueue::new(Mutex::new(VecDeque::new()));
    my_rocket = my_rocket.manage(msg_queue).manage(clients);

    // Catalog registry (G2). Empty by default; for hosted
    // STORAGE_BACKEND=github, populated at startup from the
    // catalog repo's local clone or from a manually-provided yaml
    // path via PANKOSMIA_CATALOG_PATH.
    let catalog = Arc::new(crate::catalog::CatalogRegistry::empty());
    if let Ok(path) = env::var("PANKOSMIA_CATALOG_PATH") {
        match std::fs::read_to_string(&path) {
            Ok(yaml) => match catalog.reload_from_yaml(&yaml) {
                Ok(diff) => println!(
                    "Catalog loaded from {}: {} added, {} removed",
                    path,
                    diff.added.len(),
                    diff.removed.len()
                ),
                Err(e) => println!("WARN: catalog parse error: {}", e),
            },
            Err(e) => println!("WARN: could not read catalog at {}: {}", path, e),
        }
    }
    my_rocket = my_rocket.manage(catalog.clone());

    // Phase 2 storage abstraction. Endpoints call this trait object
    // instead of `std::fs::*` directly. The runtime selector picks
    // the implementation from `STORAGE_BACKEND=fs|github`.
    let project_store = crate::store::selector::build_project_store(
        std::path::PathBuf::from(repo_dir_path.clone()),
        Some(catalog.clone()),
    );
    // Periodic-fetch fallback for missed language webhooks. Spawned
    // before `manage` consumes the store so we can hold our own
    // Arc clone. No-op cadence in FS mode (empty catalog).
    if let Some(interval) = crate::server::periodic_fetch::interval_from_env() {
        crate::server::periodic_fetch::spawn(
            catalog.clone(),
            project_store.clone(),
            interval,
        );
    } else {
        println!("periodic_fetch: disabled (PANKOSMIA_PERIODIC_FETCH_INTERVAL_SECS=0)");
    }
    my_rocket = my_rocket.manage(project_store);

    // M4 — scaling primitives. Per-language locks for git
    // serialization; bounded blocking pools for git2 / CPU work so
    // one heavy operation can't starve the request path; shared
    // file-watcher registry so 50 SSE subscribers on the same file
    // share one inotify subscription instead of opening 50.
    my_rocket = my_rocket
        .manage(crate::server::LanguageLocks::new())
        .manage(crate::server::BlockingPools::new())
        .manage(crate::server::WatcherRegistry::new())
        .manage(crate::server::RateLimiter::default_for_saves())
        .manage(crate::store::github::AudioRefConfig::from_env());

    // GitHub OAuth + token store. The OAuth client_id /
    // client_secret are read from env. If not set, the OAuth
    // endpoints respond with 502 (badly configured server); the
    // rest of the server still works.
    let github_client_id = env::var("GITHUB_CLIENT_ID").unwrap_or_default();
    let github_client_secret = env::var("GITHUB_CLIENT_SECRET").unwrap_or_default();
    let github_client =
        crate::auth::GithubClient::new(github_client_id, github_client_secret);
    my_rocket = my_rocket
        .manage(github_client.clone())
        .manage(crate::auth::TokenStore::from_env(std::path::PathBuf::from(
            repo_dir_path.clone(),
        )));

    // GitHub edit flow (save → branch → PUT contents → PR). Managed
    // unconditionally so endpoints can take it as `&State` and
    // decide at request time whether to dispatch into it.
    let edit_flow = crate::store::github::GithubEditFlow::new(catalog.clone());
    my_rocket = my_rocket.manage(edit_flow);
    let _ = github_client; // managed above; no longer baked into edit_flow

    // GitHub App auth (foundation for the App-model edit flow).
    // Always managed as `Option<GithubAppAuth>` so Rocket's sentinel
    // system can verify the state type at boot — endpoints take
    // `&State<Option<GithubAppAuth>>` and branch on the inner option.
    let app_auth_option: Option<crate::auth::GithubAppAuth> =
        match crate::auth::GithubAppAuth::from_env() {
            Ok(Some(app_auth)) => {
                println!("GitHub App auth configured (App ID present)");
                Some(app_auth)
            }
            Ok(None) => {
                println!("GitHub App auth not configured (GITHUB_APP_ID unset)");
                None
            }
            Err(e) => {
                eprintln!("WARN: GitHub App auth misconfigured: {}", e);
                None
            }
        };
    my_rocket = my_rocket.manage(app_auth_option);

    // Webhook secret for GitHub webhooks (G2). Empty when not
    // configured; webhook endpoints fail with 503 in that case.
    my_rocket = my_rocket.manage(crate::catalog::WebhookSecret::from_env());

    my_rocket
}
