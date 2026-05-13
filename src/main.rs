use rocket::fs::relative;
use serde_json::json;
use std::env;

#[rocket::main]
pub async fn main() -> Result<(), rocket::Error> {
    // Bridge PaaS-style `PORT` injection (Railway, Fly.io, Heroku-
    // family) to Rocket's expected `ROCKET_PORT`. When `PORT` is set
    // it always wins — that's the convention. Without `PORT` the
    // server falls back to `ROCKET_PORT` (or Rocket's default).
    if let Ok(port) = env::var("PORT") {
        env::set_var("ROCKET_PORT", port);
    }

    let args: Vec<String> = env::args().collect();
    let mut working_dir = "".to_string();
    if args.len() == 2 {
        working_dir = args[1].clone();
    };
    let mut app_resources_path = relative!("").to_string();
    if env::var("APP_RESOURCES_DIR").is_ok() {
        app_resources_path = env::var("APP_RESOURCES_DIR").unwrap();
    }
    let webfont_path = format!("{}webfonts", app_resources_path);
    let app_setup_path = format!("{}setup/app_setup.json", app_resources_path);
    let local_setup_path = format!("{}setup/local_setup.json", app_resources_path);
    let conf = json!({
        "working_dir": working_dir,
        "webfont_path": webfont_path,
        "app_setup_path": app_setup_path,
        "local_setup_path": local_setup_path,
        "app_resources_path": app_resources_path,
    });
    pankosmia_docker::rocket(conf).launch().await?;
    Ok(())
}
