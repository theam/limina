#![allow(clippy::unwrap_used)]
mod collaboration_support;
use collaboration_support::*;
use limina_core::Principal;
use limina_domain::collaboration::{DirectoryPerson, Role};

#[tokio::test]
async fn pending_shares_require_a_verified_email_in_the_right_organization() {
    for backend in backends().await {
        let service = backend.service();
        let owner = owner();
        create_project(&backend.db, "private", Some(&owner)).await;
        let share = service
            .share_with_email(
                "private",
                " Next@Example.Test ",
                Role::Editor,
                Some("org_allowed"),
                &owner,
            )
            .await
            .unwrap();
        assert!(share.pending);
        assert_eq!(share.email.as_deref(), Some("next@example.test"));
        let person = Principal::new("next", "Next")
            .with_email(Some("next@example.test".into()))
            .organization(Some("org_allowed".into()));
        assert_eq!(service.accept_invitations(&person).await.unwrap(), 0);
        let verified = person.email_verified(true);
        assert_eq!(
            service
                .accept_invitations(&verified.clone().organization(Some("other".into())))
                .await
                .unwrap(),
            0
        );
        assert_eq!(service.accept_invitations(&verified).await.unwrap(), 1);
        assert_eq!(service.accept_invitations(&verified).await.unwrap(), 0);
        assert_eq!(service.role_for("private", &verified).await.unwrap(), Role::Editor);
        assert_eq!(
            service.shared_projects(&verified).await.unwrap()[0].owners[0].subject,
            owner.subject
        );
        assert!(!service.sharing("private").await.unwrap().iter().any(|share| share.pending));
    }
}

#[tokio::test]
async fn directory_shares_change_roles_and_preserve_the_last_owner() {
    for backend in backends().await {
        let service = backend.service();
        let owner = owner();
        create_project(&backend.db, "private", Some(&owner)).await;
        let person = DirectoryPerson {
            subject: owner.subject.clone(),
            display_name: owner.display_name.clone(),
            email: owner.email.clone().unwrap(),
        };
        assert!(service.share_with_person("private", &person, Role::Viewer, &owner).await.is_err());
        let second = DirectoryPerson {
            subject: "second".into(),
            display_name: "Second".into(),
            email: "second@example.test".into(),
        };
        service.share_with_person("private", &second, Role::Owner, &owner).await.unwrap();
        service.share_with_person("private", &person, Role::Viewer, &owner).await.unwrap();
        assert_eq!(service.role_for("private", &owner).await.unwrap(), Role::Viewer);
        let pending = service
            .share_with_email("private", "later@example.test", Role::Viewer, None, &owner)
            .await
            .unwrap();
        assert!(pending.pending);
        service.cancel_invitation("private", "later@example.test", "Owner").await.unwrap();
        assert!(!service.sharing("private").await.unwrap().iter().any(|share| share.pending));
    }
}
