use crate::auth::auth_user::AuthUser;
use crate::store::sqlite_user_state::SqliteUserState;
use crate::utils::json_responses::make_bad_json_data_response;
use crate::utils::response::{not_ok_json_response, ok_json_response, ok_ok_json_response};
use rocket::http::{ContentType, Status};
use rocket::response::status;
use rocket::{get, post, State};
use std::path::PathBuf;
use std::sync::Arc;

/// `GET /my-resources` — repo paths the user has selected.
#[get("/my-resources")]
pub fn get_my_resources(
    user: AuthUser,
    db: &State<Option<Arc<SqliteUserState>>>,
) -> status::Custom<(ContentType, String)> {
    let db = match db.inner().as_ref() {
        Some(d) => d,
        None => return ok_json_response("[]".to_string()),
    };
    match db.get_selected_resources(&user.id) {
        Ok(paths) => ok_json_response(serde_json::to_string(&paths).unwrap()),
        Err(e) => not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("db: {}", e)),
        ),
    }
}

/// `POST /select-resource/<repo_path..>` — add a resource to the user's selections.
#[post("/select-resource/<repo_path..>")]
pub fn select_resource(
    user: AuthUser,
    db: &State<Option<Arc<SqliteUserState>>>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let db = match db.inner().as_ref() {
        Some(d) => d,
        None => {
            return not_ok_json_response(
                Status::ServiceUnavailable,
                make_bad_json_data_response("user state database not configured".into()),
            )
        }
    };
    let path_str = repo_path.to_string_lossy().to_string();
    let mut selected = match db.get_selected_resources(&user.id) {
        Ok(s) => s,
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("db: {}", e)),
            )
        }
    };
    if selected.iter().any(|s| s == &path_str) {
        return ok_ok_json_response();
    }
    selected.push(path_str);
    if let Err(e) = db.put_selected_resources(&user.id, &selected) {
        return not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("db: {}", e)),
        );
    }
    ok_ok_json_response()
}

/// `POST /deselect-resource/<repo_path..>` — remove a resource from the user's selections.
#[post("/deselect-resource/<repo_path..>")]
pub fn deselect_resource(
    user: AuthUser,
    db: &State<Option<Arc<SqliteUserState>>>,
    repo_path: PathBuf,
) -> status::Custom<(ContentType, String)> {
    let db = match db.inner().as_ref() {
        Some(d) => d,
        None => {
            return not_ok_json_response(
                Status::ServiceUnavailable,
                make_bad_json_data_response("user state database not configured".into()),
            )
        }
    };
    let path_str = repo_path.to_string_lossy().to_string();
    let mut selected = match db.get_selected_resources(&user.id) {
        Ok(s) => s,
        Err(e) => {
            return not_ok_json_response(
                Status::InternalServerError,
                make_bad_json_data_response(format!("db: {}", e)),
            )
        }
    };
    selected.retain(|s| s != &path_str);
    if let Err(e) = db.put_selected_resources(&user.id, &selected) {
        return not_ok_json_response(
            Status::InternalServerError,
            make_bad_json_data_response(format!("db: {}", e)),
        );
    }
    ok_ok_json_response()
}
