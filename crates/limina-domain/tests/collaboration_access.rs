//! Project roles, members and live tickets. Ported from `tests/test_cloud_ui_readiness.py` and
//! `tests/test_cloud_api.py` at the service level.

#![allow(clippy::unwrap_used)]

mod collaboration_support;

use std::collections::{BTreeMap, BTreeSet};

use collaboration_support::*;
use limina_core::{Principal, now};
use limina_domain::collaboration::{MemberChange, Role};
use limina_store::args;
use serde_json::{Value, json};

fn change(subject: &str, display_name: &str, email: Option<&str>, role: &str) -> MemberChange {
    MemberChange {
        subject: subject.to_owned(),
        display_name: display_name.to_owned(),
        email: email.map(str::to_owned),
        role: role.to_owned(),
    }
}

fn keys(value: &Value) -> Vec<&str> {
    value.as_object().unwrap().keys().map(String::as_str).collect()
}

/// `test_oidc_principals_and_project_roles_are_enforced_centrally`
#[tokio::test]
async fn project_roles_are_enforced_centrally() {
    for backend in backends().await {
        let service = backend.service();
        let id = create_project(&backend.db, "ui-ready", Some(&owner())).await;

        let owned = service.visible_project_slugs(&owner()).await.unwrap();
        assert_eq!(owned, Some(BTreeSet::from(["ui-ready".to_owned()])), "{}", backend.name);
        assert_eq!(service.visible_project_slugs(&viewer()).await.unwrap(), Some(BTreeSet::new()));
        let denied = service.require_role("ui-ready", &viewer(), Role::Viewer).await.unwrap_err();
        assert_eq!((denied.code(), denied.http_status()), ("permission_denied", 403));
        assert_eq!(denied.message, "You do not have access to this project.");
        assert_eq!(Value::Object(denied.details), json!({"project": "ui-ready"}));

        let member = service
            .set_member(
                "ui-ready",
                &change("viewer", "Victor Viewer", Some("viewer@example.test"), "VIEWER"),
                &owner().actor(),
            )
            .await
            .unwrap();
        assert_eq!(member.role, Role::Viewer);
        let json = serde_json::to_value(&member).unwrap();
        assert_eq!(
            keys(&json),
            ["subject", "display_name", "email", "role", "created_at", "updated_at"]
        );
        assert_eq!(json["role"], "VIEWER");
        assert!(json["created_at"].as_str().unwrap().ends_with("+00:00"));
        assert_eq!(
            service.require_role("ui-ready", &viewer(), Role::Viewer).await.unwrap(),
            Role::Viewer
        );

        let start = service.require_role("ui-ready", &viewer(), Role::Editor).await.unwrap_err();
        assert_eq!(start.code(), "permission_denied");
        assert_eq!(start.message, "Project role EDITOR or higher is required.");
        assert_eq!(
            Value::Object(start.details),
            json!({"project": "ui-ready", "required_role": "EDITOR", "actual_role": "VIEWER"})
        );

        let orphan = service
            .set_member(
                "ui-ready",
                &change("owner", "Olivia Owner", None, "EDITOR"),
                "Olivia Owner",
            )
            .await
            .unwrap_err();
        assert_eq!((orphan.code(), orphan.http_status()), ("invariant_violation", 422));
        assert_eq!(orphan.message, "A project must retain at least one owner.");

        // Events carry the authenticated principal's actor name.
        let events = event_log(&backend.db, &id).await;
        assert_eq!(
            events,
            [(
                "project.member_set".to_owned(),
                "Olivia Owner".to_owned(),
                None,
                json!({"subject": "viewer", "role": "VIEWER"})
            )]
        );
    }
}

/// `test_project_responses_include_the_callers_role`, with private projects: an instance
/// administrator of a multi-user runtime needs a membership like everyone else; only trusted
/// local operators own every project.
#[tokio::test]
async fn roles_resolve_for_members_and_local_operators() {
    for backend in backends().await {
        let service = backend.service();
        create_project(&backend.db, "ui-ready", Some(&owner())).await;
        for (subject, name, role) in
            [("viewer", "Victor Viewer", "VIEWER"), ("editor", "Erin Editor", "editor")]
        {
            service
                .set_member("ui-ready", &change(subject, name, None, role), "Olivia Owner")
                .await
                .unwrap();
        }

        assert_eq!(service.role_for("ui-ready", &owner()).await.unwrap(), Role::Owner);
        assert_eq!(service.role_for("UI-Ready", &viewer()).await.unwrap(), Role::Viewer);
        assert_eq!(service.role_for("ui-ready", &editor()).await.unwrap(), Role::Editor);
        let denied = service.role_for("ui-ready", &admin()).await.unwrap_err();
        assert_eq!((denied.code(), denied.http_status()), ("permission_denied", 403));
        assert_eq!(Value::Object(denied.details), json!({"project": "ui-ready"}));
        let missing = service.role_for("missing", &admin()).await.unwrap_err();
        assert_eq!(missing.code(), "not_found");
        for workos_admin in [admin().mode("workos"), admin().mode("oidc")] {
            assert!(service.role_for("ui-ready", &workos_admin).await.is_err());
            assert_eq!(service.project_roles(&workos_admin).await.unwrap(), Some(BTreeMap::new()));
        }
        let local = Principal::local("operator", false);
        assert_eq!(
            service.require_role("ui-ready", &local, Role::Owner).await.unwrap(),
            Role::Owner
        );
        // Local operators own every project without a lookup, as in the Python runtime.
        assert_eq!(service.role_for("missing", &local).await.unwrap(), Role::Owner);
        let open_mode = Principal::local("", true);
        assert_eq!(service.role_for("ui-ready", &open_mode).await.unwrap(), Role::Owner);

        assert_eq!(
            service.project_roles(&viewer()).await.unwrap(),
            Some(BTreeMap::from([("ui-ready".to_owned(), Role::Viewer)]))
        );
        assert_eq!(service.project_roles(&admin()).await.unwrap(), Some(BTreeMap::new()));
        assert_eq!(service.visible_project_slugs(&local).await.unwrap(), None);
        assert_eq!(service.visible_project_slugs(&open_mode).await.unwrap(), None);

        // An administrator who is a member has their membership's role.
        service
            .set_member(
                "ui-ready",
                &change("instance-admin", "Alice Admin", None, "VIEWER"),
                "Olivia Owner",
            )
            .await
            .unwrap();
        assert_eq!(service.role_for("ui-ready", &admin()).await.unwrap(), Role::Viewer);
        assert_eq!(
            service.require_role("ui-ready", &editor(), Role::Editor).await.unwrap(),
            Role::Editor
        );
        let archive = service.require_role("ui-ready", &editor(), Role::Owner).await.unwrap_err();
        assert_eq!(archive.details["actual_role"], "EDITOR");
    }
}

/// `test_idempotency_keys_are_scoped_to_signed_subjects`: visibility follows the signed subject,
/// never the display name.
#[tokio::test]
async fn visibility_follows_subjects_not_display_names() {
    for backend in backends().await {
        let service = backend.service();
        create_project(&backend.db, "first", Some(&owner())).await;
        create_project(&backend.db, "second", Some(&same_name())).await;
        let first = service.visible_project_slugs(&owner()).await.unwrap().unwrap();
        let second = service.visible_project_slugs(&same_name()).await.unwrap().unwrap();
        assert_eq!(first.into_iter().collect::<Vec<_>>(), ["first"]);
        assert_eq!(second.into_iter().collect::<Vec<_>>(), ["second"]);
    }
}

#[tokio::test]
async fn members_are_listed_changed_and_removed() {
    for backend in backends().await {
        let service = backend.service();
        let id = create_project(&backend.db, "team", None).await;

        // The first member of a project without members becomes its owner.
        service.add_creator("team", &owner()).await.unwrap();
        service.add_creator("TEAM", &same_name()).await.unwrap();
        let members = service.list_members("team").await.unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!((members[0].subject.as_str(), members[0].role), ("owner", Role::Owner));
        assert_eq!(members[0].email.as_deref(), Some("owner@example.test"));
        let missing = service.add_creator("missing", &owner()).await.unwrap_err();
        assert_eq!(missing.message, "Project 'missing' does not exist.");

        let added = service
            .set_member(
                "team",
                &change("  editor ", " Erin Editor ", Some(" erin@example.test "), "editor"),
                "Olivia Owner",
            )
            .await
            .unwrap();
        assert_eq!(
            (
                added.subject.as_str(),
                added.display_name.as_str(),
                added.email.as_deref(),
                added.role
            ),
            ("editor", "Erin Editor", Some("erin@example.test"), Role::Editor)
        );
        service
            .set_member(
                "team",
                &change("viewer", "Victor Viewer", Some(""), "VIEWER"),
                "Olivia Owner",
            )
            .await
            .unwrap();
        service
            .set_member("team", &change("auditor", "Ada Auditor", None, "viewer"), "Olivia Owner")
            .await
            .unwrap();

        // Ordered by role descending (VIEWER, OWNER, EDITOR), then display name.
        let listed: Vec<(String, Role, Option<String>)> = service
            .list_members("team")
            .await
            .unwrap()
            .into_iter()
            .map(|member| (member.subject, member.role, member.email))
            .collect();
        assert_eq!(
            listed,
            [
                ("auditor".to_owned(), Role::Viewer, None),
                ("viewer".to_owned(), Role::Viewer, None),
                ("owner".to_owned(), Role::Owner, Some("owner@example.test".to_owned())),
                ("editor".to_owned(), Role::Editor, Some("erin@example.test".to_owned())),
            ]
        );

        let promoted = service
            .set_member("team", &change("editor", "Erin E.", None, "OWNER"), "Olivia Owner")
            .await
            .unwrap();
        assert_eq!((promoted.role, promoted.email), (Role::Owner, None));
        assert_eq!(promoted.created_at, added.created_at);
        assert!(promoted.updated_at >= added.updated_at);
        // With two owners, either can step down; the last one cannot leave.
        service
            .set_member("team", &change("owner", "Olivia Owner", None, "EDITOR"), "Erin E.")
            .await
            .unwrap();
        let last = service.remove_member("team", "editor", "Erin E.").await.unwrap_err();
        assert_eq!(last.message, "A project must retain at least one owner.");

        let removed = service.remove_member("team", "viewer", "Erin E.").await.unwrap();
        assert_eq!(removed.subject, "viewer");
        let gone = service.remove_member("team", "viewer", "Erin E.").await.unwrap_err();
        assert_eq!(
            (gone.code(), gone.message.as_str()),
            ("not_found", "Project member does not exist.")
        );
        assert_eq!(Value::Object(gone.details), json!({"subject": "viewer"}));

        for (role, subject, name) in
            [("admin", "x", "X"), ("VIEWER", " ", "X"), ("VIEWER", "x", "\t")]
        {
            let error = service
                .set_member("team", &change(subject, name, None, role), "Erin E.")
                .await
                .unwrap_err();
            assert_eq!(error.code(), "invariant_violation");
        }
        let invalid = service
            .set_member("team", &change("x", "X", None, "admin"), "Erin E.")
            .await
            .unwrap_err();
        assert_eq!(invalid.message, "Project role must be OWNER, EDITOR, or VIEWER.");
        let blank = service
            .set_member("team", &change(" ", "X", None, "VIEWER"), "Erin E.")
            .await
            .unwrap_err();
        assert_eq!(blank.message, "Member subject and display name are required.");

        let events: Vec<(String, Value)> = event_log(&backend.db, &id)
            .await
            .into_iter()
            .map(|(kind, _, _, payload)| (kind, payload))
            .collect();
        assert_eq!(events.len(), 6);
        assert_eq!(events[5], ("project.member_removed".to_owned(), json!({"subject": "viewer"})));
    }
}

/// `test_browser_ticket_is_single_use_and_supports_read_only_viewers`
#[tokio::test]
async fn live_tickets_are_single_use_and_carry_the_role() {
    for backend in backends().await {
        let service = backend.service();
        create_project(&backend.db, "ui-ready", Some(&owner())).await;
        create_project(&backend.db, "elsewhere", Some(&owner())).await;
        service
            .set_member(
                "ui-ready",
                &change("viewer", "Victor Viewer", None, "VIEWER"),
                "Olivia Owner",
            )
            .await
            .unwrap();

        let role = service.require_role("ui-ready", &viewer(), Role::Viewer).await.unwrap();
        let before = now();
        let grant = service.issue_live_ticket("ui-ready", &viewer(), role).await.unwrap();
        assert_eq!(grant.ticket.len(), 43, "secrets.token_urlsafe(32)");
        let lifetime = grant.expires_at - before;
        assert!(
            lifetime >= chrono::Duration::seconds(60) && lifetime < chrono::Duration::seconds(62)
        );
        let json = serde_json::to_value(&grant).unwrap();
        assert_eq!(keys(&json), ["ticket", "expires_at"]);
        assert!(json["expires_at"].as_str().unwrap().ends_with("+00:00"));

        // Only the digest is stored.
        let mut tx = backend.db.begin().await.unwrap();
        let stored: Vec<(String,)> =
            tx.fetch_all("SELECT token_hash FROM live_tickets", args![]).await.unwrap();
        tx.commit().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].0.len(), 64);
        assert_ne!(stored[0].0, grant.ticket);

        let elsewhere = service.consume_live_ticket("elsewhere", &grant.ticket).await.unwrap_err();
        assert_eq!(elsewhere.code(), "permission_denied");
        let (principal, role) =
            service.consume_live_ticket("ui-ready", &grant.ticket).await.unwrap();
        assert_eq!(role, Role::Viewer);
        assert_eq!(
            principal,
            Principal {
                subject: "viewer".to_owned(),
                display_name: "Victor Viewer".to_owned(),
                email: None,
                email_verified: false,
                organization_id: None,
                instance_admin: false,
                project_admin: false,
                auth_mode: "live-ticket".to_owned(),
            }
        );
        let reused = service.consume_live_ticket("ui-ready", &grant.ticket).await.unwrap_err();
        assert_eq!((reused.code(), reused.http_status()), ("permission_denied", 403));
        assert_eq!(reused.message, "The live attachment ticket is invalid or expired.");
        assert!(reused.details.is_empty());

        let admin_grant =
            service.issue_live_ticket("ui-ready", &admin(), Role::Owner).await.unwrap();
        let (principal, role) =
            service.consume_live_ticket("ui-ready", &admin_grant.ticket).await.unwrap();
        assert_eq!(
            (principal.subject.as_str(), principal.instance_admin, role),
            ("instance-admin", true, Role::Owner)
        );
        assert!(!principal.acts_on_every_project(), "administration grants no project access");

        // A trusted local operator's ticket keeps acting on the project without a membership.
        let operator = Principal::local("operator", true);
        let operator_grant =
            service.issue_live_ticket("ui-ready", &operator, Role::Owner).await.unwrap();
        let (principal, _) =
            service.consume_live_ticket("ui-ready", &operator_grant.ticket).await.unwrap();
        assert!(principal.acts_on_every_project());
        assert_eq!(service.role_for("ui-ready", &principal).await.unwrap(), Role::Owner);

        // Lifetimes are clamped to 15–120 seconds, and expired tickets are refused.
        let short =
            service.issue_live_ticket_for("ui-ready", &viewer(), Role::Viewer, 1).await.unwrap();
        let long =
            service.issue_live_ticket_for("ui-ready", &viewer(), Role::Viewer, 3600).await.unwrap();
        assert!(short.expires_at - now() <= chrono::Duration::seconds(15));
        assert!(short.expires_at - now() > chrono::Duration::seconds(10));
        assert!(long.expires_at - now() <= chrono::Duration::seconds(120));
        assert!(long.expires_at - now() > chrono::Duration::seconds(115));
        let mut tx = backend.db.begin_write().await.unwrap();
        tx.execute(
            "UPDATE live_tickets SET expires_at = $1",
            args![now() - chrono::Duration::seconds(1)],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let expired = service.consume_live_ticket("ui-ready", &short.ticket).await.unwrap_err();
        assert_eq!(expired.message, "The live attachment ticket is invalid or expired.");
        let missing =
            service.issue_live_ticket("missing", &viewer(), Role::Viewer).await.unwrap_err();
        assert_eq!(missing.code(), "not_found");
    }
}
