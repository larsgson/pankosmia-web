//! Provision new language repos under `pankosmia-langs`.
//!
//! Usage:
//!   cargo run --bin provision-langs -- <csv-path>
//!
//! CSV columns: `bcp47,english_name,native_name,direction,script`
//! (`direction` defaults to `ltr`; `script` defaults to `Latn`).
//!
//! For each row, this:
//!   1. Mints an installation token via `GithubAppAuth`.
//!   2. Creates `pankosmia-langs/<bcp47>` as a public auto-init repo
//!      with default branch `main`.
//!   3. Atomically replaces HEAD's tree with the per-language seed
//!      tree (README, LICENSE, language.yaml, obs/manifest.yaml,
//!      obs/media.yaml fetched from door43, obs/content/01.md..50.md
//!      empty).
//!   4. Sets the topics `pankosmia-language` and `bcp47-<code>`.
//!
//! Env vars: `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY_PATH` (or
//! `_KEY`), `PANKOSMIA_DEFAULT_INSTALLATION_ID`.

use pankosmia_docker::auth::github_app::GithubAppAuth;
use serde::Serialize;
use serde_json::json;
use std::env;
use std::error::Error;

const ORG: &str = "pankosmia-langs";
const GITHUB_API: &str = "https://api.github.com";
const USER_AGENT: &str = "pankosmia-provision-langs";
const DOOR43_MEDIA_YAML: &str =
    "https://git.door43.org/unfoldingWord/en_obs/raw/branch/master/media.yaml";

struct LangRow {
    bcp47: String,
    english_name: String,
    native_name: String,
    direction: String,
    script: String,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let csv_path = args.get(1).ok_or("usage: provision-langs <csv-path>")?;
    let rows = read_csv(csv_path)?;
    if rows.is_empty() {
        return Err("CSV had no data rows".into());
    }

    let app = GithubAppAuth::from_env()?.ok_or("GITHUB_APP_ID + private key env vars required")?;
    let installation_id: u64 = env::var("PANKOSMIA_DEFAULT_INSTALLATION_ID")
        .map_err(|_| "PANKOSMIA_DEFAULT_INSTALLATION_ID required")?
        .parse()
        .map_err(|e| format!("PANKOSMIA_DEFAULT_INSTALLATION_ID not numeric: {}", e))?;
    let token = app.installation_token(installation_id).await?;

    let http = reqwest::Client::builder().user_agent(USER_AGENT).build()?;

    println!("[provision] fetching door43 en_obs/media.yaml ...");
    let media_yaml = http
        .get(DOOR43_MEDIA_YAML)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    for row in rows {
        println!("\n[provision] {} ({}) ...", row.bcp47, row.english_name);
        provision_one(&http, &token, &row, &media_yaml).await?;
        println!(
            "[provision] {} done → https://github.com/{}/{}",
            row.bcp47, ORG, row.bcp47
        );
    }
    Ok(())
}

async fn provision_one(
    http: &reqwest::Client,
    token: &str,
    row: &LangRow,
    media_yaml: &str,
) -> Result<(), Box<dyn Error>> {
    let repo = &row.bcp47;
    let full = format!("{}/{}", ORG, repo);

    create_org_repo(http, token, row).await?;
    let head_sha = wait_for_main(http, token, &full).await?;

    let mut entries: Vec<TreeEntry> = Vec::new();
    let seeds = build_seed_files(row, media_yaml);
    for (path, content) in &seeds {
        let blob_sha = create_blob(http, token, &full, content.as_bytes()).await?;
        entries.push(TreeEntry {
            path: path.clone(),
            mode: "100644".into(),
            entry_type: "blob".into(),
            sha: Some(blob_sha),
        });
    }

    // Fresh tree (no base_tree) so the auto-init README is dropped —
    // we wrote our own.
    let tree_sha = create_tree(http, token, &full, None, &entries).await?;
    let commit_sha = create_commit(
        http,
        token,
        &full,
        "Seed language repo",
        &tree_sha,
        &[&head_sha],
    )
    .await?;
    update_main(http, token, &full, &commit_sha).await?;

    set_topics(
        http,
        token,
        &full,
        &[
            "pankosmia-language".into(),
            format!("bcp47-{}", row.bcp47.to_lowercase()),
        ],
    )
    .await?;

    Ok(())
}

// --- CSV parsing -----------------------------------------------------

fn read_csv(path: &str) -> Result<Vec<LangRow>, Box<dyn Error>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines.next().ok_or("empty CSV")?;
    let cols: Vec<&str> = header.split(',').map(|s| s.trim()).collect();
    let idx = |name: &str| {
        cols.iter()
            .position(|c| c.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("CSV missing column: {}", name))
    };
    let i_bcp47 = idx("bcp47")?;
    let i_en = idx("english_name")?;
    let i_nat = idx("native_name")?;
    let i_dir = cols
        .iter()
        .position(|c| c.eq_ignore_ascii_case("direction"));
    let i_script = cols.iter().position(|c| c.eq_ignore_ascii_case("script"));

    let mut out = Vec::new();
    for line in lines {
        let fields: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        out.push(LangRow {
            bcp47: fields.get(i_bcp47).copied().unwrap_or("").to_string(),
            english_name: fields.get(i_en).copied().unwrap_or("").to_string(),
            native_name: fields.get(i_nat).copied().unwrap_or("").to_string(),
            direction: i_dir
                .and_then(|i| fields.get(i).copied())
                .filter(|s| !s.is_empty())
                .unwrap_or("ltr")
                .to_string(),
            script: i_script
                .and_then(|i| fields.get(i).copied())
                .filter(|s| !s.is_empty())
                .unwrap_or("Latn")
                .to_string(),
        });
    }
    Ok(out)
}

// --- seed file generation -------------------------------------------

fn build_seed_files(row: &LangRow, media_yaml: &str) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = Vec::new();

    files.push(("README.md".into(), readme_md(row)));
    files.push(("LICENSE.md".into(), license_md()));
    files.push(("language.yaml".into(), language_yaml(row)));
    files.push(("obs/manifest.yaml".into(), obs_manifest_yaml(row)));
    files.push(("obs/media.yaml".into(), media_yaml.to_string()));
    for n in 1..=50u32 {
        files.push((format!("obs/content/{:02}.md", n), String::new()));
    }
    files
}

fn readme_md(row: &LangRow) -> String {
    format!(
        "# {english} ({bcp47})\n\
         \n\
         _{native}_\n\
         \n\
         Open Bible Stories translation in **{english}**.\n\
         \n\
         - Language code: `{bcp47}`\n\
         - Script: `{script}`\n\
         - Direction: `{direction}`\n\
         - Resources present: see top-level folders (`obs/`, …).\n\
         \n\
         Edit at <https://pankosmia-web.up.railway.app/?lang={bcp47}>.\n\
         \n\
         Licensed under [CC BY-SA 4.0](./LICENSE.md).\n",
        english = row.english_name,
        native = row.native_name,
        bcp47 = row.bcp47,
        script = row.script,
        direction = row.direction,
    )
}

fn license_md() -> String {
    "# License\n\
     \n\
     This work is licensed under a **Creative Commons \
     Attribution-ShareAlike 4.0 International License (CC BY-SA 4.0)**.\n\
     \n\
     You are free to share and adapt this work for any purpose, including \
     commercially, provided you give appropriate credit and distribute your \
     contributions under the same license.\n\
     \n\
     Canonical license text: <https://creativecommons.org/licenses/by-sa/4.0/legalcode>\n\
     \n\
     Human-readable summary: <https://creativecommons.org/licenses/by-sa/4.0/>\n"
        .to_string()
}

fn language_yaml(row: &LangRow) -> String {
    format!(
        "bcp47: '{bcp47}'\n\
         english_name: '{english}'\n\
         native_name: '{native}'\n\
         direction: '{direction}'\n\
         script: '{script}'\n",
        bcp47 = row.bcp47,
        english = yaml_esc(&row.english_name),
        native = yaml_esc(&row.native_name),
        direction = row.direction,
        script = row.script,
    )
}

fn obs_manifest_yaml(row: &LangRow) -> String {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    format!(
        "dublin_core:\n\
         \x20\x20conformsto: 'rc0.2'\n\
         \x20\x20contributor: []\n\
         \x20\x20creator: 'pankosmia'\n\
         \x20\x20description: 'Open Bible Stories in {english}.'\n\
         \x20\x20format: 'text/markdown'\n\
         \x20\x20identifier: 'obs'\n\
         \x20\x20issued: '{today}'\n\
         \x20\x20language:\n\
         \x20\x20\x20\x20direction: '{direction}'\n\
         \x20\x20\x20\x20identifier: '{bcp47}'\n\
         \x20\x20\x20\x20title: '{native}'\n\
         \x20\x20modified: '{today}'\n\
         \x20\x20publisher: 'pankosmia'\n\
         \x20\x20relation: []\n\
         \x20\x20rights: 'CC BY-SA 4.0'\n\
         \x20\x20source:\n\
         \x20\x20\x20\x20-\n\
         \x20\x20\x20\x20\x20\x20identifier: 'obs'\n\
         \x20\x20\x20\x20\x20\x20language: 'en'\n\
         \x20\x20\x20\x20\x20\x20version: '9'\n\
         \x20\x20subject: 'Open Bible Stories'\n\
         \x20\x20title: 'Open Bible Stories ({english})'\n\
         \x20\x20type: 'book'\n\
         \x20\x20version: '0'\n\
         \n\
         checking:\n\
         \x20\x20checking_entity: []\n\
         \x20\x20checking_level: '1'\n\
         \n\
         projects:\n\
         \x20\x20-\n\
         \x20\x20\x20\x20categories:\n\
         \x20\x20\x20\x20identifier: 'obs'\n\
         \x20\x20\x20\x20path: './content'\n\
         \x20\x20\x20\x20sort: 0\n\
         \x20\x20\x20\x20title: 'Open Bible Stories ({english})'\n\
         \x20\x20\x20\x20versification:\n",
        english = yaml_esc(&row.english_name),
        native = yaml_esc(&row.native_name),
        bcp47 = row.bcp47,
        direction = row.direction,
        today = today,
    )
}

fn yaml_esc(s: &str) -> String {
    s.replace('\'', "''")
}

// --- GitHub REST calls ----------------------------------------------

#[derive(Serialize)]
struct CreateRepoBody<'a> {
    name: &'a str,
    description: String,
    private: bool,
    auto_init: bool,
    default_branch: &'a str,
    has_issues: bool,
    has_wiki: bool,
    has_projects: bool,
}

async fn create_org_repo(
    http: &reqwest::Client,
    token: &str,
    row: &LangRow,
) -> Result<(), Box<dyn Error>> {
    let body = CreateRepoBody {
        name: &row.bcp47,
        description: format!("Open Bible Stories in {} ({})", row.english_name, row.bcp47),
        private: false,
        auto_init: true,
        default_branch: "main",
        has_issues: true,
        has_wiki: false,
        has_projects: false,
    };
    let resp = http
        .post(format!("{}/orgs/{}/repos", GITHUB_API, ORG))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if status.as_u16() == 422 {
        // Already exists — treat as idempotent and continue.
        let body = resp.text().await.unwrap_or_default();
        if body.contains("name already exists") {
            println!(
                "[provision]   repo {}/{} already exists, continuing",
                ORG, row.bcp47
            );
            return Ok(());
        }
        return Err(format!("create repo 422: {}", body).into());
    }
    if !status.is_success() {
        return Err(format!(
            "create repo {}: {}",
            status,
            resp.text().await.unwrap_or_default()
        )
        .into());
    }
    Ok(())
}

async fn wait_for_main(
    http: &reqwest::Client,
    token: &str,
    full: &str,
) -> Result<String, Box<dyn Error>> {
    // auto_init returns 201 but the ref may not exist for a beat.
    // Poll briefly.
    for attempt in 0..20 {
        let resp = http
            .get(format!("{}/repos/{}/git/ref/heads/main", GITHUB_API, full))
            .bearer_auth(token)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        if resp.status().is_success() {
            let v: serde_json::Value = resp.json().await?;
            let sha = v
                .pointer("/object/sha")
                .and_then(|s| s.as_str())
                .ok_or("ref response missing /object/sha")?
                .to_string();
            return Ok(sha);
        }
        if attempt < 19 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    Err("timed out waiting for main branch to appear after auto_init".into())
}

async fn create_blob(
    http: &reqwest::Client,
    token: &str,
    full: &str,
    content: &[u8],
) -> Result<String, Box<dyn Error>> {
    use base64::Engine as _;
    let body = json!({
        "content": base64::engine::general_purpose::STANDARD.encode(content),
        "encoding": "base64",
    });
    let resp = http
        .post(format!("{}/repos/{}/git/blobs", GITHUB_API, full))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    let v: serde_json::Value = resp.json().await?;
    Ok(v.get("sha")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string())
}

#[derive(Serialize)]
struct TreeEntry {
    path: String,
    mode: String,
    #[serde(rename = "type")]
    entry_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sha: Option<String>,
}

async fn create_tree(
    http: &reqwest::Client,
    token: &str,
    full: &str,
    base_tree: Option<&str>,
    entries: &[TreeEntry],
) -> Result<String, Box<dyn Error>> {
    let body = json!({
        "base_tree": base_tree,
        "tree": entries,
    });
    let resp = http
        .post(format!("{}/repos/{}/git/trees", GITHUB_API, full))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        return Err(format!(
            "create_tree {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        )
        .into());
    }
    let v: serde_json::Value = resp.json().await?;
    Ok(v.get("sha")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string())
}

async fn create_commit(
    http: &reqwest::Client,
    token: &str,
    full: &str,
    message: &str,
    tree_sha: &str,
    parents: &[&str],
) -> Result<String, Box<dyn Error>> {
    let body = json!({
        "message": message,
        "tree": tree_sha,
        "parents": parents,
    });
    let resp = http
        .post(format!("{}/repos/{}/git/commits", GITHUB_API, full))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    let v: serde_json::Value = resp.json().await?;
    Ok(v.get("sha")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string())
}

async fn update_main(
    http: &reqwest::Client,
    token: &str,
    full: &str,
    sha: &str,
) -> Result<(), Box<dyn Error>> {
    let body = json!({ "sha": sha, "force": true });
    http.patch(format!("{}/repos/{}/git/refs/heads/main", GITHUB_API, full))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn set_topics(
    http: &reqwest::Client,
    token: &str,
    full: &str,
    topics: &[String],
) -> Result<(), Box<dyn Error>> {
    let body = json!({ "names": topics });
    http.put(format!("{}/repos/{}/topics", GITHUB_API, full))
        .bearer_auth(token)
        .header("Accept", "application/vnd.github+json")
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}
