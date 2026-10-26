#![allow(clippy::unwrap_used)]
use axum::{Json, Router, extract::Query, http::HeaderMap, routing::get};
use limina_server::{
    app::PeopleDirectory,
    auth::{WorkOsSettings, directory::WorkOsDirectory},
};
use serde_json::{Value, json};
use std::collections::HashMap;

#[tokio::test]
async fn the_directory_requires_a_verified_matching_user_and_active_organization_membership() {
    async fn users(
        Query(query): Query<HashMap<String, String>>,
        headers: HeaderMap,
    ) -> Json<Value> {
        assert_eq!(headers["authorization"], "Bearer sk_test");
        let email = &query["email"];
        let data = if email == "missing@example.test" {
            json!([])
        } else {
            json!([{"id":email,"email":email,"email_verified":email!="unverified@example.test","first_name":"Example","last_name":"Person"}])
        };
        Json(json!({"data":data}))
    }
    async fn memberships(Query(query): Query<HashMap<String, String>>) -> Json<Value> {
        assert_eq!(query["organization_id"], "org_allowed");
        Json(
            json!({"data":[{"organization_id":"org_allowed","status":if query["user_id"]=="outside@example.test"{"inactive"}else{"active"}}]}),
        )
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/user_management/users", get(users))
                .route("/user_management/organization_memberships", get(memberships)),
        )
        .await
        .unwrap();
    });
    let mut settings = WorkOsSettings::new("client_test", "sk_test", "p".repeat(40)).unwrap();
    settings.api_base_url = base;
    settings.organization_id = Some("org_allowed".into());
    let directory = WorkOsDirectory::new(
        &settings,
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(2)).build().unwrap(),
    );
    let person = directory
        .find_by_email(" Allowed@Example.Test ", directory.organization())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(person.subject, "allowed@example.test");
    assert_eq!(person.display_name, "Example Person");
    for email in ["missing@example.test", "unverified@example.test", "outside@example.test"] {
        assert!(directory.find_by_email(email, directory.organization()).await.unwrap().is_none());
    }
    task.abort();
}
